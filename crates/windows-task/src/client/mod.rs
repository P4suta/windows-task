//! Local and remote Task Scheduler sessions on a dedicated COM MTA worker.

use std::{
    fmt,
    sync::{Arc, mpsc},
    thread,
};

#[cfg(feature = "async")]
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[cfg(feature = "history")]
use std::sync::atomic::AtomicBool;

#[cfg(any(feature = "async", feature = "history"))]
use std::sync::atomic::Ordering;

#[cfg(feature = "async")]
use std::sync::atomic::AtomicU8;

#[cfg(feature = "history")]
use std::time::{Duration, Instant, SystemTime};

#[cfg(windows)]
use std::str::FromStr;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Credential, DiagnosticReport, Error, ErrorKind, FolderPath, Password, Result, TaskPath,
    ValidationReport,
    model::{LogonType, SecurityDescriptor, TaskDateTime, TaskDefinition, TaskSchemaVersion},
    xml::{RawTaskXml, TaskSnapshot},
};

#[cfg(feature = "history")]
use crate::history::{HistoryEvent, HistoryEventKind, HistoryQuery, ResultConfidence, RunOutcome};

#[cfg(windows)]
mod sys;
#[cfg(not(windows))]
mod sys_portable;
#[cfg(not(windows))]
use sys_portable as sys;

type Job = Box<dyn FnOnce(&mut sys::Session) + Send + 'static>;

/// Process-wide COM behavior. `windows-task` never silently initializes COM
/// security because another library may already own that decision.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ComSecurityPolicy {
    /// Use existing process security and configure returned proxies when possible.
    #[default]
    ExistingProcess,
    /// Require proxy configuration to succeed; otherwise connection fails.
    RequireProxyBlanket,
}

/// Builder for one local or remote scheduler connection.
#[derive(Debug, Default)]
pub struct SchedulerBuilder {
    target: Option<String>,
    credential: Option<Credential>,
    com_security: ComSecurityPolicy,
}

impl SchedulerBuilder {
    /// Targets the local scheduler using the current token.
    #[must_use]
    pub fn local(mut self) -> Self {
        self.target = None;
        self.credential = None;
        self
    }

    /// Targets a remote computer name or DNS name.
    #[must_use]
    pub fn remote(mut self, computer: impl Into<String>) -> Self {
        self.target = Some(computer.into());
        self
    }

    /// Supplies a one-shot remote connection credential.
    #[must_use]
    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }

    /// Selects how existing process COM security is treated.
    #[must_use]
    pub const fn com_security(mut self, policy: ComSecurityPolicy) -> Self {
        self.com_security = policy;
        self
    }

    /// Connects while blocking the current thread. All subsequent native calls
    /// occur on a private MTA worker, not this thread.
    pub fn connect_blocking(self) -> Result<Scheduler> {
        Scheduler::connect(self)
    }

    /// Connects without depending on Tokio, async-std, or an executor-specific API.
    #[cfg(feature = "async")]
    pub fn connect_async(self) -> ConnectFuture {
        let (sender, receiver) = futures_channel::oneshot::channel();
        thread::spawn(move || {
            drop(sender.send(Scheduler::connect(self)));
        });
        ConnectFuture { receiver }
    }
}

/// Cloneable scheduler session backed by one dedicated MTA worker.
#[derive(Clone)]
pub struct Scheduler {
    worker: Arc<Worker>,
    connection: ConnectionInfo,
}

impl Scheduler {
    /// Starts a scheduler builder.
    #[must_use]
    pub fn builder() -> SchedulerBuilder {
        SchedulerBuilder::default()
    }

    fn connect(builder: SchedulerBuilder) -> Result<Self> {
        let target = builder.target.clone();
        let input = sys::ConnectionInput {
            target: builder.target,
            credential: builder.credential,
            com_security: builder.com_security,
        };
        let (sender, receiver) = mpsc::channel::<Job>();
        let (init_sender, init_receiver) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("windows-task-com".into())
            .spawn(move || match sys::Session::connect(input) {
                Ok(mut session) => {
                    let info = session.connection_info();
                    drop(init_sender.send(info));
                    for job in receiver {
                        job(&mut session);
                    }
                }
                Err(error) => {
                    drop(init_sender.send(Err(error)));
                }
            })
            .map_err(|error| {
                Error::new(
                    ErrorKind::WorkerStopped,
                    format!("cannot start COM worker: {error}"),
                )
            })?;
        let connection = init_receiver.recv().map_err(|_| {
            Error::new(
                ErrorKind::WorkerStopped,
                "COM worker stopped before reporting connection status",
            )
            .with_target(target.unwrap_or_else(|| "local".into()))
        })??;
        Ok(Self {
            worker: Arc::new(Worker {
                sender: Some(sender),
                join: Some(join),
            }),
            connection,
        })
    }

    /// Returns immutable connection metadata.
    #[must_use]
    pub const fn connection_info(&self) -> &ConnectionInfo {
        &self.connection
    }

    /// Creates the blocking API view.
    #[must_use]
    pub fn blocking(&self) -> BlockingScheduler {
        BlockingScheduler(self.clone())
    }

    /// Creates the runtime-neutral asynchronous API view.
    #[cfg(feature = "async")]
    #[must_use]
    pub fn asynchronous(&self) -> AsyncScheduler {
        AsyncScheduler(self.clone())
    }
}

impl fmt::Debug for Scheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Scheduler")
            .field("connection", &self.connection)
            .finish_non_exhaustive()
    }
}

struct Worker {
    sender: Option<mpsc::Sender<Job>>,
    join: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn call<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut sys::Session) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .ok_or_else(worker_stopped)?
            .send(Box::new(move |session| {
                drop(sender.send(operation(session)));
            }))
            .map_err(|_| worker_stopped())?;
        receiver.recv().map_err(|_| worker_stopped())?
    }

    #[cfg(feature = "async")]
    fn call_async<T, F>(&self, operation: F) -> OperationFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut sys::Session) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = futures_channel::oneshot::channel();
        let state = Arc::new(AtomicU8::new(OPERATION_QUEUED));
        let job_state = Arc::clone(&state);
        let job = Box::new(move |session: &mut sys::Session| {
            if job_state
                .compare_exchange(
                    OPERATION_QUEUED,
                    OPERATION_STARTED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
            {
                drop(sender.send(Err(Error::new(
                    ErrorKind::Cancelled,
                    "operation was cancelled before it started",
                ))));
                return;
            }
            drop(sender.send(operation(session)));
        });
        let queued = self
            .sender
            .as_ref()
            .ok_or_else(worker_stopped)
            .and_then(|queue| queue.send(job).map_err(|_| worker_stopped()));
        let receiver = queued.is_ok().then_some(receiver);
        OperationFuture {
            receiver,
            immediate: queued.err(),
            state,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(join) = self.join.take() {
            drop(join.join());
        }
    }
}

fn worker_stopped() -> Error {
    Error::new(
        ErrorKind::WorkerStopped,
        "Task Scheduler COM worker is unavailable",
    )
}

/// Runtime-neutral future returned by `connect_async`.
#[cfg(feature = "async")]
#[derive(Debug)]
pub struct ConnectFuture {
    receiver: futures_channel::oneshot::Receiver<Result<Scheduler>>,
}

#[cfg(feature = "async")]
impl Future for ConnectFuture {
    type Output = Result<Scheduler>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver)
            .poll(context)
            .map(|result| result.unwrap_or_else(|_| Err(worker_stopped())))
    }
}

/// A cancellable-before-start, runtime-neutral scheduler operation.
#[cfg(feature = "async")]
#[derive(Debug)]
pub struct OperationFuture<T> {
    receiver: Option<futures_channel::oneshot::Receiver<Result<T>>>,
    immediate: Option<Error>,
    state: Arc<AtomicU8>,
}

#[cfg(feature = "async")]
const OPERATION_QUEUED: u8 = 0;
#[cfg(feature = "async")]
const OPERATION_STARTED: u8 = 1;
#[cfg(feature = "async")]
const OPERATION_CANCELLED: u8 = 2;

#[cfg(feature = "async")]
impl<T> OperationFuture<T> {
    /// Whether the native operation has begun. Dropping after this point does
    /// not attempt to abort an in-flight COM RPC.
    #[must_use]
    pub fn has_started(&self) -> bool {
        self.state.load(Ordering::Acquire) == OPERATION_STARTED
    }
}

#[cfg(feature = "async")]
impl<T> Future for OperationFuture<T> {
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(error) = self.immediate.take() {
            return Poll::Ready(Err(error));
        }
        let receiver = self.receiver.as_mut().expect("receiver or immediate error");
        Pin::new(receiver)
            .poll(context)
            .map(|result| result.unwrap_or_else(|_| Err(worker_stopped())))
    }
}

#[cfg(feature = "async")]
impl<T> Drop for OperationFuture<T> {
    fn drop(&mut self) {
        self.state
            .compare_exchange(
                OPERATION_QUEUED,
                OPERATION_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .unwrap_or(OPERATION_CANCELLED);
    }
}

/// Runtime-neutral future for a history-correlated task run wait.
#[cfg(all(feature = "async", feature = "history"))]
#[derive(Debug)]
pub struct WaitFuture {
    receiver: Option<futures_channel::oneshot::Receiver<Result<RunOutcome>>>,
    immediate: Option<Error>,
    cancelled: Arc<AtomicBool>,
}

#[cfg(all(feature = "async", feature = "history"))]
impl Future for WaitFuture {
    type Output = Result<RunOutcome>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(error) = self.immediate.take() {
            return Poll::Ready(Err(error));
        }
        let receiver = self.receiver.as_mut().expect("receiver or immediate error");
        Pin::new(receiver)
            .poll(context)
            .map(|result| result.unwrap_or_else(|_| Err(worker_stopped())))
    }
}

#[cfg(all(feature = "async", feature = "history"))]
impl Drop for WaitFuture {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Information reported by the connected Task Scheduler service.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ConnectionInfo {
    /// Server name selected by Task Scheduler.
    pub target_server: String,
    /// Connected username, when reported.
    pub user: Option<String>,
    /// Connected domain, when reported.
    pub domain: Option<String>,
    /// Highest Task Scheduler interface/schema version.
    pub highest_version: u32,
}

/// Capabilities determined from the connected service and read-only probes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct Capabilities {
    /// Native packed highest-version value.
    pub highest_version: u32,
    /// Highest typed schema that can be registered.
    pub schema_version: TaskSchemaVersion,
    /// Required privileges on principals are accepted.
    pub required_privileges: bool,
    /// Automatic maintenance settings are accepted.
    pub maintenance_settings: bool,
    /// Volatile task settings are accepted.
    pub volatile_tasks: bool,
    /// Operational event history can be queried.
    pub history_query: bool,
    /// Remote event history can be queried in this session.
    pub remote_history_query: bool,
}

/// Task Scheduler runtime state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum TaskState {
    /// State is not known.
    Unknown,
    /// Task is disabled.
    Disabled,
    /// Task is queued.
    Queued,
    /// Task is ready.
    Ready,
    /// Task is running.
    Running,
}

/// Snapshot and status for one registered task.
#[derive(Clone, Debug)]
pub struct RegisteredTask {
    /// Absolute task path.
    pub path: TaskPath,
    /// Runtime state.
    pub state: TaskState,
    /// Whether triggers and demand start are enabled.
    pub enabled: bool,
    /// Last action result HRESULT/process exit code.
    pub last_result: i32,
    /// Number of missed runs.
    pub missed_runs: u32,
    /// Last run, if one has occurred.
    pub last_run: Option<TaskDateTime>,
    /// Next scheduled run, if any.
    pub next_run: Option<TaskDateTime>,
    /// Original Task XML plus typed parse result.
    pub snapshot: TaskSnapshot,
}

/// Information about one scheduler folder.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct TaskFolder {
    /// Absolute folder path.
    pub path: FolderPath,
    /// Optional security descriptor when requested.
    pub security_descriptor: Option<SecurityDescriptor>,
}

/// One executing task instance.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct RunningTask {
    /// Absolute registered task path.
    pub path: TaskPath,
    /// Per-run instance GUID.
    pub instance_id: Uuid,
    /// Current action identifier or path.
    pub current_action: Option<String>,
    /// Task host process identifier.
    pub engine_process_id: u32,
    /// Current state.
    pub state: TaskState,
}

/// Result of starting a task.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct RunHandle {
    /// Per-run instance GUID used to correlate history.
    pub instance_id: Uuid,
    /// Registered task path.
    pub path: TaskPath,
    /// Task host process identifier when already assigned.
    pub engine_process_id: Option<u32>,
}

/// Polling and correlation policy for [`BlockingScheduler::wait_for_run`].
#[cfg(feature = "history")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WaitOptions {
    /// Overall wait limit.
    pub timeout: Duration,
    /// Interval between history/running-task probes.
    pub poll_interval: Duration,
    /// Time to wait for delayed Event Log delivery after an instance vanishes.
    pub history_grace: Duration,
    /// Permit the less precise registered-task last-result fallback.
    pub allow_polling_fallback: bool,
}

#[cfg(feature = "history")]
impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5 * 60),
            poll_interval: Duration::from_millis(250),
            history_grace: Duration::from_secs(2),
            allow_polling_fallback: true,
        }
    }
}

/// Blocking iterator over newly observed Operational history events.
#[cfg(feature = "history")]
#[derive(Debug)]
pub struct HistoryWatcher {
    receiver: mpsc::Receiver<Result<HistoryEvent>>,
    stop: Option<mpsc::Sender<()>>,
    join: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "history")]
impl Iterator for HistoryWatcher {
    type Item = Result<HistoryEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

#[cfg(feature = "history")]
impl Drop for HistoryWatcher {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            if stop.send(()).is_err() {
                // The worker already stopped, which is the requested state.
            }
        }
        if let Some(join) = self.join.take() {
            drop(join.join());
        }
    }
}

/// Run/RunEx options.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunOptions {
    /// Up to 32 values substituted in order, starting at `$(Arg0)`.
    pub parameters: Vec<String>,
    /// Optional Terminal Services session id.
    pub session_id: Option<u32>,
    /// Optional user SID used by RunEx.
    pub user_sid: Option<String>,
    /// Starts regardless of constraint checks.
    pub ignore_constraints: bool,
    /// Runs in the caller's security context (`TASK_RUN_AS_SELF`) instead of
    /// the registered task principal.
    pub as_self: bool,
}

/// Registration behavior.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum RegistrationMode {
    /// Fails if the task exists.
    Create,
    /// Fails if the task does not exist.
    Update,
    /// Creates or replaces as needed.
    #[default]
    CreateOrUpdate,
    /// Asks Task Scheduler to validate without persisting.
    ValidateOnly,
}

/// Options that affect task registration but not the task definition.
#[derive(Debug)]
pub struct RegistrationOptions {
    /// Create/update mode.
    pub mode: RegistrationMode,
    /// Prevent registration triggers from firing.
    pub ignore_registration_triggers: bool,
    /// Registers the task disabled.
    pub disabled: bool,
    /// Does not add a principal ACE to the task folder.
    pub dont_add_principal_ace: bool,
    /// Optional password consumed by password-backed registration.
    pub password: Option<Password>,
}

impl Default for RegistrationOptions {
    fn default() -> Self {
        Self {
            mode: RegistrationMode::CreateOrUpdate,
            ignore_registration_triggers: true,
            disabled: false,
            dont_add_principal_ace: false,
            password: None,
        }
    }
}

/// Enumeration behavior.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ListOptions {
    /// Descends into child folders.
    pub recursive: bool,
    /// Includes hidden tasks.
    pub include_hidden: bool,
}

bitflags::bitflags! {
    /// Explicit security-information flags for SDDL operations.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub struct SecurityInformation: u32 {
        /// Owner SID.
        const OWNER = 0x0000_0001;
        /// Primary group SID.
        const GROUP = 0x0000_0002;
        /// DACL.
        const DACL = 0x0000_0004;
        /// SACL; requires appropriate privilege.
        const SACL = 0x0000_0008;
    }
}

/// Blocking view over a scheduler session.
#[derive(Clone, Debug)]
pub struct BlockingScheduler(Scheduler);

impl BlockingScheduler {
    /// Returns target capabilities.
    pub fn capabilities(&self) -> Result<Capabilities> {
        self.0.worker.call(sys::Session::capabilities)
    }

    /// Runs read-only connectivity, rights, schema, and history checks.
    pub fn doctor(&self) -> Result<DiagnosticReport> {
        self.0.worker.call(sys::Session::doctor)
    }

    /// Queries the Task Scheduler Operational event channel.
    #[cfg(feature = "history")]
    pub fn history(&self, query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
        self.0.worker.call(move |session| session.history(query))
    }

    /// Explicitly enables or disables the Task Scheduler Operational channel.
    /// This normally requires administrator rights and is never done while
    /// connecting or querying.
    #[cfg(feature = "history")]
    pub fn set_history_enabled(&self, enabled: bool) -> Result<()> {
        self.0
            .worker
            .call(move |session| session.set_history_enabled(enabled))
    }

    /// Waits for one run, preferring exact instance-GUID Event Log
    /// correlation and explicitly labeling the polling fallback.
    #[cfg(feature = "history")]
    pub fn wait_for_run(&self, handle: &RunHandle, options: WaitOptions) -> Result<RunOutcome> {
        wait_for_run(self, handle, options, None)
    }

    /// Watches new history records by polling the channel on a helper thread.
    /// If `query.since` is omitted, the watch begins at the call time instead
    /// of replaying the entire log.
    #[cfg(feature = "history")]
    pub fn watch_history(
        &self,
        mut query: HistoryQuery,
        poll_interval: Duration,
    ) -> Result<HistoryWatcher> {
        if poll_interval.is_zero() {
            return Err(Error::new(
                ErrorKind::InvalidDefinition,
                "history watch poll interval must be non-zero",
            ));
        }
        query.forward = false;
        if query.since.is_none() {
            query.since = Some(SystemTime::now());
        }
        let scheduler = self.clone();
        let (event_sender, receiver) = mpsc::channel();
        let (stop, stop_receiver) = mpsc::channel();
        let join = thread::Builder::new()
            .name("windows-task-history".into())
            .spawn(move || {
                let mut last_record_id = 0_u64;
                loop {
                    if stop_receiver.try_recv().is_ok() {
                        break;
                    }
                    match scheduler.history(query.clone()) {
                        Ok(mut events) => {
                            events.sort_by_key(|event| event.record_id);
                            for event in events {
                                if event.record_id <= last_record_id {
                                    continue;
                                }
                                last_record_id = event.record_id;
                                if event_sender.send(Ok(event)).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            drop(event_sender.send(Err(error)));
                            return;
                        }
                    }
                    match stop_receiver.recv_timeout(poll_interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })
            .map_err(|error| {
                Error::new(
                    ErrorKind::WorkerStopped,
                    format!("cannot start history watcher: {error}"),
                )
            })?;
        Ok(HistoryWatcher {
            receiver,
            stop: Some(stop),
            join: Some(join),
        })
    }

    /// Validates a definition offline and against the target without persisting it.
    pub fn validate(&self, definition: TaskDefinition) -> Result<ValidationReport> {
        self.0
            .worker
            .call(move |session| session.validate(definition))
    }

    /// Gets one registered task and its original XML.
    pub fn get_task(&self, path: &TaskPath) -> Result<RegisteredTask> {
        let path = path.clone();
        self.0.worker.call(move |session| session.get_task(path))
    }

    /// Enumerates tasks beneath a folder.
    pub fn list_tasks(
        &self,
        folder: &FolderPath,
        options: ListOptions,
    ) -> Result<Vec<RegisteredTask>> {
        let folder = folder.clone();
        self.0
            .worker
            .call(move |session| session.list_tasks(folder, options))
    }

    /// Registers canonical XML generated from a typed definition.
    pub fn register(
        &self,
        path: &TaskPath,
        definition: TaskDefinition,
        options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.register(path, definition, options))
    }

    /// Registers already validated raw Task XML.
    pub fn register_raw(
        &self,
        path: &TaskPath,
        xml: RawTaskXml,
        logon_type: LogonType,
        options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.register_raw(path, xml, logon_type, options))
    }

    /// Deletes a task. Running instances are not stopped implicitly.
    pub fn delete_task(&self, path: &TaskPath) -> Result<()> {
        let path = path.clone();
        self.0.worker.call(move |session| session.delete_task(path))
    }

    /// Enables or disables a registered task.
    pub fn set_enabled(&self, path: &TaskPath, enabled: bool) -> Result<()> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.set_enabled(path, enabled))
    }

    /// Starts one task instance.
    pub fn run(&self, path: &TaskPath, options: RunOptions) -> Result<RunHandle> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.run(path, options))
    }

    /// Stops all instances of one registered task.
    pub fn stop_all(&self, path: &TaskPath) -> Result<()> {
        let path = path.clone();
        self.0.worker.call(move |session| session.stop_all(path))
    }

    /// Stops one specific running instance.
    pub fn stop_instance(&self, instance_id: Uuid) -> Result<()> {
        self.0
            .worker
            .call(move |session| session.stop_instance(instance_id))
    }

    /// Lists running instances visible to the connected user.
    pub fn running_tasks(&self, include_hidden: bool) -> Result<Vec<RunningTask>> {
        self.0
            .worker
            .call(move |session| session.running_tasks(include_hidden))
    }

    /// Creates a child folder, optionally with SDDL.
    pub fn create_folder(
        &self,
        path: &FolderPath,
        security: Option<SecurityDescriptor>,
    ) -> Result<TaskFolder> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.create_folder(path, security))
    }

    /// Lists child folders, recursively when requested.
    pub fn list_folders(&self, path: &FolderPath, recursive: bool) -> Result<Vec<TaskFolder>> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.list_folders(path, recursive))
    }

    /// Deletes an empty child folder.
    pub fn delete_folder(&self, path: &FolderPath) -> Result<()> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.delete_folder(path))
    }

    /// Reads a task's SDDL.
    pub fn task_security(
        &self,
        path: &TaskPath,
        information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.task_security(path, information))
    }

    /// Reads a folder's SDDL.
    pub fn folder_security(
        &self,
        path: &FolderPath,
        information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.folder_security(path, information))
    }

    /// Replaces selected portions of a task security descriptor.
    pub fn set_task_security(
        &self,
        path: &TaskPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> Result<()> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.set_task_security(path, descriptor, information))
    }

    /// Replaces selected portions of a folder security descriptor.
    pub fn set_folder_security(
        &self,
        path: &FolderPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> Result<()> {
        let path = path.clone();
        self.0
            .worker
            .call(move |session| session.set_folder_security(path, descriptor, information))
    }
}

/// Runtime-neutral asynchronous view over a scheduler session.
#[cfg(feature = "async")]
#[derive(Clone, Debug)]
pub struct AsyncScheduler(Scheduler);

#[cfg(feature = "async")]
impl AsyncScheduler {
    /// Returns target capabilities.
    pub fn capabilities(&self) -> OperationFuture<Capabilities> {
        self.0.worker.call_async(sys::Session::capabilities)
    }

    /// Queries Task Scheduler Operational event history.
    #[cfg(feature = "history")]
    pub fn history(&self, query: HistoryQuery) -> OperationFuture<Vec<HistoryEvent>> {
        self.0
            .worker
            .call_async(move |session| session.history(query))
    }

    /// Explicitly enables or disables Task Scheduler Operational history.
    #[cfg(feature = "history")]
    pub fn set_history_enabled(&self, enabled: bool) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.set_history_enabled(enabled))
    }

    /// Waits for a run without tying the future to a specific async runtime.
    /// Dropping the future asks the helper thread to stop between probes.
    #[cfg(feature = "history")]
    pub fn wait_for_run(&self, handle: RunHandle, options: WaitOptions) -> WaitFuture {
        let scheduler = BlockingScheduler(self.0.clone());
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = futures_channel::oneshot::channel();
        let spawned = thread::Builder::new()
            .name("windows-task-wait".into())
            .spawn(move || {
                drop(sender.send(wait_for_run(
                    &scheduler,
                    &handle,
                    options,
                    Some(&thread_cancelled),
                )));
            });
        WaitFuture {
            receiver: spawned.as_ref().ok().map(|_| receiver),
            immediate: spawned.err().map(|error| {
                Error::new(
                    ErrorKind::WorkerStopped,
                    format!("cannot start run waiter: {error}"),
                )
            }),
            cancelled,
        }
    }

    /// Validates a definition offline and against the connected target.
    #[must_use]
    pub fn validate(&self, definition: TaskDefinition) -> OperationFuture<ValidationReport> {
        self.0
            .worker
            .call_async(move |session| session.validate(definition))
    }

    /// Gets one registered task and its original XML.
    pub fn get_task(&self, path: TaskPath) -> OperationFuture<RegisteredTask> {
        self.0
            .worker
            .call_async(move |session| session.get_task(path))
    }

    /// Enumerates tasks beneath a folder.
    pub fn list_tasks(
        &self,
        folder: FolderPath,
        options: ListOptions,
    ) -> OperationFuture<Vec<RegisteredTask>> {
        self.0
            .worker
            .call_async(move |session| session.list_tasks(folder, options))
    }

    /// Registers a typed task definition.
    pub fn register(
        &self,
        path: TaskPath,
        definition: TaskDefinition,
        options: RegistrationOptions,
    ) -> OperationFuture<RegisteredTask> {
        self.0
            .worker
            .call_async(move |session| session.register(path, definition, options))
    }

    /// Registers a bounded raw Task XML document.
    #[must_use]
    pub fn register_raw(
        &self,
        path: TaskPath,
        xml: RawTaskXml,
        logon_type: LogonType,
        options: RegistrationOptions,
    ) -> OperationFuture<RegisteredTask> {
        self.0
            .worker
            .call_async(move |session| session.register_raw(path, xml, logon_type, options))
    }

    /// Deletes a registered task.
    pub fn delete_task(&self, path: TaskPath) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.delete_task(path))
    }

    /// Enables or disables a registered task.
    #[must_use]
    pub fn set_enabled(&self, path: TaskPath, enabled: bool) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.set_enabled(path, enabled))
    }

    /// Starts a task instance.
    pub fn run(&self, path: TaskPath, options: RunOptions) -> OperationFuture<RunHandle> {
        self.0
            .worker
            .call_async(move |session| session.run(path, options))
    }

    /// Stops all instances of one task.
    #[must_use]
    pub fn stop_all(&self, path: TaskPath) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.stop_all(path))
    }

    /// Stops one instance by GUID.
    #[must_use]
    pub fn stop_instance(&self, instance_id: Uuid) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.stop_instance(instance_id))
    }

    /// Lists running task instances.
    #[must_use]
    pub fn running_tasks(&self, include_hidden: bool) -> OperationFuture<Vec<RunningTask>> {
        self.0
            .worker
            .call_async(move |session| session.running_tasks(include_hidden))
    }

    /// Creates one child folder.
    #[must_use]
    pub fn create_folder(
        &self,
        path: FolderPath,
        security: Option<SecurityDescriptor>,
    ) -> OperationFuture<TaskFolder> {
        self.0
            .worker
            .call_async(move |session| session.create_folder(path, security))
    }

    /// Lists child folders.
    #[must_use]
    pub fn list_folders(
        &self,
        path: FolderPath,
        recursive: bool,
    ) -> OperationFuture<Vec<TaskFolder>> {
        self.0
            .worker
            .call_async(move |session| session.list_folders(path, recursive))
    }

    /// Deletes one empty child folder.
    #[must_use]
    pub fn delete_folder(&self, path: FolderPath) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.delete_folder(path))
    }

    /// Reads task SDDL.
    #[must_use]
    pub fn task_security(
        &self,
        path: TaskPath,
        information: SecurityInformation,
    ) -> OperationFuture<SecurityDescriptor> {
        self.0
            .worker
            .call_async(move |session| session.task_security(path, information))
    }

    /// Reads folder SDDL.
    #[must_use]
    pub fn folder_security(
        &self,
        path: FolderPath,
        information: SecurityInformation,
    ) -> OperationFuture<SecurityDescriptor> {
        self.0
            .worker
            .call_async(move |session| session.folder_security(path, information))
    }

    /// Replaces selected task security descriptor portions.
    #[must_use]
    pub fn set_task_security(
        &self,
        path: TaskPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.set_task_security(path, descriptor, information))
    }

    /// Replaces selected folder security descriptor portions.
    #[must_use]
    pub fn set_folder_security(
        &self,
        path: FolderPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> OperationFuture<()> {
        self.0
            .worker
            .call_async(move |session| session.set_folder_security(path, descriptor, information))
    }

    /// Runs target diagnostics.
    pub fn doctor(&self) -> OperationFuture<DiagnosticReport> {
        self.0.worker.call_async(sys::Session::doctor)
    }
}

#[cfg(feature = "history")]
fn wait_for_run(
    scheduler: &BlockingScheduler,
    handle: &RunHandle,
    options: WaitOptions,
    cancelled: Option<&AtomicBool>,
) -> Result<RunOutcome> {
    if options.poll_interval.is_zero() {
        return Err(Error::new(
            ErrorKind::InvalidDefinition,
            "run wait poll interval must be non-zero",
        ));
    }
    let started = Instant::now();
    let since = SystemTime::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut history_usable = true;
    let mut observed_running = false;
    let mut absent_since = None::<Instant>;
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(Error::new(ErrorKind::Cancelled, "run wait was cancelled")
                .with_target(handle.instance_id.to_string()));
        }
        if started.elapsed() >= options.timeout {
            return Err(Error::new(ErrorKind::Timeout, "task run wait elapsed")
                .with_target(handle.instance_id.to_string()));
        }

        if history_usable {
            let query = HistoryQuery {
                task: Some(handle.path.clone()),
                instance_id: Some(handle.instance_id),
                since: Some(since),
                limit: Some(64),
                forward: false,
            };
            match scheduler.history(query) {
                Ok(events) => {
                    for event in events {
                        let result_code = match event.kind {
                            HistoryEventKind::Completed
                            | HistoryEventKind::Failed
                            | HistoryEventKind::Stopped => event.result_code,
                            _ => None,
                        };
                        if let Some(result_code) = result_code {
                            return Ok(RunOutcome {
                                instance_id: handle.instance_id,
                                result_code,
                                confidence: ResultConfidence::Exact,
                            });
                        }
                    }
                }
                Err(_) if options.allow_polling_fallback => history_usable = false,
                Err(error) => return Err(error),
            }
        }

        let running = scheduler.running_tasks(true)?;
        if running
            .iter()
            .any(|instance| instance.instance_id == handle.instance_id)
        {
            observed_running = true;
            absent_since = None;
        } else {
            absent_since.get_or_insert_with(Instant::now);
        }
        let absent_long_enough =
            absent_since.is_some_and(|instant| instant.elapsed() >= options.history_grace);
        let start_grace_elapsed = started.elapsed() >= options.history_grace;
        if options.allow_polling_fallback
            && absent_long_enough
            && (observed_running || start_grace_elapsed)
        {
            let task = scheduler.get_task(&handle.path)?;
            return Ok(RunOutcome {
                instance_id: handle.instance_id,
                result_code: task.last_result,
                confidence: ResultConfidence::PollingFallback,
            });
        }
        if !history_usable && !options.allow_polling_fallback {
            return Err(Error::new(
                ErrorKind::HistoryUnavailable,
                "exact run correlation requires readable Task Scheduler history",
            ));
        }
        let remaining = options.timeout.saturating_sub(started.elapsed());
        thread::sleep(
            options
                .poll_interval
                .min(remaining)
                .min(Duration::from_secs(1)),
        );
    }
}

#[cfg(windows)]
pub(crate) fn parse_task_path(value: &str) -> Result<TaskPath> {
    TaskPath::from_str(value).map_err(|error| Error::new(ErrorKind::InvalidPath, error.to_string()))
}

#[cfg(windows)]
pub(crate) fn parse_folder_path(value: &str) -> Result<FolderPath> {
    FolderPath::from_str(value)
        .map_err(|error| Error::new(ErrorKind::InvalidPath, error.to_string()))
}

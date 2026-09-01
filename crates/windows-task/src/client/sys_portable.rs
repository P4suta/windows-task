#![allow(
    clippy::needless_pass_by_ref_mut,
    reason = "method receivers intentionally mirror the Windows worker implementation"
)]
#![allow(
    clippy::unused_self,
    reason = "portable stubs preserve the Windows session method surface"
)]

use crate::{
    Credential, DiagnosticReport, Error, ErrorKind, FolderPath, Result, TaskPath, ValidationReport,
    model::{LogonType, SecurityDescriptor, TaskDefinition},
    xml::RawTaskXml,
};

use super::{
    Capabilities, ComSecurityPolicy, ConnectionInfo, ListOptions, RegisteredTask,
    RegistrationOptions, RunHandle, RunOptions, RunningTask, SecurityInformation, TaskFolder,
};

#[cfg(feature = "history")]
use crate::history::{HistoryEvent, HistoryQuery};

pub(super) struct ConnectionInput {
    pub(super) target: Option<String>,
    pub(super) credential: Option<Credential>,
    pub(super) com_security: ComSecurityPolicy,
}

pub(super) struct Session {
    _private: (),
}

impl Session {
    pub(super) fn connect(input: ConnectionInput) -> Result<Self> {
        drop((input.credential, input.com_security));
        Err(unsupported().with_target(input.target.unwrap_or_else(|| "local".into())))
    }

    pub(super) fn connection_info(&mut self) -> Result<ConnectionInfo> {
        Err(unsupported())
    }

    pub(super) fn capabilities(&mut self) -> Result<Capabilities> {
        Err(unsupported())
    }

    pub(super) fn doctor(&mut self) -> Result<DiagnosticReport> {
        Err(unsupported())
    }

    #[cfg(feature = "history")]
    pub(super) fn history(&mut self, _query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
        Err(unsupported())
    }

    #[cfg(feature = "history")]
    pub(super) fn set_history_enabled(&mut self, _enabled: bool) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn validate(&mut self, _definition: TaskDefinition) -> Result<ValidationReport> {
        Err(unsupported())
    }

    pub(super) fn get_task(&mut self, _path: TaskPath) -> Result<RegisteredTask> {
        Err(unsupported())
    }

    pub(super) fn list_tasks(
        &mut self,
        _folder: FolderPath,
        _options: ListOptions,
    ) -> Result<Vec<RegisteredTask>> {
        Err(unsupported())
    }

    pub(super) fn register(
        &mut self,
        _path: TaskPath,
        _definition: TaskDefinition,
        _options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        Err(unsupported())
    }

    pub(super) fn register_raw(
        &mut self,
        _path: TaskPath,
        _xml: RawTaskXml,
        _logon_type: LogonType,
        _options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        Err(unsupported())
    }

    pub(super) fn delete_task(&mut self, _path: TaskPath) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn set_enabled(&mut self, _path: TaskPath, _enabled: bool) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn run(&mut self, _path: TaskPath, _options: RunOptions) -> Result<RunHandle> {
        Err(unsupported())
    }

    pub(super) fn stop_all(&mut self, _path: TaskPath) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn stop_instance(&mut self, _instance_id: uuid::Uuid) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn running_tasks(&mut self, _include_hidden: bool) -> Result<Vec<RunningTask>> {
        Err(unsupported())
    }

    pub(super) fn create_folder(
        &mut self,
        _path: FolderPath,
        _security: Option<SecurityDescriptor>,
    ) -> Result<TaskFolder> {
        Err(unsupported())
    }

    pub(super) fn list_folders(
        &mut self,
        _path: FolderPath,
        _recursive: bool,
    ) -> Result<Vec<TaskFolder>> {
        Err(unsupported())
    }

    pub(super) fn delete_folder(&mut self, _path: FolderPath) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn task_security(
        &mut self,
        _path: TaskPath,
        _information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        Err(unsupported())
    }

    pub(super) fn folder_security(
        &mut self,
        _path: FolderPath,
        _information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        Err(unsupported())
    }

    pub(super) fn set_task_security(
        &mut self,
        _path: TaskPath,
        _descriptor: SecurityDescriptor,
        _information: SecurityInformation,
    ) -> Result<()> {
        Err(unsupported())
    }

    pub(super) fn set_folder_security(
        &mut self,
        _path: FolderPath,
        _descriptor: SecurityDescriptor,
        _information: SecurityInformation,
    ) -> Result<()> {
        Err(unsupported())
    }
}

fn unsupported() -> Error {
    Error::new(
        ErrorKind::UnsupportedPlatform,
        "live Task Scheduler operations require Windows",
    )
}

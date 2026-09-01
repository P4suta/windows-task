#![allow(
    unsafe_code,
    reason = "all Task Scheduler COM calls are isolated to this MTA worker module"
)]
#![allow(
    clippy::needless_pass_by_value,
    clippy::needless_pass_by_ref_mut,
    clippy::unused_self,
    reason = "owned worker jobs and Windows/non-Windows session signatures intentionally match"
)]

use std::{ffi::c_void, mem::ManuallyDrop};

#[cfg(feature = "history")]
use std::mem::size_of;

use uuid::Uuid;
use windows::{
    Win32::{
        Foundation::{SYSTEMTIME, VARIANT_BOOL},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoSetProxyBlanket, CoUninitialize, EOAC_DYNAMIC_CLOAKING,
                RPC_C_AUTHN_LEVEL_PKT_PRIVACY, RPC_C_IMP_LEVEL_IMPERSONATE,
            },
            Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement},
            TaskScheduler::{
                IRegisteredTask, IRunningTask, ITaskFolder, ITaskService, TASK_CREATE,
                TASK_CREATE_OR_UPDATE, TASK_DISABLE, TASK_DONT_ADD_PRINCIPAL_ACE, TASK_ENUM_HIDDEN,
                TASK_IGNORE_REGISTRATION_TRIGGERS, TASK_LOGON_GROUP, TASK_LOGON_INTERACTIVE_TOKEN,
                TASK_LOGON_INTERACTIVE_TOKEN_OR_PASSWORD, TASK_LOGON_NONE, TASK_LOGON_PASSWORD,
                TASK_LOGON_S4U, TASK_LOGON_SERVICE_ACCOUNT, TASK_RUN_AS_SELF,
                TASK_RUN_IGNORE_CONSTRAINTS, TASK_RUN_NO_FLAGS, TASK_RUN_USE_SESSION_ID,
                TASK_RUN_USER_SID, TASK_STATE_DISABLED, TASK_STATE_QUEUED, TASK_STATE_READY,
                TASK_STATE_RUNNING, TASK_UPDATE, TASK_VALIDATE_ONLY, TaskScheduler,
            },
            Variant::{
                VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_ARRAY, VT_BSTR,
                VariantTimeToSystemTime,
            },
        },
    },
    core::{BSTR, Error as WindowsError, IUnknown, Interface},
};

#[cfg(feature = "history")]
use windows::{
    Win32::System::EventLog::{
        EVT_HANDLE, EVT_RPC_LOGIN, EVT_VARIANT, EVT_VARIANT_0, EvtChannelConfigEnabled, EvtClose,
        EvtFormatMessage, EvtFormatMessageEvent, EvtNext, EvtOpenChannelConfig,
        EvtOpenPublisherMetadata, EvtOpenSession, EvtQuery, EvtQueryChannelPath,
        EvtQueryForwardDirection, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml,
        EvtRpcLogin, EvtRpcLoginAuthNegotiate, EvtSaveChannelConfig, EvtSetChannelConfigProperty,
        EvtVarTypeBoolean,
    },
    core::{BOOL, PCWSTR, PWSTR},
};

use crate::{
    Credential, Diagnostic, DiagnosticCode, DiagnosticLevel, DiagnosticReport, Error, ErrorKind,
    FolderPath, MAX_RUN_ARGUMENTS, Result, TaskPath, ValidationReport,
    model::{
        LogonType, PrincipalIdentity, SecurityDescriptor, TaskDateTime, TaskDefinition,
        TaskSchemaVersion,
    },
    xml::{self, RawTaskXml, TaskSnapshot},
};

#[cfg(feature = "history")]
use crate::history::{
    HistoryEvent, HistoryQuery, MAX_EVENT_XML_BYTES, OPERATIONAL_CHANNEL, from_event_xml,
};

use super::{
    Capabilities, ComSecurityPolicy, ConnectionInfo, ListOptions, RegisteredTask, RegistrationMode,
    RegistrationOptions, RunHandle, RunOptions, RunningTask, SecurityInformation, TaskFolder,
    TaskState, parse_folder_path, parse_task_path,
};

pub(super) struct ConnectionInput {
    pub(super) target: Option<String>,
    pub(super) credential: Option<Credential>,
    pub(super) com_security: ComSecurityPolicy,
}

pub(super) struct Session {
    service: ITaskService,
    target: Option<String>,
    security_policy: ComSecurityPolicy,
    #[cfg(feature = "history")]
    event_access: EventAccess,
    _apartment: ComApartment,
}

impl Session {
    pub(super) fn connect(input: ConnectionInput) -> Result<Self> {
        let apartment = ComApartment::initialize()?;
        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None::<&IUnknown>, CLSCTX_INPROC_SERVER) }
                .map_err(|error| {
                    native_error(
                        "create Task Scheduler service",
                        input.target.as_deref(),
                        error,
                    )
                })?;

        configure_proxy(&service, input.com_security)?;
        let server = input
            .target
            .as_deref()
            .map_or_else(VARIANT::default, VARIANT::from);
        let (identity, password) = if let Some(credential) = input.credential {
            let (username, password) = credential.into_parts();
            let (user, domain) = split_username(&username);
            (Some((user, domain)), Some(SecretVariant::new(password)))
        } else {
            (None, None)
        };
        let user = identity
            .as_ref()
            .map_or_else(VARIANT::default, |(user, _)| VARIANT::from(user.as_str()));
        let domain = identity
            .as_ref()
            .and_then(|(_, domain)| domain.as_deref())
            .map_or_else(VARIANT::default, VARIANT::from);
        let empty_password = VARIANT::default();
        let password_variant = password
            .as_ref()
            .map_or(&empty_password, SecretVariant::as_variant);
        unsafe { service.Connect(&server, &user, &domain, password_variant) }.map_err(|error| {
            native_error(
                "connect Task Scheduler service",
                input.target.as_deref(),
                error,
            )
        })?;
        configure_proxy(&service, input.com_security)?;
        #[cfg(feature = "history")]
        let event_access = EventAccess::connect(
            input.target.as_deref(),
            identity.as_ref(),
            password.as_ref(),
        );
        Ok(Self {
            service,
            target: input.target,
            security_policy: input.com_security,
            #[cfg(feature = "history")]
            event_access,
            _apartment: apartment,
        })
    }

    pub(super) fn connection_info(&mut self) -> Result<ConnectionInfo> {
        let target_server =
            bstr_string(unsafe { self.service.TargetServer() }, "read target server")?;
        let user = nonempty_bstr(
            unsafe { self.service.ConnectedUser() },
            "read connected user",
        )?;
        let domain = nonempty_bstr(
            unsafe { self.service.ConnectedDomain() },
            "read connected domain",
        )?;
        let highest_version = unsafe { self.service.HighestVersion() }
            .map_err(|error| self.native("read highest version", error))?;
        Ok(ConnectionInfo {
            target_server,
            user,
            domain,
            highest_version,
        })
    }

    pub(super) fn capabilities(&mut self) -> Result<Capabilities> {
        let highest_version = unsafe { self.service.HighestVersion() }
            .map_err(|error| self.native("read highest version", error))?;
        let minor = highest_version & 0xFFFF;
        let schema_version = match minor {
            0..=2 => TaskSchemaVersion::V1_2,
            3 => TaskSchemaVersion::V1_3,
            4 => TaskSchemaVersion::V1_4,
            5 => TaskSchemaVersion::V1_5,
            _ => TaskSchemaVersion::V1_6,
        };
        #[cfg(feature = "history")]
        let history_query = self.event_access.probe().is_ok();
        #[cfg(not(feature = "history"))]
        let history_query = false;
        Ok(Capabilities {
            highest_version,
            schema_version,
            required_privileges: minor >= 3,
            maintenance_settings: minor >= 4,
            volatile_tasks: minor >= 6,
            history_query,
            remote_history_query: self.target.is_some() && history_query,
        })
    }

    pub(super) fn doctor(&mut self) -> Result<DiagnosticReport> {
        let connection = self.connection_info()?;
        let capabilities = self.capabilities()?;
        let mut diagnostics = vec![Diagnostic {
            level: DiagnosticLevel::Info,
            code: DiagnosticCode::Other("connected".into()),
            path: "scheduler.connect".into(),
            message: format!(
                "connected to {} as {}",
                connection.target_server,
                connection.user.as_deref().unwrap_or("current token")
            ),
            remediation: None,
        }];
        let root = self.folder(&FolderPath::root());
        match root {
            Ok(_) => diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Info,
                code: DiagnosticCode::Other("root_readable".into()),
                path: "scheduler.root".into(),
                message: "scheduler root is readable".into(),
                remediation: None,
            }),
            Err(error) => diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: if error.kind() == ErrorKind::AccessDenied {
                    DiagnosticCode::InsufficientRights
                } else {
                    DiagnosticCode::RemoteConnectivity
                },
                path: "scheduler.root".into(),
                message: error.to_string(),
                remediation: Some(
                    "verify the Schedule service, RPC/DCOM firewall rules, and caller rights"
                        .into(),
                ),
            }),
        }
        if !capabilities.history_query {
            #[cfg(feature = "history")]
            let detail = self
                .event_access
                .unavailable_reason()
                .unwrap_or("Operational history is disabled or inaccessible");
            #[cfg(not(feature = "history"))]
            let detail = "the crate was built without the history feature";
            diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Warning,
                code: DiagnosticCode::HistoryUnavailable,
                path: "history.operational".into(),
                message: detail.into(),
                remediation: Some(
                    "enable and grant read access to Microsoft-Windows-TaskScheduler/Operational"
                        .into(),
                ),
            });
        }
        Ok(DiagnosticReport {
            target: Some(connection.target_server),
            diagnostics,
        })
    }

    #[cfg(feature = "history")]
    pub(super) fn history(&mut self, query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
        self.event_access.query(query)
    }

    #[cfg(feature = "history")]
    pub(super) fn set_history_enabled(&mut self, enabled: bool) -> Result<()> {
        self.event_access.set_enabled(enabled)
    }

    pub(super) fn validate(&mut self, definition: TaskDefinition) -> Result<ValidationReport> {
        let report = definition.validate();
        if !report.is_valid() {
            return Ok(report);
        }
        let xml = xml::to_string(&definition)?;
        let native = unsafe { self.service.NewTask(0) }
            .map_err(|error| self.native("create validation definition", error))?;
        configure_proxy(&native, self.security_policy)?;
        unsafe { native.SetXmlText(&BSTR::from(xml)) }
            .map_err(|error| self.native("validate task XML", error))?;
        Ok(report)
    }

    pub(super) fn get_task(&mut self, path: TaskPath) -> Result<RegisteredTask> {
        let task = self.registered_task(&path)?;
        self.task_info(task)
    }

    pub(super) fn list_tasks(
        &mut self,
        folder: FolderPath,
        options: ListOptions,
    ) -> Result<Vec<RegisteredTask>> {
        let mut output = Vec::new();
        self.list_tasks_into(&folder, options, &mut output)?;
        Ok(output)
    }

    pub(super) fn register(
        &mut self,
        path: TaskPath,
        definition: TaskDefinition,
        options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        let logon_type = definition.principal.logon_type;
        let xml = RawTaskXml::new(xml::to_string(&definition)?.into_bytes())?;
        self.register_raw(path, xml, logon_type, options)
    }

    pub(super) fn register_raw(
        &mut self,
        path: TaskPath,
        xml: RawTaskXml,
        logon_type: LogonType,
        mut options: RegistrationOptions,
    ) -> Result<RegisteredTask> {
        let definition = xml.definition()?;
        let validation = definition.validate();
        if !validation.is_valid() {
            return Err(Error::new(
                ErrorKind::InvalidDefinition,
                "task definition failed portable validation",
            )
            .with_target(path.to_string()));
        }
        if matches!(
            logon_type,
            LogonType::Password | LogonType::InteractiveTokenOrPassword
        ) && options.password.is_none()
        {
            return Err(Error::new(
                ErrorKind::Authentication,
                "this logon type requires a separately supplied registration password",
            )
            .with_target(path.to_string()));
        }
        let folder = self.folder(&path.folder())?;
        let xml_text = xml.decoded()?;
        let user_id = principal_user(&definition);
        let user = user_id
            .as_deref()
            .map_or_else(VARIANT::default, VARIANT::from);
        let password = options.password.take().map(|password| {
            let units = password.into_utf16();
            SecretVariant::new(units)
        });
        let empty_password = VARIANT::default();
        let password_variant = password
            .as_ref()
            .map_or(&empty_password, SecretVariant::as_variant);
        let sddl_text = definition
            .registration
            .security_descriptor
            .as_ref()
            .map(|descriptor| descriptor.as_sddl().to_owned());
        let sddl = sddl_text
            .as_deref()
            .map_or_else(VARIANT::default, VARIANT::from);
        let flags = registration_flags(&options);
        let registered = unsafe {
            folder.RegisterTask(
                &BSTR::from(path.name()),
                &BSTR::from(xml_text),
                flags,
                &user,
                password_variant,
                native_logon_type(logon_type),
                &sddl,
            )
        }
        .map_err(|error| self.native_for("register task", &path.to_string(), error))?;
        configure_proxy(&registered, self.security_policy)?;
        self.task_info(registered)
    }

    pub(super) fn delete_task(&mut self, path: TaskPath) -> Result<()> {
        let folder = self.folder(&path.folder())?;
        unsafe { folder.DeleteTask(&BSTR::from(path.name()), 0) }
            .map_err(|error| self.native_for("delete task", &path.to_string(), error))
    }

    pub(super) fn set_enabled(&mut self, path: TaskPath, enabled: bool) -> Result<()> {
        let task = self.registered_task(&path)?;
        unsafe { task.SetEnabled(VARIANT_BOOL::from(enabled)) }
            .map_err(|error| self.native_for("set task enabled", &path.to_string(), error))
    }

    pub(super) fn run(&mut self, path: TaskPath, options: RunOptions) -> Result<RunHandle> {
        if options.parameters.len() > MAX_RUN_ARGUMENTS {
            return Err(Error::new(
                ErrorKind::InvalidDefinition,
                "Run and RunEx accept at most 32 parameter substitutions",
            ));
        }
        let task = self.registered_task(&path)?;
        let parameters = string_array_variant(&options.parameters)?;
        let running = if options.session_id.is_some()
            || options.user_sid.is_some()
            || options.ignore_constraints
            || options.as_self
        {
            let mut flags = TASK_RUN_NO_FLAGS.0;
            if options.ignore_constraints {
                flags |= TASK_RUN_IGNORE_CONSTRAINTS.0;
            }
            if options.as_self {
                flags |= TASK_RUN_AS_SELF.0;
            }
            if options.session_id.is_some() {
                flags |= TASK_RUN_USE_SESSION_ID.0;
            }
            if options.user_sid.is_some() {
                flags |= TASK_RUN_USER_SID.0;
            }
            unsafe {
                task.RunEx(
                    &parameters,
                    flags,
                    options
                        .session_id
                        .map(i32::try_from)
                        .transpose()
                        .map_err(|_| {
                            Error::new(
                                ErrorKind::InvalidDefinition,
                                "session id exceeds the RunEx signed range",
                            )
                        })?
                        .unwrap_or(0),
                    &BSTR::from(options.user_sid.as_deref().unwrap_or("")),
                )
            }
        } else {
            unsafe { task.Run(&parameters) }
        }
        .map_err(|error| self.native_for("run task", &path.to_string(), error))?;
        configure_proxy(&running, self.security_policy)?;
        let instance_id = running_instance_id(&running)?;
        let engine_process_id = unsafe { running.EnginePID() }.ok().filter(|pid| *pid != 0);
        Ok(RunHandle {
            instance_id,
            path,
            engine_process_id,
        })
    }

    pub(super) fn stop_all(&mut self, path: TaskPath) -> Result<()> {
        let task = self.registered_task(&path)?;
        unsafe { task.Stop(0) }
            .map_err(|error| self.native_for("stop task instances", &path.to_string(), error))
    }

    pub(super) fn stop_instance(&mut self, instance_id: Uuid) -> Result<()> {
        let collection = unsafe { self.service.GetRunningTasks(TASK_ENUM_HIDDEN.0) }
            .map_err(|error| self.native("enumerate running tasks", error))?;
        configure_proxy(&collection, self.security_policy)?;
        let count = unsafe { collection.Count() }
            .map_err(|error| self.native("count running tasks", error))?;
        for index in 1..=count {
            let running = unsafe { collection.get_Item(&VARIANT::from(index)) }
                .map_err(|error| self.native("read running task", error))?;
            configure_proxy(&running, self.security_policy)?;
            if running_instance_id(&running)? == instance_id {
                return unsafe { running.Stop() }
                    .map_err(|error| self.native("stop running task", error));
            }
        }
        Err(
            Error::new(ErrorKind::NotFound, "running task instance was not found")
                .with_target(instance_id.to_string()),
        )
    }

    pub(super) fn running_tasks(&mut self, include_hidden: bool) -> Result<Vec<RunningTask>> {
        let flags = if include_hidden {
            TASK_ENUM_HIDDEN.0
        } else {
            0
        };
        let collection = unsafe { self.service.GetRunningTasks(flags) }
            .map_err(|error| self.native("enumerate running tasks", error))?;
        configure_proxy(&collection, self.security_policy)?;
        let count = unsafe { collection.Count() }
            .map_err(|error| self.native("count running tasks", error))?;
        (1..=count)
            .map(|index| {
                let running = unsafe { collection.get_Item(&VARIANT::from(index)) }
                    .map_err(|error| self.native("read running task", error))?;
                self.running_info(&running)
            })
            .collect()
    }

    pub(super) fn create_folder(
        &mut self,
        path: FolderPath,
        security: Option<SecurityDescriptor>,
    ) -> Result<TaskFolder> {
        if path.is_root() {
            return Err(Error::new(
                ErrorKind::InvalidPath,
                "cannot create scheduler root",
            ));
        }
        let parent_path = path.parent().expect("non-root folder has parent");
        let parent = self.folder(&parent_path)?;
        let sddl_text = security.as_ref().map(|value| value.as_sddl().to_owned());
        let sddl = sddl_text
            .as_deref()
            .map_or_else(VARIANT::default, VARIANT::from);
        let folder = unsafe {
            parent.CreateFolder(
                &BSTR::from(path.name().expect("non-root folder has name")),
                &sddl,
            )
        }
        .map_err(|error| self.native_for("create folder", path.as_str(), error))?;
        configure_proxy(&folder, self.security_policy)?;
        Ok(TaskFolder {
            path,
            security_descriptor: security,
        })
    }

    pub(super) fn list_folders(
        &mut self,
        path: FolderPath,
        recursive: bool,
    ) -> Result<Vec<TaskFolder>> {
        let mut output = Vec::new();
        self.list_folders_into(&path, recursive, &mut output)?;
        Ok(output)
    }

    pub(super) fn delete_folder(&mut self, path: FolderPath) -> Result<()> {
        if path.is_root() {
            return Err(Error::new(
                ErrorKind::InvalidPath,
                "cannot delete scheduler root",
            ));
        }
        let parent = self.folder(&path.parent().expect("non-root folder has parent"))?;
        unsafe { parent.DeleteFolder(&BSTR::from(path.name().expect("folder name")), 0) }
            .map_err(|error| self.native_for("delete folder", path.as_str(), error))
    }

    pub(super) fn task_security(
        &mut self,
        path: TaskPath,
        information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        let task = self.registered_task(&path)?;
        let security_information =
            i32::try_from(information.bits()).expect("security information flags fit in i32");
        let sddl = unsafe { task.GetSecurityDescriptor(security_information) }
            .map_err(|error| self.native_for("read task security", path.as_str(), error))?;
        SecurityDescriptor::from_sddl(bstr_to_string(sddl, "decode task security")?)
            .map_err(|error| Error::new(ErrorKind::Win32, error.to_string()))
    }

    pub(super) fn folder_security(
        &mut self,
        path: FolderPath,
        information: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        let folder = self.folder(&path)?;
        let security_information =
            i32::try_from(information.bits()).expect("security information flags fit in i32");
        let sddl = unsafe { folder.GetSecurityDescriptor(security_information) }
            .map_err(|error| self.native_for("read folder security", path.as_str(), error))?;
        SecurityDescriptor::from_sddl(bstr_to_string(sddl, "decode folder security")?)
            .map_err(|error| Error::new(ErrorKind::Win32, error.to_string()))
    }

    pub(super) fn set_task_security(
        &mut self,
        path: TaskPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> Result<()> {
        let task = self.registered_task(&path)?;
        unsafe {
            task.SetSecurityDescriptor(
                &BSTR::from(descriptor.as_sddl()),
                i32::try_from(information.bits()).expect("security information flags fit in i32"),
            )
        }
        .map_err(|error| self.native_for("set task security", path.as_str(), error))
    }

    pub(super) fn set_folder_security(
        &mut self,
        path: FolderPath,
        descriptor: SecurityDescriptor,
        information: SecurityInformation,
    ) -> Result<()> {
        let folder = self.folder(&path)?;
        unsafe {
            folder.SetSecurityDescriptor(
                &BSTR::from(descriptor.as_sddl()),
                i32::try_from(information.bits()).expect("security information flags fit in i32"),
            )
        }
        .map_err(|error| self.native_for("set folder security", path.as_str(), error))
    }

    fn folder(&self, path: &FolderPath) -> Result<ITaskFolder> {
        let folder = unsafe { self.service.GetFolder(&BSTR::from(path.as_str())) }
            .map_err(|error| self.native_for("open folder", path.as_str(), error))?;
        configure_proxy(&folder, self.security_policy)?;
        Ok(folder)
    }

    fn registered_task(&self, path: &TaskPath) -> Result<IRegisteredTask> {
        let folder = self.folder(&path.folder())?;
        let task = unsafe { folder.GetTask(&BSTR::from(path.name())) }
            .map_err(|error| self.native_for("open task", path.as_str(), error))?;
        configure_proxy(&task, self.security_policy)?;
        Ok(task)
    }

    fn task_info(&self, task: IRegisteredTask) -> Result<RegisteredTask> {
        configure_proxy(&task, self.security_policy)?;
        let path_text = bstr_string(unsafe { task.Path() }, "read task path")?;
        let path = parse_task_path(&path_text)?;
        let state = map_state(
            unsafe { task.State() }
                .map_err(|error| self.native_for("read task state", path.as_str(), error))?,
        );
        let enabled = bool::from(
            unsafe { task.Enabled() }
                .map_err(|error| self.native_for("read task enabled", path.as_str(), error))?,
        );
        let last_result = unsafe { task.LastTaskResult() }
            .map_err(|error| self.native_for("read last task result", path.as_str(), error))?;
        let missed_runs = unsafe { task.NumberOfMissedRuns() }
            .map_err(|error| self.native_for("read missed runs", path.as_str(), error))?
            .max(0);
        let missed_runs = u32::try_from(missed_runs).expect("non-negative missed run count");
        let last_run = automation_time(
            unsafe { task.LastRunTime() }
                .map_err(|error| self.native_for("read last run time", path.as_str(), error))?,
        )?;
        let next_run = automation_time(
            unsafe { task.NextRunTime() }
                .map_err(|error| self.native_for("read next run time", path.as_str(), error))?,
        )?;
        let xml = bstr_string(unsafe { task.Xml() }, "read task XML")?;
        let snapshot = TaskSnapshot::parse(xml.into_bytes())?;
        Ok(RegisteredTask {
            path,
            state,
            enabled,
            last_result,
            missed_runs,
            last_run,
            next_run,
            snapshot,
        })
    }

    fn running_info(&self, running: &IRunningTask) -> Result<RunningTask> {
        configure_proxy(running, self.security_policy)?;
        let path_text = bstr_string(unsafe { running.Path() }, "read running path")?;
        let path = parse_task_path(&path_text)?;
        let instance_id = running_instance_id(running)?;
        let current_action = nonempty_bstr(
            unsafe { running.CurrentAction() },
            "read running current action",
        )?;
        let engine_process_id = unsafe { running.EnginePID() }
            .map_err(|error| self.native_for("read task engine PID", path.as_str(), error))?;
        let state = map_state(
            unsafe { running.State() }
                .map_err(|error| self.native_for("read running state", path.as_str(), error))?,
        );
        Ok(RunningTask {
            path,
            instance_id,
            current_action,
            engine_process_id,
            state,
        })
    }

    fn list_tasks_into(
        &self,
        folder_path: &FolderPath,
        options: ListOptions,
        output: &mut Vec<RegisteredTask>,
    ) -> Result<()> {
        let folder = self.folder(folder_path)?;
        let flags = if options.include_hidden {
            TASK_ENUM_HIDDEN.0
        } else {
            0
        };
        let tasks = unsafe { folder.GetTasks(flags) }
            .map_err(|error| self.native_for("enumerate tasks", folder_path.as_str(), error))?;
        configure_proxy(&tasks, self.security_policy)?;
        let count = unsafe { tasks.Count() }
            .map_err(|error| self.native_for("count tasks", folder_path.as_str(), error))?;
        for index in 1..=count {
            let task = unsafe { tasks.get_Item(&VARIANT::from(index)) }.map_err(|error| {
                self.native_for("read task collection", folder_path.as_str(), error)
            })?;
            output.push(self.task_info(task)?);
        }
        if options.recursive {
            let folders = unsafe { folder.GetFolders(0) }.map_err(|error| {
                self.native_for("enumerate folders", folder_path.as_str(), error)
            })?;
            configure_proxy(&folders, self.security_policy)?;
            let count = unsafe { folders.Count() }
                .map_err(|error| self.native_for("count folders", folder_path.as_str(), error))?;
            for index in 1..=count {
                let child =
                    unsafe { folders.get_Item(&VARIANT::from(index)) }.map_err(|error| {
                        self.native_for("read folder collection", folder_path.as_str(), error)
                    })?;
                configure_proxy(&child, self.security_policy)?;
                let child_path_text = bstr_string(unsafe { child.Path() }, "read folder path")?;
                let child_path = parse_folder_path(&child_path_text)?;
                self.list_tasks_into(&child_path, options, output)?;
            }
        }
        Ok(())
    }

    fn list_folders_into(
        &self,
        folder_path: &FolderPath,
        recursive: bool,
        output: &mut Vec<TaskFolder>,
    ) -> Result<()> {
        let folder = self.folder(folder_path)?;
        let folders = unsafe { folder.GetFolders(0) }
            .map_err(|error| self.native_for("enumerate folders", folder_path.as_str(), error))?;
        configure_proxy(&folders, self.security_policy)?;
        let count = unsafe { folders.Count() }
            .map_err(|error| self.native_for("count folders", folder_path.as_str(), error))?;
        for index in 1..=count {
            let child = unsafe { folders.get_Item(&VARIANT::from(index)) }.map_err(|error| {
                self.native_for("read folder collection", folder_path.as_str(), error)
            })?;
            configure_proxy(&child, self.security_policy)?;
            let child_path_text = bstr_string(unsafe { child.Path() }, "read folder path")?;
            let child_path = parse_folder_path(&child_path_text)?;
            output.push(TaskFolder {
                path: child_path.clone(),
                security_descriptor: None,
            });
            if recursive {
                self.list_folders_into(&child_path, true, output)?;
            }
        }
        Ok(())
    }

    fn native(&self, operation: &str, error: WindowsError) -> Error {
        native_error(operation, self.target.as_deref(), error)
    }

    fn native_for(&self, operation: &str, target: &str, error: WindowsError) -> Error {
        native_error(operation, Some(target), error)
    }
}

#[cfg(feature = "history")]
enum EventAccess {
    Local,
    Remote(EventHandle),
    Unavailable(Error),
}

#[cfg(feature = "history")]
impl EventAccess {
    fn connect(
        target: Option<&str>,
        identity: Option<&(String, Option<String>)>,
        password: Option<&SecretVariant>,
    ) -> Self {
        let Some(target) = target else {
            return Self::Local;
        };
        match open_event_session(target, identity, password) {
            Ok(handle) => Self::Remote(handle),
            Err(error) => Self::Unavailable(error),
        }
    }

    fn session(&self) -> Result<Option<EVT_HANDLE>> {
        match self {
            Self::Local => Ok(None),
            Self::Remote(handle) => Ok(Some(handle.raw())),
            Self::Unavailable(error) => Err(error.clone()),
        }
    }

    fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Unavailable(error) => Some(error.message()),
            Self::Local | Self::Remote(_) => None,
        }
    }

    fn probe(&self) -> Result<()> {
        let session = self.session()?;
        let channel = BSTR::from(OPERATIONAL_CHANNEL);
        let query = BSTR::from("*");
        let handle = unsafe {
            EvtQuery(
                session,
                &channel,
                &query,
                EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
            )
        }
        .map_err(|error| event_error("open Task Scheduler history", error))?;
        drop(EventHandle(handle));
        Ok(())
    }

    fn query(&self, query: HistoryQuery) -> Result<Vec<HistoryEvent>> {
        const DEFAULT_LIMIT: usize = 256;
        const MAX_RETURNED: usize = 100_000;
        const MAX_SCANNED: usize = 100_000;

        let wanted = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_RETURNED);
        if wanted == 0 {
            return Ok(Vec::new());
        }
        let session = self.session()?;
        let direction = if query.forward {
            EvtQueryForwardDirection.0
        } else {
            EvtQueryReverseDirection.0
        };
        let channel = BSTR::from(OPERATIONAL_CHANNEL);
        let xpath = BSTR::from("*");
        let result_set = EventHandle(
            unsafe { EvtQuery(session, &channel, &xpath, EvtQueryChannelPath.0 | direction) }
                .map_err(|error| event_error("query Task Scheduler history", error))?,
        );
        let provider = open_task_scheduler_metadata(session).ok();
        let mut output = Vec::with_capacity(wanted.min(256));
        let mut scanned = 0_usize;
        while output.len() < wanted && scanned < MAX_SCANNED {
            let mut raw_events = [0_isize; 16];
            let mut returned = 0_u32;
            let next = unsafe {
                EvtNext(
                    result_set.raw(),
                    &mut raw_events,
                    u32::MAX,
                    0,
                    &raw mut returned,
                )
            };
            if let Err(error) = next {
                if is_no_more_items(&error) {
                    break;
                }
                return Err(event_error("read Task Scheduler history", error));
            }
            if returned == 0 {
                break;
            }
            for raw in raw_events
                .into_iter()
                .take(usize::try_from(returned).expect("EvtNext count fits usize"))
            {
                let event_handle = EventHandle(EVT_HANDLE(raw));
                scanned += 1;
                let xml = render_event_xml(event_handle.raw())?;
                let mut event = from_event_xml(&xml)?;
                if let Some(metadata) = provider.as_ref() {
                    event.message = format_event_message(metadata.raw(), event_handle.raw())
                        .ok()
                        .or(event.message);
                }
                if query
                    .task
                    .as_ref()
                    .is_some_and(|path| event.task_path.as_ref() != Some(path))
                    || query
                        .instance_id
                        .is_some_and(|instance| event.instance_id != Some(instance))
                    || query.since.is_some_and(|since| event.timestamp < since)
                {
                    continue;
                }
                output.push(event);
                if output.len() == wanted {
                    break;
                }
            }
        }
        Ok(output)
    }

    fn set_enabled(&self, enabled: bool) -> Result<()> {
        let session = self.session()?;
        let channel = BSTR::from(OPERATIONAL_CHANNEL);
        let config = EventHandle(
            unsafe { EvtOpenChannelConfig(session, &channel, 0) }
                .map_err(|error| event_error("open Task Scheduler history configuration", error))?,
        );
        let value = EVT_VARIANT {
            Anonymous: EVT_VARIANT_0 {
                BooleanVal: BOOL::from(enabled),
            },
            Count: 0,
            Type: u32::try_from(EvtVarTypeBoolean.0).expect("variant type is non-negative"),
        };
        unsafe {
            EvtSetChannelConfigProperty(config.raw(), EvtChannelConfigEnabled, 0, &raw const value)
        }
        .map_err(|error| event_error("change Task Scheduler history state", error))?;
        unsafe { EvtSaveChannelConfig(config.raw(), 0) }
            .map_err(|error| event_error("save Task Scheduler history state", error))
    }
}

#[cfg(feature = "history")]
struct EventHandle(EVT_HANDLE);

#[cfg(feature = "history")]
impl EventHandle {
    const fn raw(&self) -> EVT_HANDLE {
        self.0
    }
}

#[cfg(feature = "history")]
impl Drop for EventHandle {
    fn drop(&mut self) {
        drop(unsafe { EvtClose(self.0) });
    }
}

#[cfg(feature = "history")]
struct WideString(Vec<u16>);

#[cfg(feature = "history")]
impl WideString {
    fn new(value: &str) -> Self {
        let mut units: Vec<_> = value.encode_utf16().collect();
        units.push(0);
        Self(units)
    }

    fn as_pwstr(&self) -> PWSTR {
        PWSTR::from_raw(self.0.as_ptr().cast_mut())
    }
}

#[cfg(feature = "history")]
fn open_event_session(
    target: &str,
    identity: Option<&(String, Option<String>)>,
    password: Option<&SecretVariant>,
) -> Result<EventHandle> {
    let server = WideString::new(target);
    let user = identity.map(|(user, _)| WideString::new(user));
    let domain = identity.and_then(|(_, domain)| domain.as_deref().map(WideString::new));
    let login = EVT_RPC_LOGIN {
        Server: server.as_pwstr(),
        User: user.as_ref().map_or_else(PWSTR::null, WideString::as_pwstr),
        Domain: domain
            .as_ref()
            .map_or_else(PWSTR::null, WideString::as_pwstr),
        Password: password.map_or_else(PWSTR::null, SecretVariant::as_pwstr),
        Flags: EvtRpcLoginAuthNegotiate.0,
    };
    let handle = unsafe {
        EvtOpenSession(
            EvtRpcLogin,
            (&raw const login).cast::<c_void>(),
            Some(5_000),
            Some(0),
        )
    }
    .map_err(|error| native_error("connect remote Windows Event Log", Some(target), error))?;
    Ok(EventHandle(handle))
}

#[cfg(feature = "history")]
fn open_task_scheduler_metadata(session: Option<EVT_HANDLE>) -> Result<EventHandle> {
    let provider = BSTR::from("Microsoft-Windows-TaskScheduler");
    unsafe { EvtOpenPublisherMetadata(session, &provider, PCWSTR::null(), 0, 0) }
        .map(EventHandle)
        .map_err(|error| event_error("open Task Scheduler event metadata", error))
}

#[cfg(feature = "history")]
fn render_event_xml(event: EVT_HANDLE) -> Result<String> {
    let mut bytes_used = 0_u32;
    let mut property_count = 0_u32;
    let sizing = unsafe {
        EvtRender(
            None,
            event,
            EvtRenderEventXml.0,
            0,
            None,
            &raw mut bytes_used,
            &raw mut property_count,
        )
    };
    if bytes_used == 0 {
        return match sizing {
            Err(error) => Err(event_error("size rendered Task Scheduler event", error)),
            Ok(()) => Err(Error::new(
                ErrorKind::HistoryUnavailable,
                "Windows Event Log returned empty rendered event XML",
            )),
        };
    }
    if usize::try_from(bytes_used).unwrap_or(usize::MAX) > MAX_EVENT_XML_BYTES {
        return Err(Error::new(
            ErrorKind::HistoryUnavailable,
            "rendered Task Scheduler event exceeds the 1 MiB limit",
        ));
    }
    let units = usize::try_from(bytes_used)
        .expect("u32 fits usize")
        .div_ceil(size_of::<u16>());
    let mut buffer = vec![0_u16; units];
    unsafe {
        EvtRender(
            None,
            event,
            EvtRenderEventXml.0,
            bytes_used,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            &raw mut bytes_used,
            &raw mut property_count,
        )
    }
    .map_err(|error| event_error("render Task Scheduler event", error))?;
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..length]).map_err(|error| {
        Error::new(
            ErrorKind::HistoryUnavailable,
            format!("rendered Task Scheduler event is invalid UTF-16: {error}"),
        )
    })
}

#[cfg(feature = "history")]
fn format_event_message(metadata: EVT_HANDLE, event: EVT_HANDLE) -> Result<String> {
    let mut units_used = 0_u32;
    let sizing = unsafe {
        EvtFormatMessage(
            Some(metadata),
            Some(event),
            0,
            None,
            EvtFormatMessageEvent.0,
            None,
            &raw mut units_used,
        )
    };
    if units_used == 0 {
        return match sizing {
            Err(error) => Err(event_error("size Task Scheduler event message", error)),
            Ok(()) => Ok(String::new()),
        };
    }
    if usize::try_from(units_used).unwrap_or(usize::MAX) > MAX_EVENT_XML_BYTES / 2 {
        return Err(Error::new(
            ErrorKind::HistoryUnavailable,
            "formatted Task Scheduler event message exceeds the limit",
        ));
    }
    let mut buffer = vec![0_u16; usize::try_from(units_used).expect("u32 fits usize")];
    unsafe {
        EvtFormatMessage(
            Some(metadata),
            Some(event),
            0,
            None,
            EvtFormatMessageEvent.0,
            Some(&mut buffer),
            &raw mut units_used,
        )
    }
    .map_err(|error| event_error("format Task Scheduler event message", error))?;
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..length]).map_err(|error| {
        Error::new(
            ErrorKind::HistoryUnavailable,
            format!("formatted Task Scheduler event message is invalid UTF-16: {error}"),
        )
    })
}

#[cfg(feature = "history")]
fn is_no_more_items(error: &WindowsError) -> bool {
    u32::from_ne_bytes(error.code().0.to_ne_bytes()) == 0x8007_0103
}

#[cfg(feature = "history")]
fn event_error(operation: &str, error: WindowsError) -> Error {
    native_error(operation, Some(OPERATIONAL_CHANNEL), error)
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| native_error("initialize COM MTA", None, error))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct SecretVariant {
    variant: VARIANT,
    units: usize,
}

impl SecretVariant {
    fn new(units: zeroize::Zeroizing<Vec<u16>>) -> Self {
        let units_len = units.len();
        let variant = VARIANT::from(BSTR::from_wide(&units));
        drop(units);
        Self {
            variant,
            units: units_len,
        }
    }

    const fn as_variant(&self) -> &VARIANT {
        &self.variant
    }

    #[cfg(feature = "history")]
    fn as_pwstr(&self) -> PWSTR {
        unsafe {
            let inner = &*self.variant.Anonymous.Anonymous;
            let bstr = &*inner.Anonymous.bstrVal;
            PWSTR::from_raw(bstr.as_ptr().cast_mut())
        }
    }
}

impl Drop for SecretVariant {
    fn drop(&mut self) {
        unsafe {
            let inner = &mut *self.variant.Anonymous.Anonymous;
            if inner.vt == VT_BSTR {
                let bstr = &mut *inner.Anonymous.bstrVal;
                let pointer = bstr.as_ptr().cast_mut();
                for index in 0..self.units {
                    pointer.add(index).write_volatile(0);
                }
            }
        }
    }
}

fn configure_proxy<T: Interface>(object: &T, policy: ComSecurityPolicy) -> Result<()> {
    let unknown: IUnknown = object
        .cast()
        .map_err(|error| native_error("query COM proxy identity", None, error))?;
    let result = unsafe {
        CoSetProxyBlanket(
            &unknown,
            10,
            0,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_DYNAMIC_CLOAKING,
        )
    };
    match (policy, result) {
        (_, Ok(())) | (ComSecurityPolicy::ExistingProcess, Err(_)) => Ok(()),
        (ComSecurityPolicy::RequireProxyBlanket, Err(error)) => {
            Err(native_error("configure COM proxy security", None, error))
        }
    }
}

fn split_username(username: &str) -> (String, Option<String>) {
    username.split_once('\\').map_or_else(
        || (username.into(), None),
        |(domain, user)| (user.into(), Some(domain.into())),
    )
}

fn principal_user(definition: &TaskDefinition) -> Option<String> {
    match &definition.principal.identity {
        PrincipalIdentity::None => None,
        PrincipalIdentity::User(value) | PrincipalIdentity::Group(value) => Some(value.clone()),
        PrincipalIdentity::ServiceAccount(account) => Some(
            match account {
                crate::model::ServiceAccount::LocalSystem => "SYSTEM",
                crate::model::ServiceAccount::LocalService => "NT AUTHORITY\\LOCAL SERVICE",
                crate::model::ServiceAccount::NetworkService => "NT AUTHORITY\\NETWORK SERVICE",
            }
            .into(),
        ),
    }
}

const fn native_logon_type(
    logon_type: LogonType,
) -> windows::Win32::System::TaskScheduler::TASK_LOGON_TYPE {
    match logon_type {
        LogonType::None => TASK_LOGON_NONE,
        LogonType::Password => TASK_LOGON_PASSWORD,
        LogonType::InteractiveToken => TASK_LOGON_INTERACTIVE_TOKEN,
        LogonType::S4u => TASK_LOGON_S4U,
        LogonType::Group => TASK_LOGON_GROUP,
        LogonType::ServiceAccount => TASK_LOGON_SERVICE_ACCOUNT,
        LogonType::InteractiveTokenOrPassword => TASK_LOGON_INTERACTIVE_TOKEN_OR_PASSWORD,
    }
}

fn registration_flags(options: &RegistrationOptions) -> i32 {
    if matches!(options.mode, RegistrationMode::ValidateOnly) {
        return TASK_VALIDATE_ONLY.0;
    }
    let mut flags = match options.mode {
        RegistrationMode::Create => TASK_CREATE.0,
        RegistrationMode::Update => TASK_UPDATE.0,
        RegistrationMode::CreateOrUpdate => TASK_CREATE_OR_UPDATE.0,
        RegistrationMode::ValidateOnly => unreachable!("validate-only returned above"),
    };
    if options.ignore_registration_triggers {
        flags |= TASK_IGNORE_REGISTRATION_TRIGGERS.0;
    }
    if options.disabled {
        flags |= TASK_DISABLE.0;
    }
    if options.dont_add_principal_ace {
        flags |= TASK_DONT_ADD_PRINCIPAL_ACE.0;
    }
    flags
}

fn string_array_variant(values: &[String]) -> Result<VARIANT> {
    if values.is_empty() {
        return Ok(VARIANT::default());
    }
    let array = unsafe {
        SafeArrayCreateVector(
            VT_BSTR,
            0,
            values.len().try_into().map_err(|_| {
                Error::new(ErrorKind::InvalidDefinition, "too many RunEx arguments")
            })?,
        )
    };
    if array.is_null() {
        return Err(Error::new(ErrorKind::Com, "SafeArrayCreateVector failed"));
    }
    for (index, value) in values.iter().enumerate() {
        let bstr = BSTR::from(value);
        let native_index = i32::try_from(index)
            .map_err(|_| Error::new(ErrorKind::InvalidDefinition, "argument index overflow"))?;
        if let Err(error) = unsafe {
            SafeArrayPutElement(
                array,
                &raw const native_index,
                (&raw const bstr).cast::<c_void>(),
            )
        } {
            drop(unsafe { SafeArrayDestroy(array) });
            return Err(native_error("store RunEx argument", None, error));
        }
    }
    Ok(VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VARENUM(VT_ARRAY.0 | VT_BSTR.0),
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { parray: array },
            }),
        },
    })
}

fn map_state(state: windows::Win32::System::TaskScheduler::TASK_STATE) -> TaskState {
    match state {
        value if value == TASK_STATE_DISABLED => TaskState::Disabled,
        value if value == TASK_STATE_QUEUED => TaskState::Queued,
        value if value == TASK_STATE_READY => TaskState::Ready,
        value if value == TASK_STATE_RUNNING => TaskState::Running,
        _ => TaskState::Unknown,
    }
}

fn running_instance_id(running: &IRunningTask) -> Result<Uuid> {
    let value = bstr_string(unsafe { running.InstanceGuid() }, "read instance GUID")?;
    Uuid::parse_str(value.trim_matches(['{', '}']))
        .map_err(|error| Error::new(ErrorKind::Com, format!("invalid instance GUID: {error}")))
}

fn automation_time(value: f64) -> Result<Option<TaskDateTime>> {
    if value <= 0.0 {
        return Ok(None);
    }
    let mut time = SYSTEMTIME::default();
    if unsafe { VariantTimeToSystemTime(value, &raw mut time) } == 0 {
        return Err(Error::new(
            ErrorKind::Com,
            "cannot decode Task Scheduler DATE value",
        ));
    }
    TaskDateTime::parse(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    ))
    .map(Some)
    .map_err(|error| Error::new(ErrorKind::Com, error.to_string()))
}

fn nonempty_bstr(value: windows::core::Result<BSTR>, operation: &str) -> Result<Option<String>> {
    let text = bstr_string(value, operation)?;
    Ok((!text.is_empty()).then_some(text))
}

fn bstr_string(value: windows::core::Result<BSTR>, operation: &str) -> Result<String> {
    bstr_to_string(
        value.map_err(|error| native_error(operation, None, error))?,
        operation,
    )
}

fn bstr_to_string(value: BSTR, operation: &str) -> Result<String> {
    String::try_from(value).map_err(|error| {
        Error::new(
            ErrorKind::Com,
            format!("{operation}: invalid UTF-16: {error}"),
        )
    })
}

fn native_error(operation: &str, target: Option<&str>, error: WindowsError) -> Error {
    let code = error.code().0;
    let kind = native_error_kind(u32::from_ne_bytes(code.to_ne_bytes()));
    let mut result = Error::new(kind, error.message())
        .with_operation(operation)
        .with_native_code(code);
    if let Some(target) = target {
        result = result.with_target(target);
    }
    result
}

const fn native_error_kind(code: u32) -> ErrorKind {
    match code {
        0x8007_0005 => ErrorKind::AccessDenied,
        0x8007_0002 | 0x8007_0003 | 0x8004_1309 | 0x8004_130D => ErrorKind::NotFound,
        0x8007_00B7 => ErrorKind::AlreadyExists,
        0x8007_0056 | 0x8007_052E | 0x8004_130F | 0x8004_1310 | 0x8004_1311 | 0x8004_1312
        | 0x8004_1320 => ErrorKind::Authentication,
        0x8007_06BA | 0x8007_06BE | 0x8007_06BF | 0x8004_130C | 0x8004_1315 | 0x8004_1322
        | 0x8004_1323 => ErrorKind::SchedulerUnavailable,
        0x8004_130E | 0x8004_1316 | 0x8004_1317 | 0x8004_1318 | 0x8004_1319 | 0x8004_131A
        | 0x8004_131D | 0x8004_131E | 0x8004_1321 => ErrorKind::InvalidDefinition,
        0x8004_1313 | 0x8004_1314 | 0x8004_1327 | 0x8004_1329 | 0x8004_1330 => {
            ErrorKind::Capability
        }
        0x8004_131F | 0x8004_1326 | 0x8004_1328 => ErrorKind::Conflict,
        _ => ErrorKind::Com,
    }
}

#[cfg(test)]
mod tests {
    use super::native_error_kind;
    use crate::ErrorKind;

    #[test]
    fn classifies_scheduler_specific_hresults() {
        assert_eq!(native_error_kind(0x8004_130F), ErrorKind::Authentication);
        assert_eq!(native_error_kind(0x8004_131A), ErrorKind::InvalidDefinition);
        assert_eq!(
            native_error_kind(0x8004_1322),
            ErrorKind::SchedulerUnavailable
        );
    }
}

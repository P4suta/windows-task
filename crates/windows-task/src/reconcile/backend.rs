//! The production reconciliation algorithm is shared with deterministic fault tests.

use crate::{
    FolderPath, Result, SecurityDescriptor, TaskPath,
    client::{
        BlockingScheduler, ListOptions, RegisteredTask, RegistrationOptions, SecurityInformation,
        TaskFolder,
    },
    model::{LogonType, TaskDefinition},
    xml::RawTaskXml,
};

pub(super) struct Native<'a>(pub(super) &'a BlockingScheduler);

pub(super) struct Recorded<'a, B> {
    pub(super) inner: &'a B,
    pub(super) change: &'a super::Change,
    pub(super) phase: super::ApplyPhase,
    pub(super) journal: &'a std::cell::RefCell<Vec<super::JournalEntry>>,
}

macro_rules! native_operation {
    (register, $scheduler:expr, $path:expr, $definition:expr, $options:expr) => {
        BlockingScheduler::register_commit($scheduler, $path, &$definition, $options)
    };
    (register_raw, $scheduler:expr, $path:expr, $xml:expr, $logon:expr, $options:expr) => {
        BlockingScheduler::register_raw_commit($scheduler, $path, $xml, $logon, $options)
    };
    ($name:ident, $scheduler:expr $(, $argument:expr)*) => {
        BlockingScheduler::$name($scheduler, $($argument),*)
    };
}

macro_rules! operations {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)*) => {
        pub(super) trait Backend {
            $(fn $name(&self, $($arg: $ty),*) -> Result<$out>;)*
        }
        impl Backend for Native<'_> {
            $(fn $name(&self, $($arg: $ty),*) -> Result<$out> {
                native_operation!($name, self.0, $($arg),*)
            })*
        }
        impl<B: Backend> Backend for Recorded<'_, B> {
            $(fn $name(&self, $($arg: $ty),*) -> Result<$out> {
                let mut entry = super::JournalEntry {
                    change: self.change.clone(), phase: self.phase,
                    operation: stringify!($name).into(),
                    outcome: super::StepOutcome::Attempted, error: None,
                };
                self.journal.borrow_mut().push(entry.clone());
                let result = self.inner.$name($($arg),*);
                entry.outcome = if result.is_ok() { super::StepOutcome::Succeeded } else { super::StepOutcome::Unknown };
                entry.error = result.as_ref().err().cloned();
                self.journal.borrow_mut().push(entry);
                result
            })*
        }
    };
}

operations! {
    fn get_task(path: &TaskPath) -> RegisteredTask;
    fn list_tasks(path: &FolderPath, options: ListOptions) -> Vec<RegisteredTask>;
    fn list_folders(path: &FolderPath, recursive: bool) -> Vec<TaskFolder>;
    fn task_security(path: &TaskPath, information: SecurityInformation) -> SecurityDescriptor;
    fn folder_security(path: &FolderPath, information: SecurityInformation) -> SecurityDescriptor;
    fn register(path: &TaskPath, definition: TaskDefinition, options: RegistrationOptions) -> ();
    fn register_raw(path: &TaskPath, xml: RawTaskXml, logon_type: LogonType, options: RegistrationOptions) -> ();
    fn create_folder(path: &FolderPath, security: Option<SecurityDescriptor>) -> TaskFolder;
    fn delete_folder(path: &FolderPath) -> ();
    fn delete_task(path: &TaskPath) -> ();
    fn set_enabled(path: &TaskPath, enabled: bool) -> ();
    fn set_task_security(path: &TaskPath, descriptor: SecurityDescriptor, information: SecurityInformation) -> ();
    fn set_folder_security(path: &FolderPath, descriptor: SecurityDescriptor, information: SecurityInformation) -> ();
    fn stop_all(path: &TaskPath) -> ();
}

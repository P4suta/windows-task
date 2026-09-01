#![cfg(windows)]

use windows_task::{
    Error, ErrorKind, FolderPath,
    client::{ListOptions, RegistrationMode, RegistrationOptions, Scheduler},
    model::{Action, ExecAction, LogonType, PrincipalIdentity, ServiceAccount, TaskDefinition},
};

#[test]
fn local_scheduler_can_be_inspected() -> windows_task::Result<()> {
    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let capabilities = blocking.capabilities()?;

    assert!(
        capabilities.highest_version >= 0x0001_0002,
        "Task Scheduler 2.0 or newer is required"
    );
    drop(blocking.list_folders(&FolderPath::root(), false)?);
    drop(blocking.list_tasks(&FolderPath::root(), ListOptions::default())?);
    Ok(())
}

#[test]
fn isolated_task_round_trip() -> windows_task::Result<()> {
    if std::env::var_os("WINDOWS_TASK_MUTATION_TESTS").as_deref() != Some(std::ffi::OsStr::new("1"))
    {
        return Ok(());
    }

    let scheduler = Scheduler::builder().local().connect_blocking()?;
    let blocking = scheduler.blocking();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let folder = FolderPath::root()
        .join(&format!("windows-task-ci-{suffix}"))
        .expect("generated folder name is valid");
    let task = folder
        .task("disabled-smoke")
        .expect("static task name is valid");

    let outcome = (|| {
        blocking.create_folder(&folder, None)?;

        let mut definition = TaskDefinition::new(Action::Exec(
            ExecAction::new("cmd.exe").args(["/d", "/c", "exit", "0"]),
        ));
        definition.principal.identity =
            PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem);
        definition.principal.logon_type = LogonType::ServiceAccount;
        definition.settings.enabled = false;

        let registered = blocking.register(
            &task,
            definition,
            RegistrationOptions {
                mode: RegistrationMode::Create,
                disabled: true,
                ..RegistrationOptions::default()
            },
        )?;
        if registered.path != task || registered.enabled {
            return Err(Error::new(
                ErrorKind::Other,
                "registered smoke task did not round-trip as the requested disabled task",
            ));
        }
        let read_back = blocking.get_task(&task)?;
        if read_back.path != task {
            return Err(Error::new(
                ErrorKind::Other,
                "read-back task path differs from the registered path",
            ));
        }
        Ok(())
    })();

    // Cleanup is attempted even when registration or read-back failed. A unique
    // folder keeps the test from touching any pre-existing scheduler state.
    let task_cleanup = blocking.delete_task(&task);
    let folder_cleanup = blocking.delete_folder(&folder);
    outcome?;
    task_cleanup?;
    folder_cleanup
}

//! Creates a small desired-state TOML manifest.

use std::str::FromStr as _;

use uuid::Uuid;
use windows_task::{
    FolderPath, TaskPath,
    manifest::{DocumentFormat, ManagedTask, TaskManifest},
    model::{Action, ExecAction, TaskDefinition},
};

fn main() -> windows_task::Result<()> {
    let namespace = FolderPath::from_str("\\Acme").map_err(|error| {
        windows_task::Error::new(windows_task::ErrorKind::InvalidPath, error.to_string())
    })?;
    let mut manifest = TaskManifest::new(Uuid::new_v4(), "Acme Agent", namespace);
    let path = TaskPath::from_str("\\Acme\\Heartbeat").map_err(|error| {
        windows_task::Error::new(windows_task::ErrorKind::InvalidPath, error.to_string())
    })?;
    manifest.tasks.push(ManagedTask {
        path,
        definition: TaskDefinition::new(Action::Exec(ExecAction::new("acme-agent.exe"))),
        credentials: Default::default(),
    });
    println!("{}", manifest.to_string(DocumentFormat::Toml)?);
    Ok(())
}

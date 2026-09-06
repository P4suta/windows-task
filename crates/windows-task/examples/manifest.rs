//! Writes the fully explicit desired-state manifest checked in beside this
//! file as `desired-state.toml`. Run it after changing the model to regenerate
//! that document:
//!
//! ```text
//! cargo run --example manifest --features serde > crates/windows-task/examples/desired-state.toml
//! ```

use uuid::Uuid;
use windows_task::{
    FolderPath,
    manifest::{DocumentFormat, ManagedTask, TaskManifest},
    model::{DailyTrigger, ExecAction, TaskDateTime, TaskDefinition},
};

// The owner UUID is written into the registration Source marker of every
// managed task, so it must stay stable for the lifetime of the manifest.
// Generate one once with `Uuid::new_v4()` and paste it in; generating it at run
// time would make reconciliation lose track of the tasks it already owns.
const OWNER: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0000);

fn main() -> windows_task::Result<()> {
    let namespace: FolderPath = "\\Acme".parse()?;
    let mut manifest = TaskManifest::new(OWNER, "Acme Agent", namespace);

    manifest.tasks.push(ManagedTask {
        path: "\\Acme\\Heartbeat".parse()?,
        definition: TaskDefinition::builder(
            ExecAction::new("acme-agent.exe").args(["--heartbeat"]),
        )
        .description("Reports agent liveness once a day.")
        .trigger(DailyTrigger::new(TaskDateTime::wall_clock(
            2026, 9, 5, 6, 0, 0,
        )?))
        .build()?,
        credentials: Default::default(),
    });

    print!("{}", manifest.to_string(DocumentFormat::Toml)?);
    Ok(())
}

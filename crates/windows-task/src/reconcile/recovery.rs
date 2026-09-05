//! Observe both sides of each mutation and refuse to overwrite unrelated drift.

use super::{
    ApplyOptions, ApplyPhase, ApplyReport, Backend, Change, Error, ErrorKind, JournalEntry,
    PreparedApply, Result, RollbackFailure, SecurityDescriptor, SecurityInformation, StepOutcome,
    TaskDefinition, TaskManifest, TaskPath, compensate_change, definitions_semantically_equal,
    managed_task, owned_definition,
};

#[derive(Clone, Debug)]
pub(super) enum Observed {
    Missing,
    Task {
        definition: Box<TaskDefinition>,
        enabled: bool,
        security: Option<SecurityDescriptor>,
    },
    Folder(Option<SecurityDescriptor>),
}

impl PartialEq for Observed {
    fn eq(&self, other: &Self) -> bool {
        let security_equal = |left: &Option<SecurityDescriptor>,
                              right: &Option<SecurityDescriptor>| {
            match (left, right) {
                (Some(left), Some(right)) => left.access_equivalent(right),
                (None, None) => true,
                _ => false,
            }
        };
        match (self, other) {
            (Self::Missing, Self::Missing) => true,
            (Self::Folder(left), Self::Folder(right)) => security_equal(left, right),
            (
                Self::Task {
                    definition: left,
                    enabled: left_enabled,
                    security: left_security,
                },
                Self::Task {
                    definition: right,
                    enabled: right_enabled,
                    security: right_security,
                },
            ) => {
                left == right
                    && left_enabled == right_enabled
                    && security_equal(left_security, right_security)
            }
            _ => false,
        }
    }
}
impl Eq for Observed {}

pub(super) fn task_path(change: &Change) -> Option<&TaskPath> {
    match change {
        Change::CreateTask(path)
        | Change::UpdateTask(path)
        | Change::AdoptTask(path)
        | Change::DeleteTask(path)
        | Change::SetEnabled { path, .. }
        | Change::SetTaskSecurity(path) => Some(path),
        Change::CreateFolder(_) | Change::SetFolderSecurity(_) => None,
    }
}

pub(super) fn same_target(left: &Change, right: &Change) -> bool {
    match (task_path(left), task_path(right)) {
        (Some(left), Some(right)) => left == right,
        (None, None) => match (left, right) {
            (
                Change::CreateFolder(left) | Change::SetFolderSecurity(left),
                Change::CreateFolder(right) | Change::SetFolderSecurity(right),
            ) => left == right,
            _ => false,
        },
        _ => false,
    }
}

pub(super) fn observe(
    backend: &impl Backend,
    manifest: &TaskManifest,
    change: &Change,
    allow_irreversible: bool,
) -> Result<Observed> {
    let information = information(manifest, change);
    if let Some(path) = task_path(change) {
        let task = match backend.get_task(path) {
            Ok(task) => task,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Observed::Missing),
            Err(error) => return Err(error),
        };
        let mut definition = task.snapshot.definition()?.clone();
        definition.registration.date = None;
        definition.registration.security_descriptor = None;
        let security = match backend.task_security(path, information) {
            Ok(security) => Some(security),
            Err(_) if allow_irreversible => None,
            Err(error) => return Err(error),
        };
        Ok(Observed::Task {
            definition: Box::new(definition),
            enabled: task.enabled,
            security,
        })
    } else {
        let (Change::CreateFolder(path) | Change::SetFolderSecurity(path)) = change else {
            unreachable!()
        };
        match backend.folder_security(path, information) {
            Ok(security) => Ok(Observed::Folder(Some(security))),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Observed::Missing),
            Err(_) if allow_irreversible => {
                backend.list_folders(path, false)?;
                Ok(Observed::Folder(None))
            }
            Err(error) => Err(error),
        }
    }
}

pub(super) fn expected_after(
    backend: &impl Backend,
    before: &Observed,
    after: &Observed,
    manifest: &TaskManifest,
    change: &Change,
) -> bool {
    match (change, after) {
        (
            Change::CreateTask(path) | Change::UpdateTask(path) | Change::AdoptTask(path),
            Observed::Task {
                definition,
                enabled,
                ..
            },
        ) => managed_task(manifest, path).is_ok_and(|task| {
            let expected = owned_definition(manifest, task);
            definitions_semantically_equal(definition, &expected)
                && *enabled == expected.settings.enabled
                && expected
                    .registration
                    .security_descriptor
                    .as_ref()
                    .is_none_or(|desired| {
                        backend
                            .task_security(path, super::security_information_for_sddl(desired))
                            .is_ok_and(|actual| actual.access_equivalent(desired))
                    })
        }),
        (Change::DeleteTask(_), Observed::Missing)
        | (Change::CreateFolder(_), Observed::Folder(_)) => true,
        (Change::SetFolderSecurity(path), Observed::Folder(_)) => manifest
            .folders
            .iter()
            .find(|folder| folder.path == *path)
            .and_then(|folder| folder.security_descriptor.as_ref())
            .is_some_and(|desired| {
                backend
                    .folder_security(path, super::security_information_for_sddl(desired))
                    .is_ok_and(|actual| actual.access_equivalent(desired))
            }),
        (
            Change::SetEnabled { enabled, .. },
            Observed::Task {
                enabled: actual,
                definition,
                security,
            },
        ) => {
            matches!(before, Observed::Task { definition: old, security: old_security, .. }
                if definitions_semantically_equal(old, definition) && security == old_security && enabled == actual)
        }
        (
            Change::SetTaskSecurity(path),
            Observed::Task {
                definition,
                enabled,
                ..
            },
        ) => {
            // Verify the requested sections by read-back. A successful native
            // setter alone cannot prove that another writer did not change them.
            matches!(before, Observed::Task { definition: old, enabled: old_enabled, .. }
                if definitions_semantically_equal(old, definition) && enabled == old_enabled)
                && managed_task(manifest, path).is_ok_and(|task| {
                    task.definition
                        .registration
                        .security_descriptor
                        .as_ref()
                        .is_some_and(|desired| {
                            backend
                                .task_security(path, super::security_information_for_sddl(desired))
                                .is_ok_and(|actual| actual.access_equivalent(desired))
                        })
                })
        }
        _ => false,
    }
}

pub(super) fn record(
    report: &mut ApplyReport,
    change: &Change,
    phase: ApplyPhase,
    outcome: StepOutcome,
    error: Option<Error>,
) {
    report.journal.push(JournalEntry {
        operation: format!("{phase:?}"),
        change: change.clone(),
        phase,
        outcome,
        error,
    });
}

pub(super) struct Undo {
    pub(super) change: Change,
    pub(super) before: Observed,
    pub(super) after: Observed,
}

pub(super) fn compensate_observed(
    backend: &impl Backend,
    manifest: &TaskManifest,
    options: ApplyOptions,
    prepared: &mut PreparedApply,
    report: &mut ApplyReport,
    undo: Vec<Undo>,
) {
    for entry in undo.into_iter().rev() {
        record(
            report,
            &entry.change,
            ApplyPhase::Rollback,
            StepOutcome::Attempted,
            None,
        );
        let result = crate::observe::Operation::new("rollback").run(|| {
            if observe(backend, manifest, &entry.change, options.allow_irreversible)? != entry.after
            {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "state changed after apply; refusing to overwrite external changes",
                ));
            }
            let native_journal = std::cell::RefCell::new(Vec::new());
            let recorded = super::backend::Recorded {
                inner: backend,
                change: &entry.change,
                phase: ApplyPhase::Rollback,
                journal: &native_journal,
            };
            let result = compensate_change(&recorded, &entry.change, options, prepared);
            report.journal.extend(native_journal.into_inner());
            result?;
            if observe(backend, manifest, &entry.change, options.allow_irreversible)?
                != entry.before
            {
                return Err(Error::new(
                    ErrorKind::Irreversible,
                    "rollback returned but original state is not confirmed",
                ));
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                record(
                    report,
                    &entry.change,
                    ApplyPhase::Rollback,
                    StepOutcome::Succeeded,
                    None,
                );
                report.rolled_back.push(entry.change);
            }
            Err(error) => {
                record(
                    report,
                    &entry.change,
                    ApplyPhase::Rollback,
                    StepOutcome::Unknown,
                    Some(error.clone()),
                );
                report.unresolved.push(entry.change.clone());
                report.rollback_failures.push(RollbackFailure {
                    change: entry.change,
                    phase: ApplyPhase::Rollback,
                    error,
                });
            }
        }
    }
}

pub(super) fn information(manifest: &TaskManifest, change: &Change) -> SecurityInformation {
    let descriptor = if let Some(path) = task_path(change) {
        manifest
            .tasks
            .iter()
            .find(|task| task.path == *path)
            .and_then(|task| task.definition.registration.security_descriptor.as_ref())
    } else {
        let (Change::CreateFolder(path) | Change::SetFolderSecurity(path)) = change else {
            unreachable!()
        };
        manifest
            .folders
            .iter()
            .find(|folder| folder.path == *path)
            .and_then(|folder| folder.security_descriptor.as_ref())
    };
    let base = SecurityInformation::OWNER | SecurityInformation::GROUP | SecurityInformation::DACL;
    descriptor.map_or(base, |descriptor| {
        base | super::security_information_for_sddl(descriptor)
    })
}

pub(super) fn agrees_with_inspection(
    state: &super::CurrentState,
    change: &Change,
    observed: &Observed,
) -> bool {
    if let Some(path) = task_path(change) {
        match (state.tasks.iter().find(|task| task.path == *path), observed) {
            (None, Observed::Missing) => true,
            (
                Some(old),
                Observed::Task {
                    definition,
                    enabled,
                    ..
                },
            ) => {
                definitions_semantically_equal(&old.definition, definition)
                    && old.enabled == *enabled
            }
            _ => false,
        }
    } else {
        let (Change::CreateFolder(path) | Change::SetFolderSecurity(path)) = change else {
            unreachable!()
        };
        state.folders.contains_key(path) == matches!(observed, Observed::Folder(_))
    }
}

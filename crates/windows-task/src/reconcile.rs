//! Ownership-safe desired-state planning, application, and compensation.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

mod backend;
#[cfg(test)]
mod fault_tests;
mod recovery;
use backend::Backend;
use recovery::{Observed, Undo, compensate_observed, observe, record};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    Error, ErrorKind, FolderPath, Password, Result, TaskPath,
    client::{
        BlockingScheduler, ListOptions, RegistrationMode, RegistrationOptions, SecurityInformation,
    },
    manifest::{ManagedTask, TaskManifest},
    model::{LogonType, SecurityDescriptor, TaskDefinition},
    xml::RawTaskXml,
};

/// One semantic desired-state change.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Change {
    /// Create a scheduler folder.
    CreateFolder(FolderPath),
    /// Create a new task.
    CreateTask(TaskPath),
    /// Replace a managed task whose semantic definition drifted.
    UpdateTask(TaskPath),
    /// Adopt an existing unowned task after explicit authorization.
    AdoptTask(TaskPath),
    /// Delete a managed task absent from desired state.
    DeleteTask(TaskPath),
    /// Change enabled state only.
    SetEnabled {
        /// Registered task path.
        path: TaskPath,
        /// Desired state.
        enabled: bool,
    },
    /// Change task security without replacing the definition.
    SetTaskSecurity(TaskPath),
    /// Change folder security without replacing its children.
    SetFolderSecurity(FolderPath),
}

/// Rollback guarantees for a planned change.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum RollbackSafety {
    /// The old state is fully reconstructable.
    Reversible,
    /// Rollback requires a separately supplied registration password.
    RequiresCredential,
    /// Native side effects, such as a triggered run, cannot be undone.
    Irreversible,
}

/// Change plus its rollback classification.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct PlannedChange {
    /// Intended operation.
    pub change: Change,
    /// Available compensation guarantee.
    pub rollback: RollbackSafety,
}

/// Deterministically ordered dry-run plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct Plan {
    /// Desired changes in apply order.
    pub changes: Vec<PlannedChange>,
    /// Planned instance-stop requests cannot be undone by restoring definitions.
    #[serde(default)]
    pub irreversible_effects: Vec<TaskPath>,
}

impl Plan {
    /// Includes the irreversible stop requests that `ApplyOptions::stop_running`
    /// would issue for this plan, without making any scheduler calls.
    #[must_use]
    pub fn with_stop_running(mut self) -> Self {
        self.irreversible_effects = self
            .changes
            .iter()
            .filter_map(|planned| match &planned.change {
                Change::UpdateTask(path) | Change::AdoptTask(path) | Change::DeleteTask(path) => {
                    Some(path.clone())
                }
                _ => None,
            })
            .collect();
        self
    }
    /// Whether current state already matches desired state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Whether any change needs extra input or cannot be compensated.
    #[must_use]
    pub fn contains_irreversible_changes(&self) -> bool {
        !self.irreversible_effects.is_empty()
            || self
                .changes
                .iter()
                .any(|change| change.rollback != RollbackSafety::Reversible)
    }
}

/// State required to build a plan without mutating Task Scheduler.
#[derive(Clone, Debug)]
pub struct CurrentTask {
    /// Path.
    pub path: TaskPath,
    /// Parsed task definition.
    pub definition: TaskDefinition,
    /// Effective enabled state.
    pub enabled: bool,
    /// Whether the Source marker or a preserved legacy URI matches this manifest.
    pub owned: bool,
    /// Whether rollback needs a password.
    pub password_backed: bool,
}

/// Read-only scheduler snapshot used by planning.
#[derive(Clone, Debug, Default)]
pub struct CurrentState {
    /// Tasks below the manifest namespace.
    pub tasks: Vec<CurrentTask>,
    /// Existing folders and their SDDL when it was requested.
    pub folders: BTreeMap<FolderPath, Option<SecurityDescriptor>>,
}

/// Options that affect planning only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlanOptions {
    /// Delete owned tasks absent from desired state.
    pub prune: bool,
    /// Permit replacing an existing task without this manifest's ownership marker.
    pub adopt: bool,
}

/// Inspects the complete managed namespace without changing it.
pub fn inspect(scheduler: &BlockingScheduler, manifest: &TaskManifest) -> Result<CurrentState> {
    inspect_backend(&backend::Native(scheduler), manifest)
}

/// Plans against the connected scheduler after resolving native principal and
/// SDDL representations. Does not register tasks or change permissions.
pub fn plan_live(
    scheduler: &BlockingScheduler,
    manifest: &TaskManifest,
    options: PlanOptions,
) -> Result<Plan> {
    let manifest = normalize_manifest(scheduler, manifest)?;
    plan_state(&manifest, &inspect(scheduler, &manifest)?, options)
}

fn normalize_manifest(
    scheduler: &BlockingScheduler,
    manifest: &TaskManifest,
) -> Result<TaskManifest> {
    ensure_valid_manifest(manifest)?;
    let mut manifest = manifest.clone();
    for task in &mut manifest.tasks {
        task.definition = scheduler.normalize_definition(task.definition.clone())?;
        if let Some(descriptor) = task.definition.registration.security_descriptor.take() {
            let information = security_information_for_sddl(&descriptor);
            task.definition.registration.security_descriptor =
                Some(scheduler.normalize_security(descriptor, information)?);
        }
    }
    for folder in &mut manifest.folders {
        if let Some(descriptor) = folder.security_descriptor.take() {
            let information = security_information_for_sddl(&descriptor);
            folder.security_descriptor =
                Some(scheduler.normalize_security(descriptor, information)?);
        }
    }
    Ok(manifest)
}

fn inspect_backend(scheduler: &impl Backend, manifest: &TaskManifest) -> Result<CurrentState> {
    ensure_valid_manifest(manifest)?;
    let list_options = ListOptions {
        recursive: true,
        include_hidden: true,
    };
    let registered = match scheduler.list_tasks(&manifest.namespace, list_options) {
        Ok(tasks) => tasks,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(CurrentState::default()),
        Err(error) => return Err(error),
    };
    let mut folders = BTreeMap::new();
    folders.insert(manifest.namespace.clone(), None);
    for folder in scheduler.list_folders(&manifest.namespace, true)? {
        folders.insert(folder.path, folder.security_descriptor);
    }
    for desired in &manifest.folders {
        if folders.contains_key(&desired.path) {
            if let Some(expected) = &desired.security_descriptor {
                let descriptor = scheduler
                    .folder_security(&desired.path, security_information_for_sddl(expected))?;
                folders.insert(desired.path.clone(), Some(descriptor));
            }
        }
    }

    let mut tasks = Vec::with_capacity(registered.len());
    for task in registered {
        let mut definition = task.snapshot.definition()?.clone();
        if let Some(expected_security) = manifest
            .tasks
            .iter()
            .find(|desired| desired.path == task.path)
            .and_then(|desired| desired.definition.registration.security_descriptor.as_ref())
        {
            definition.registration.security_descriptor = Some(
                scheduler
                    .task_security(&task.path, security_information_for_sddl(expected_security))?,
            );
        }
        let expected_uri = manifest.ownership_uri(&task.path);
        let owned = definition.registration.uri.as_deref() == Some(expected_uri.as_str())
            || definition
                .registration
                .source
                .as_deref()
                .is_some_and(|source| {
                    source == expected_uri
                        || source
                            .strip_prefix(&expected_uri)
                            .is_some_and(|suffix| suffix.starts_with('\n'))
                });
        let password_backed = password_backed(definition.principal.logon_type);
        tasks.push(CurrentTask {
            path: task.path,
            definition,
            enabled: task.enabled,
            owned,
            password_backed,
        });
    }
    tasks.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(CurrentState { tasks, folders })
}

/// Plans against a caller-provided task snapshot. Folders are conservatively
/// treated as missing; use [`plan_state`] after [`inspect`] for a live dry run.
pub fn plan(manifest: &TaskManifest, current: &[CurrentTask], prune: bool) -> Result<Plan> {
    plan_state(
        manifest,
        &CurrentState {
            tasks: current.to_vec(),
            folders: BTreeMap::new(),
        },
        PlanOptions {
            prune,
            adopt: false,
        },
    )
}

/// Builds a deterministic semantic plan from a complete current snapshot.
pub fn plan_state(
    manifest: &TaskManifest,
    current: &CurrentState,
    options: PlanOptions,
) -> Result<Plan> {
    ensure_valid_manifest(manifest)?;
    let mut changes = Vec::new();
    for folder in required_folders(manifest) {
        if !current.folders.contains_key(&folder) {
            changes.push(PlannedChange {
                change: Change::CreateFolder(folder),
                rollback: RollbackSafety::Reversible,
            });
        }
    }

    let current_by_path: BTreeMap<_, _> = current
        .tasks
        .iter()
        .map(|task| (&task.path, task))
        .collect();
    for desired in &manifest.tasks {
        let desired_definition = owned_definition(manifest, desired);
        match current_by_path.get(&desired.path) {
            None => changes.push(PlannedChange {
                change: Change::CreateTask(desired.path.clone()),
                rollback: RollbackSafety::Reversible,
            }),
            Some(existing) if !existing.owned && !options.adopt => {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "an existing task is not owned by this manifest",
                )
                .with_target(desired.path.to_string()));
            }
            Some(existing) if !existing.owned => changes.push(PlannedChange {
                change: Change::AdoptTask(desired.path.clone()),
                rollback: rollback_safety(existing),
            }),
            Some(existing) => {
                if !definitions_semantically_equal(&existing.definition, &desired_definition) {
                    changes.push(PlannedChange {
                        change: Change::UpdateTask(desired.path.clone()),
                        rollback: rollback_safety(existing),
                    });
                    continue;
                }
                if existing.enabled != desired_definition.settings.enabled {
                    changes.push(PlannedChange {
                        change: Change::SetEnabled {
                            path: desired.path.clone(),
                            enabled: desired_definition.settings.enabled,
                        },
                        rollback: RollbackSafety::Reversible,
                    });
                }
                if let Some(desired_security) =
                    desired_definition.registration.security_descriptor.as_ref()
                {
                    if existing
                        .definition
                        .registration
                        .security_descriptor
                        .as_ref()
                        .is_none_or(|actual| !actual.access_equivalent(desired_security))
                    {
                        changes.push(PlannedChange {
                            change: Change::SetTaskSecurity(desired.path.clone()),
                            rollback: RollbackSafety::Reversible,
                        });
                    }
                }
            }
        }
    }

    for desired in &manifest.folders {
        if let Some(descriptor) = &desired.security_descriptor {
            let current_descriptor = current.folders.get(&desired.path).and_then(Option::as_ref);
            if current_descriptor.is_none_or(|actual| !actual.access_equivalent(descriptor)) {
                changes.push(PlannedChange {
                    change: Change::SetFolderSecurity(desired.path.clone()),
                    rollback: RollbackSafety::Reversible,
                });
            }
        }
    }

    if options.prune {
        let desired: BTreeSet<_> = manifest.tasks.iter().map(|task| &task.path).collect();
        for existing in current
            .tasks
            .iter()
            .filter(|task| task.owned && !desired.contains(&task.path))
        {
            changes.push(PlannedChange {
                change: Change::DeleteTask(existing.path.clone()),
                rollback: rollback_safety(existing),
            });
        }
    }
    changes.sort_by(|left, right| change_order(&left.change).cmp(&change_order(&right.change)));
    Ok(Plan {
        changes,
        irreversible_effects: Vec::new(),
    })
}

/// Why a password is being requested during apply preflight.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CredentialPurpose {
    /// Register the desired task definition.
    DesiredRegistration,
    /// Restore the previous task definition during compensation.
    Rollback,
}

/// Resolves registration passwords from a secret store. Implementations must
/// never log or serialize the returned value.
pub trait CredentialResolver {
    /// Resolves one password. `reference` is the non-secret manifest value
    /// when one is available.
    fn registration_password(
        &mut self,
        path: &TaskPath,
        reference: Option<&str>,
        purpose: CredentialPurpose,
    ) -> Result<Password>;
}

impl<F> CredentialResolver for F
where
    F: FnMut(&TaskPath, Option<&str>, CredentialPurpose) -> Result<Password>,
{
    fn registration_password(
        &mut self,
        path: &TaskPath,
        reference: Option<&str>,
        purpose: CredentialPurpose,
    ) -> Result<Password> {
        self(path, reference, purpose)
    }
}

/// Resolver used by [`apply`] when no secret provider is supplied.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCredentials;

impl CredentialResolver for NoCredentials {
    fn registration_password(
        &mut self,
        path: &TaskPath,
        _reference: Option<&str>,
        purpose: CredentialPurpose,
    ) -> Result<Password> {
        let kind = match purpose {
            CredentialPurpose::DesiredRegistration => ErrorKind::Authentication,
            CredentialPurpose::Rollback => ErrorKind::Irreversible,
        };
        Err(
            Error::new(kind, "a registration password resolver is required")
                .with_target(path.to_string()),
        )
    }
}

/// Mutation and safety policy for apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplyOptions {
    /// Delete owned tasks absent from the manifest.
    pub prune: bool,
    /// Explicitly adopt unowned tasks that collide with desired paths.
    pub adopt: bool,
    /// Continue when an exact rollback snapshot cannot be prepared.
    pub allow_irreversible: bool,
    /// Stop running instances before replacing or deleting their task.
    pub stop_running: bool,
    /// Suppress registration triggers during create, update, and rollback.
    pub ignore_registration_triggers: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            prune: false,
            adopt: false,
            allow_irreversible: false,
            stop_running: false,
            ignore_registration_triggers: true,
        }
    }
}

/// Successful apply or the mutation/compensation portion of a failed apply.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ApplyReport {
    /// Whether apply completed successfully, independently of the number of changes.
    pub status: ApplyStatus,
    /// Original deterministic plan.
    pub plan: Plan,
    /// Changes successfully attempted before completion or failure.
    pub applied: Vec<Change>,
    /// Applied changes successfully compensated after a failure.
    pub rolled_back: Vec<Change>,
    /// Compensation errors; non-empty means the final state is uncertain.
    pub rollback_failures: Vec<RollbackFailure>,
    /// Ordered observations, including uncertain and partially completed operations.
    pub journal: Vec<JournalEntry>,
    /// Changes whose final state could not be safely established or restored.
    pub unresolved: Vec<Change>,
    /// Stop requests cannot be undone by restoring a task definition.
    pub irreversible_effects: Vec<TaskPath>,
}

/// Overall apply outcome, including failures before a plan can be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    /// Apply failed; inspect the failure cause and recovery evidence.
    Failed,
    /// Apply completed, including a successful operation with no changes.
    Succeeded,
}

/// One phase of an apply or compensation operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPhase {
    /// Compare live state immediately before mutation.
    Precondition,
    /// Stop existing instances (irreversible).
    Stop,
    /// Mutate the task or folder.
    Mutation,
    /// Read back state after an attempt, including a lost response.
    Verify,
    /// Compensate a previously attempted mutation.
    Rollback,
}

/// What has been established for a journaled operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// The operation is about to be invoked.
    Attempted,
    /// The operation returned successfully.
    Succeeded,
    /// The operation failed without a confirmed final state.
    Failed,
    /// A mutation may have happened but cannot safely be attributed or restored.
    Unknown,
}

/// Structured apply evidence. Errors retain native codes and diagnostic fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Specific backend operation, or the outer observation phase.
    pub operation: String,
    /// Semantic change being processed.
    pub change: Change,
    /// Execution phase.
    pub phase: ApplyPhase,
    /// Observed outcome.
    pub outcome: StepOutcome,
    /// Original error, when applicable.
    pub error: Option<Error>,
}

/// A compensation failure that can be inspected without parsing prose.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RollbackFailure {
    /// Change requiring recovery.
    pub change: Change,
    /// Phase where recovery failed.
    pub phase: ApplyPhase,
    /// Original classified error.
    pub error: Error,
}

impl ApplyReport {
    /// Whether every planned change was applied and no rollback was needed.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == ApplyStatus::Succeeded
            && self.applied.len() == self.plan.changes.len()
            && self.rolled_back.is_empty()
            && self.rollback_failures.is_empty()
            && self.unresolved.is_empty()
    }

    /// Whether compensation restored every already-applied change.
    #[must_use]
    pub fn rollback_complete(&self) -> bool {
        !self.applied.is_empty()
            && self.rolled_back.len() == self.applied.len()
            && self.rollback_failures.is_empty()
            && self.unresolved.is_empty()
            && self.irreversible_effects.is_empty()
    }
}

/// Apply failure retaining the original cause and compensation report.
#[derive(Clone, Debug)]
pub struct ApplyFailure {
    /// Native, validation, ownership, or credential error that stopped apply.
    pub cause: Error,
    /// Steps applied and compensated around the failure.
    pub report: ApplyReport,
}

impl fmt::Display for ApplyFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.cause)?;
        if !self.report.rollback_failures.is_empty() {
            write!(
                formatter,
                "; {} rollback step(s) also failed",
                self.report.rollback_failures.len()
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ApplyFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Applies with no credential source. Password-backed desired or rollback
/// operations fail safely during preflight.
pub fn apply(
    scheduler: &BlockingScheduler,
    manifest: &TaskManifest,
    options: ApplyOptions,
) -> std::result::Result<ApplyReport, ApplyFailure> {
    apply_with_credentials(scheduler, manifest, options, &mut NoCredentials)
}

/// Applies a manifest after preparing rollback snapshots and all required
/// credentials. On the first mutation error, completed changes are
/// compensated in reverse order.
pub fn apply_with_credentials(
    scheduler: &BlockingScheduler,
    manifest: &TaskManifest,
    options: ApplyOptions,
    resolver: &mut dyn CredentialResolver,
) -> std::result::Result<ApplyReport, ApplyFailure> {
    crate::observe::Operation::new("apply").scope(|| {
        let manifest = normalize_manifest(scheduler, manifest).map_err(empty_apply_failure)?;
        apply_backend(&backend::Native(scheduler), &manifest, options, resolver)
    })
}

fn apply_backend(
    scheduler: &impl Backend,
    manifest: &TaskManifest,
    options: ApplyOptions,
    resolver: &mut dyn CredentialResolver,
) -> std::result::Result<ApplyReport, ApplyFailure> {
    let state = inspect_backend(scheduler, manifest).map_err(empty_apply_failure)?;
    let plan = plan_state(
        manifest,
        &state,
        PlanOptions {
            prune: options.prune,
            adopt: options.adopt,
        },
    )
    .map_err(empty_apply_failure)?;
    let plan = if options.stop_running {
        plan.with_stop_running()
    } else {
        plan
    };
    let mut report = ApplyReport {
        status: ApplyStatus::Failed,
        plan: plan.clone(),
        applied: Vec::new(),
        rolled_back: Vec::new(),
        rollback_failures: Vec::new(),
        journal: Vec::new(),
        unresolved: Vec::new(),
        irreversible_effects: Vec::new(),
    };
    if plan.is_empty() {
        report.status = ApplyStatus::Succeeded;
        return Ok(report);
    }
    let mut expected = plan
        .changes
        .iter()
        .map(|planned| {
            observe(
                scheduler,
                manifest,
                &planned.change,
                options.allow_irreversible,
            )
        })
        .collect::<Result<Vec<_>>>()
        .map_err(|cause| ApplyFailure {
            cause,
            report: report.clone(),
        })?;
    for (planned, observed) in plan.changes.iter().zip(&expected) {
        if !recovery::agrees_with_inspection(&state, &planned.change, observed) {
            return Err(ApplyFailure {
                cause: Error::new(
                    ErrorKind::Conflict,
                    "state changed between planning and preflight",
                ),
                report,
            });
        }
    }
    let mut prepared = prepare(
        scheduler,
        manifest,
        &plan,
        options.allow_irreversible,
        resolver,
    )
    .map_err(|cause| ApplyFailure {
        cause,
        report: report.clone(),
    })?;

    let mut undo = Vec::new();
    for (index, planned) in plan.changes.iter().enumerate() {
        let change = &planned.change;
        let before = expected[index].clone();
        let precondition = observe(scheduler, manifest, change, options.allow_irreversible)
            .and_then(|live| {
                if live == before {
                    Ok(())
                } else {
                    Err(Error::new(
                        ErrorKind::Conflict,
                        "state changed during preflight",
                    ))
                }
            });
        record(
            &mut report,
            change,
            ApplyPhase::Precondition,
            if precondition.is_ok() {
                StepOutcome::Succeeded
            } else {
                StepOutcome::Failed
            },
            precondition.as_ref().err().cloned(),
        );
        if let Err(cause) = precondition {
            compensate_observed(
                scheduler,
                manifest,
                options,
                &mut prepared,
                &mut report,
                undo,
            );
            return Err(ApplyFailure { cause, report });
        }
        if let Change::SetFolderSecurity(path) = change {
            if let Observed::Folder(Some(descriptor)) = &before {
                prepared
                    .folder_security
                    .entry(path.clone())
                    .or_insert_with(|| SecuritySnapshot {
                        descriptor: descriptor.clone(),
                        information: recovery::information(manifest, change),
                    });
            }
        }
        if options.stop_running
            && matches!(
                change,
                Change::UpdateTask(_) | Change::AdoptTask(_) | Change::DeleteTask(_)
            )
        {
            let path = recovery::task_path(change).expect("task mutation has a path");
            report.irreversible_effects.push(path.clone());
            record(
                &mut report,
                change,
                ApplyPhase::Stop,
                StepOutcome::Attempted,
                None,
            );
            let result = scheduler.stop_all(path);
            record(
                &mut report,
                change,
                ApplyPhase::Stop,
                if result.is_ok() {
                    StepOutcome::Succeeded
                } else {
                    StepOutcome::Unknown
                },
                result.as_ref().err().cloned(),
            );
            if let Err(cause) = result {
                compensate_observed(
                    scheduler,
                    manifest,
                    options,
                    &mut prepared,
                    &mut report,
                    undo,
                );
                return Err(ApplyFailure { cause, report });
            }
        }
        record(
            &mut report,
            change,
            ApplyPhase::Mutation,
            StepOutcome::Attempted,
            None,
        );
        let native_journal = std::cell::RefCell::new(Vec::new());
        let recorded = backend::Recorded {
            inner: scheduler,
            change,
            phase: ApplyPhase::Mutation,
            journal: &native_journal,
        };
        let result = crate::observe::Operation::new("apply.change")
            .run(|| execute_change(&recorded, manifest, change, options, &mut prepared));
        report.journal.extend(native_journal.into_inner());
        record(
            &mut report,
            change,
            ApplyPhase::Mutation,
            if result.is_ok() {
                StepOutcome::Succeeded
            } else {
                StepOutcome::Unknown
            },
            result.as_ref().err().cloned(),
        );
        let observed = observe(scheduler, manifest, change, options.allow_irreversible);
        let mut cause = result.as_ref().err().cloned();
        match observed {
            Ok(after) if after == before && result.is_err() => {
                record(
                    &mut report,
                    change,
                    ApplyPhase::Verify,
                    StepOutcome::Succeeded,
                    None,
                );
            }
            Ok(after)
                if recovery::expected_after(scheduler, &before, &after, manifest, change)
                    && (result.is_ok()
                        || matches!(
                            change,
                            Change::CreateTask(_)
                                | Change::UpdateTask(_)
                                | Change::AdoptTask(_)
                                | Change::DeleteTask(_)
                                | Change::SetEnabled { .. }
                        )) =>
            {
                record(
                    &mut report,
                    change,
                    ApplyPhase::Verify,
                    StepOutcome::Succeeded,
                    None,
                );
                report.applied.push(change.clone());
                for (later, expected) in plan.changes.iter().zip(&mut expected).skip(index + 1) {
                    if recovery::same_target(change, &later.change) {
                        *expected = after.clone();
                    }
                }
                undo.push(Undo {
                    change: change.clone(),
                    before,
                    after,
                });
            }
            observed => {
                let error = match observed {
                    Err(error) => error,
                    Ok(after) => recovery::verification_conflict(manifest, change, &after),
                };
                record(
                    &mut report,
                    change,
                    ApplyPhase::Verify,
                    StepOutcome::Unknown,
                    Some(error.clone()),
                );
                report.unresolved.push(change.clone());
                cause.get_or_insert(error);
            }
        }
        if let Some(cause) = cause {
            compensate_observed(
                scheduler,
                manifest,
                options,
                &mut prepared,
                &mut report,
                undo,
            );
            return Err(ApplyFailure { cause, report });
        }
    }
    report.status = ApplyStatus::Succeeded;
    Ok(report)
}

struct PreparedApply {
    task_backups: BTreeMap<TaskPath, TaskBackup>,
    enabled_backups: BTreeMap<TaskPath, bool>,
    task_security: BTreeMap<TaskPath, SecuritySnapshot>,
    folder_security: BTreeMap<FolderPath, SecuritySnapshot>,
    desired_passwords: BTreeMap<TaskPath, Password>,
    created_folders: BTreeSet<FolderPath>,
}

struct TaskBackup {
    raw: RawTaskXml,
    logon_type: LogonType,
    enabled: bool,
    security: Option<SecuritySnapshot>,
    password: Option<Password>,
}

#[derive(Clone)]
struct SecuritySnapshot {
    descriptor: SecurityDescriptor,
    information: SecurityInformation,
}

fn prepare(
    scheduler: &impl Backend,
    manifest: &TaskManifest,
    plan: &Plan,
    allow_irreversible: bool,
    resolver: &mut dyn CredentialResolver,
) -> Result<PreparedApply> {
    let mut prepared = PreparedApply {
        task_backups: BTreeMap::new(),
        enabled_backups: BTreeMap::new(),
        task_security: BTreeMap::new(),
        folder_security: BTreeMap::new(),
        desired_passwords: BTreeMap::new(),
        created_folders: plan
            .changes
            .iter()
            .filter_map(|planned| match &planned.change {
                Change::CreateFolder(path) => Some(path.clone()),
                _ => None,
            })
            .collect(),
    };
    for planned in &plan.changes {
        let security_information = recovery::information(manifest, &planned.change);
        match &planned.change {
            Change::CreateTask(path) | Change::UpdateTask(path) | Change::AdoptTask(path) => {
                let desired = managed_task(manifest, path)?;
                if password_backed(desired.definition.principal.logon_type) {
                    let reference = desired.credentials.registration.as_deref();
                    let password = resolver.registration_password(
                        path,
                        reference,
                        CredentialPurpose::DesiredRegistration,
                    )?;
                    prepared.desired_passwords.insert(path.clone(), password);
                }
                if !matches!(planned.change, Change::CreateTask(_)) {
                    prepare_task_backup(
                        scheduler,
                        manifest,
                        path,
                        allow_irreversible,
                        resolver,
                        security_information,
                        &mut prepared,
                    )?;
                }
            }
            Change::DeleteTask(path) => prepare_task_backup(
                scheduler,
                manifest,
                path,
                allow_irreversible,
                resolver,
                security_information,
                &mut prepared,
            )?,
            Change::SetEnabled { path, .. } => {
                if !prepared.enabled_backups.contains_key(path) {
                    let task = scheduler.get_task(path)?;
                    prepared.enabled_backups.insert(path.clone(), task.enabled);
                }
            }
            Change::SetTaskSecurity(path) => {
                let desired = managed_task(manifest, path)?;
                let descriptor = desired
                    .definition
                    .registration
                    .security_descriptor
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new(
                            ErrorKind::InvalidDefinition,
                            "planned task security has no desired descriptor",
                        )
                        .with_target(path.to_string())
                    })?;
                let information = security_information_for_sddl(descriptor);
                match scheduler.task_security(path, information) {
                    Ok(descriptor) => {
                        prepared.task_security.insert(
                            path.clone(),
                            SecuritySnapshot {
                                descriptor,
                                information,
                            },
                        );
                    }
                    Err(_) if allow_irreversible => {}
                    Err(error) => return Err(error),
                }
            }
            Change::SetFolderSecurity(path) => {
                if prepared.created_folders.contains(path) {
                    continue;
                }
                let descriptor = manifest
                    .folders
                    .iter()
                    .find(|folder| folder.path == *path)
                    .and_then(|folder| folder.security_descriptor.as_ref())
                    .ok_or_else(|| {
                        Error::new(
                            ErrorKind::InvalidDefinition,
                            "planned folder security has no desired descriptor",
                        )
                        .with_target(path.to_string())
                    })?;
                let information = security_information_for_sddl(descriptor);
                match scheduler.folder_security(path, information) {
                    Ok(descriptor) => {
                        prepared.folder_security.insert(
                            path.clone(),
                            SecuritySnapshot {
                                descriptor,
                                information,
                            },
                        );
                    }
                    Err(_) if allow_irreversible => {}
                    Err(error) => return Err(error),
                }
            }
            Change::CreateFolder(_) => {}
        }
    }
    Ok(prepared)
}

fn prepare_task_backup(
    scheduler: &impl Backend,
    manifest: &TaskManifest,
    path: &TaskPath,
    allow_irreversible: bool,
    resolver: &mut dyn CredentialResolver,
    security_information: SecurityInformation,
    prepared: &mut PreparedApply,
) -> Result<()> {
    if prepared.task_backups.contains_key(path) {
        return Ok(());
    }
    let task = scheduler.get_task(path)?;
    let definition = task.snapshot.definition()?.clone();
    let logon_type = definition.principal.logon_type;
    let reference = manifest
        .tasks
        .iter()
        .find(|desired| desired.path == *path)
        .and_then(|desired| desired.credentials.registration.as_deref());
    let password = if password_backed(logon_type) {
        match resolver.registration_password(path, reference, CredentialPurpose::Rollback) {
            Ok(password) => Some(password),
            Err(_) if allow_irreversible => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let security = match scheduler.task_security(path, security_information) {
        Ok(descriptor) => Some(SecuritySnapshot {
            descriptor,
            information: security_information,
        }),
        Err(_) if allow_irreversible => None,
        Err(error) => return Err(error),
    };
    prepared.task_backups.insert(
        path.clone(),
        TaskBackup {
            raw: task.snapshot.raw().clone(),
            logon_type,
            enabled: task.enabled,
            security,
            password,
        },
    );
    Ok(())
}

fn execute_change(
    scheduler: &impl Backend,
    manifest: &TaskManifest,
    change: &Change,
    options: ApplyOptions,
    prepared: &mut PreparedApply,
) -> Result<()> {
    match change {
        Change::CreateFolder(path) => {
            scheduler.create_folder(path, None)?;
        }
        Change::CreateTask(path) | Change::UpdateTask(path) | Change::AdoptTask(path) => {
            // Stop requests belong to apply_backend's journaled phase.
            let desired = managed_task(manifest, path)?;
            let mut definition = owned_definition(manifest, desired);
            if !matches!(change, Change::CreateTask(_))
                && definition.registration.security_descriptor.is_none()
            {
                definition.registration.security_descriptor = prepared
                    .task_backups
                    .get(path)
                    .and_then(|backup| backup.security.as_ref())
                    .map(|security| security.descriptor.clone());
            }
            let registration = RegistrationOptions {
                mode: if matches!(change, Change::CreateTask(_)) {
                    RegistrationMode::Create
                } else {
                    RegistrationMode::CreateOrUpdate
                },
                ignore_registration_triggers: options.ignore_registration_triggers,
                password: prepared.desired_passwords.remove(path),
                dont_add_principal_ace: definition.registration.security_descriptor.is_some(),
                ..RegistrationOptions::default()
            };
            scheduler.register(path, definition, registration)?;
            // Read-back is a separate journaled boundary. A failure here must
            // still compensate the registration that has already committed.
            scheduler.get_task(path)?;
        }
        Change::DeleteTask(path) => {
            scheduler.delete_task(path)?;
        }
        Change::SetEnabled { path, enabled } => scheduler.set_enabled(path, *enabled)?,
        Change::SetTaskSecurity(path) => {
            let desired = managed_task(manifest, path)?;
            let descriptor = desired
                .definition
                .registration
                .security_descriptor
                .clone()
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidDefinition,
                        "planned task security has no desired descriptor",
                    )
                    .with_target(path.to_string())
                })?;
            let information = security_information_for_sddl(&descriptor);
            scheduler.set_task_security(path, descriptor, information)?;
        }
        Change::SetFolderSecurity(path) => {
            let descriptor = manifest
                .folders
                .iter()
                .find(|folder| folder.path == *path)
                .and_then(|folder| folder.security_descriptor.clone())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::InvalidDefinition,
                        "planned folder security has no desired descriptor",
                    )
                    .with_target(path.to_string())
                })?;
            let information = security_information_for_sddl(&descriptor);
            scheduler.set_folder_security(path, descriptor, information)?;
        }
    }
    Ok(())
}

fn compensate_change(
    scheduler: &impl Backend,
    change: &Change,
    options: ApplyOptions,
    prepared: &mut PreparedApply,
) -> Result<()> {
    match change {
        Change::CreateFolder(path) => scheduler.delete_folder(path),
        Change::CreateTask(path) => {
            if options.stop_running {
                scheduler.stop_all(path)?;
            }
            scheduler.delete_task(path)
        }
        Change::UpdateTask(path) | Change::AdoptTask(path) | Change::DeleteTask(path) => {
            let backup = prepared.task_backups.remove(path).ok_or_else(|| {
                Error::new(
                    ErrorKind::Irreversible,
                    "task rollback snapshot is unavailable",
                )
                .with_target(path.to_string())
            })?;
            if password_backed(backup.logon_type) && backup.password.is_none() {
                return Err(Error::new(
                    ErrorKind::Irreversible,
                    "task rollback password is unavailable",
                )
                .with_target(path.to_string()));
            }
            scheduler.register_raw(
                path,
                backup.raw,
                backup.logon_type,
                RegistrationOptions {
                    mode: RegistrationMode::CreateOrUpdate,
                    ignore_registration_triggers: options.ignore_registration_triggers,
                    password: backup.password,
                    ..RegistrationOptions::default()
                },
            )?;
            let enabled_result = scheduler.set_enabled(path, backup.enabled);
            let security_result = backup.security.map_or(Ok(()), |security| {
                scheduler.set_task_security(path, security.descriptor, security.information)
            });
            // ACL recovery is independent of enabled-state restoration. Both
            // attempts are retained by the recording backend even if one fails.
            enabled_result.and(security_result)
        }
        Change::SetEnabled { path, .. } => {
            let enabled = prepared.enabled_backups.get(path).ok_or_else(|| {
                Error::new(
                    ErrorKind::Irreversible,
                    "task enabled-state snapshot is unavailable",
                )
                .with_target(path.to_string())
            })?;
            scheduler.set_enabled(path, *enabled)
        }
        Change::SetTaskSecurity(path) => {
            let security = prepared.task_security.get(path).ok_or_else(|| {
                Error::new(
                    ErrorKind::Irreversible,
                    "task security snapshot is unavailable",
                )
                .with_target(path.to_string())
            })?;
            scheduler.set_task_security(path, security.descriptor.clone(), security.information)
        }
        Change::SetFolderSecurity(path) => {
            let security = prepared.folder_security.remove(path).ok_or_else(|| {
                Error::new(
                    ErrorKind::Irreversible,
                    "folder security snapshot is unavailable",
                )
                .with_target(path.to_string())
            })?;
            scheduler.set_folder_security(path, security.descriptor, security.information)
        }
    }
}

fn empty_apply_failure(cause: Error) -> ApplyFailure {
    ApplyFailure {
        cause,
        report: ApplyReport {
            status: ApplyStatus::Failed,
            plan: Plan::default(),
            applied: Vec::new(),
            rolled_back: Vec::new(),
            rollback_failures: Vec::new(),
            journal: Vec::new(),
            unresolved: Vec::new(),
            irreversible_effects: Vec::new(),
        },
    }
}

fn ensure_valid_manifest(manifest: &TaskManifest) -> Result<()> {
    let report = manifest.validate();
    if report.is_valid() {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidDefinition,
            "desired-state manifest is invalid",
        )
        .with_validation(report))
    }
}

fn managed_task<'a>(manifest: &'a TaskManifest, path: &TaskPath) -> Result<&'a ManagedTask> {
    manifest
        .tasks
        .iter()
        .find(|task| task.path == *path)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidDefinition,
                "planned task is absent from the manifest",
            )
            .with_target(path.to_string())
        })
}

fn owned_definition(manifest: &TaskManifest, task: &ManagedTask) -> TaskDefinition {
    let mut definition = task.definition.clone();
    let marker = manifest.ownership_uri(&task.path);
    definition.registration.source = Some(definition.registration.source.as_ref().map_or_else(
        || marker.clone(),
        |source| {
            if source == &marker || source.starts_with(&format!("{marker}\n")) {
                source.clone()
            } else {
                format!("{marker}\n{source}")
            }
        },
    ));
    // Windows rewrites URI to the registered task path. Source is preserved.
    definition.registration.uri = Some(task.path.to_string());
    definition
}

fn definitions_semantically_equal(left: &TaskDefinition, right: &TaskDefinition) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.registration.date = None;
    right.registration.date = None;
    left.registration.security_descriptor = None;
    right.registration.security_descriptor = None;
    left.settings.enabled = true;
    right.settings.enabled = true;
    left == right
}

fn rollback_safety(task: &CurrentTask) -> RollbackSafety {
    if task.password_backed {
        RollbackSafety::RequiresCredential
    } else {
        RollbackSafety::Reversible
    }
}

const fn password_backed(logon_type: LogonType) -> bool {
    matches!(
        logon_type,
        LogonType::Password | LogonType::InteractiveTokenOrPassword
    )
}

fn security_information_for_sddl(descriptor: &SecurityDescriptor) -> SecurityInformation {
    let sddl = descriptor.as_sddl();
    let mut information = SecurityInformation::empty();
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in sddl.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && sddl.as_bytes().get(index + 1) == Some(&b':') => {
                information |= match character {
                    'O' => SecurityInformation::OWNER,
                    'G' => SecurityInformation::GROUP,
                    'D' => SecurityInformation::DACL,
                    'S' => SecurityInformation::SACL,
                    _ => SecurityInformation::empty(),
                };
            }
            _ => {}
        }
    }
    if information.is_empty() {
        SecurityInformation::DACL
    } else {
        information
    }
}

fn required_folders(manifest: &TaskManifest) -> Vec<FolderPath> {
    let mut folders = BTreeSet::new();
    folders.insert(manifest.namespace.clone());
    for path in manifest
        .folders
        .iter()
        .map(|folder| folder.path.clone())
        .chain(manifest.tasks.iter().map(|task| task.path.folder()))
    {
        let mut cursor = Some(path);
        while let Some(folder) = cursor {
            folders.insert(folder.clone());
            if folder == manifest.namespace {
                break;
            }
            cursor = folder.parent();
        }
    }
    let mut folders: Vec<_> = folders.into_iter().collect();
    folders.sort_by_key(|folder| (folder_depth(folder), folder.clone()));
    folders
}

fn folder_depth(path: &FolderPath) -> usize {
    path.as_str()
        .split('\\')
        .filter(|component| !component.is_empty())
        .count()
}

fn change_order(change: &Change) -> (u8, usize, String) {
    match change {
        Change::CreateFolder(path) => (0, folder_depth(path), path.to_string()),
        Change::CreateTask(path) => (1, 0, path.to_string()),
        Change::AdoptTask(path) => (2, 0, path.to_string()),
        Change::UpdateTask(path) => (3, 0, path.to_string()),
        Change::SetEnabled { path, .. } => (4, 0, path.to_string()),
        Change::SetTaskSecurity(path) => (5, 0, path.to_string()),
        Change::SetFolderSecurity(path) => (5, folder_depth(path), path.to_string()),
        Change::DeleteTask(path) => (6, 0, std::cmp::Reverse(path.to_string()).0),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uuid::Uuid;

    use super::{
        Change, CurrentState, CurrentTask, PlanOptions, plan_state, security_information_for_sddl,
    };
    use crate::{
        FolderPath, SecurityDescriptor, TaskPath,
        client::SecurityInformation,
        manifest::{ManagedTask, TaskManifest},
        model::{Action, ExecAction, TaskDefinition},
    };

    fn definition() -> TaskDefinition {
        let mut definition = TaskDefinition::default();
        definition
            .actions
            .push(Action::Exec(ExecAction::new("cmd.exe")))
            .expect("action");
        definition
    }

    #[test]
    fn creates_implicit_parents_before_tasks() {
        let namespace = FolderPath::from_str("\\Acme").expect("namespace");
        let task_path = TaskPath::from_str("\\Acme\\Jobs\\Backup").expect("task path");
        let mut manifest = TaskManifest::new(Uuid::nil(), "tests", namespace);
        manifest.tasks.push(ManagedTask {
            path: task_path.clone(),
            definition: definition(),
            credentials: Default::default(),
        });
        let plan =
            plan_state(&manifest, &CurrentState::default(), PlanOptions::default()).expect("plan");
        let changes: Vec<_> = plan
            .changes
            .iter()
            .map(|change| change.change.clone())
            .collect();
        assert_eq!(
            changes,
            vec![
                Change::CreateFolder(FolderPath::from_str("\\Acme").expect("folder")),
                Change::CreateFolder(FolderPath::from_str("\\Acme\\Jobs").expect("folder")),
                Change::CreateTask(task_path),
            ]
        );
    }

    #[test]
    fn security_operations_follow_the_sddl_sections() {
        let descriptor = SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)S:(AU;SA;GR;;;WD)")
            .expect("valid non-empty SDDL");

        assert_eq!(
            security_information_for_sddl(&descriptor),
            SecurityInformation::DACL | SecurityInformation::SACL,
            "owner and group must not be read or replaced when absent"
        );
    }

    #[test]
    fn plans_enabled_and_security_drift_without_replacing_the_task() {
        let namespace = FolderPath::from_str("\\Acme").expect("namespace");
        let task_path = TaskPath::from_str("\\Acme\\Backup").expect("task path");
        let mut desired = definition();
        desired.registration.security_descriptor =
            Some(SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("desired SDDL"));
        let mut manifest = TaskManifest::new(Uuid::nil(), "tests", namespace.clone());
        manifest.tasks.push(ManagedTask {
            path: task_path.clone(),
            definition: desired.clone(),
            credentials: Default::default(),
        });

        let mut existing = desired;
        existing.registration.uri = Some(task_path.to_string());
        existing.registration.source = Some(manifest.ownership_uri(&task_path));
        existing.registration.security_descriptor =
            Some(SecurityDescriptor::from_sddl("D:(A;;GA;;;SY)").expect("existing SDDL"));
        existing.settings.enabled = false;
        let current = CurrentState {
            tasks: vec![CurrentTask {
                path: task_path.clone(),
                definition: existing,
                enabled: false,
                owned: true,
                password_backed: false,
            }],
            folders: std::iter::once((namespace, None)).collect(),
        };

        let plan = plan_state(&manifest, &current, PlanOptions::default()).expect("plan");
        assert_eq!(
            plan.changes
                .into_iter()
                .map(|planned| planned.change)
                .collect::<Vec<_>>(),
            vec![
                Change::SetEnabled {
                    path: task_path.clone(),
                    enabled: true,
                },
                Change::SetTaskSecurity(task_path),
            ]
        );
    }
}

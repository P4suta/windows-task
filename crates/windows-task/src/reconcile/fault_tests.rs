use super::*;
use crate::{
    client::{RegisteredTask, TaskFolder, TaskState},
    model::{Action, ExecAction},
    xml::TaskSnapshot,
};
use std::cell::RefCell;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct State {
    tasks: BTreeMap<TaskPath, TaskDefinition>,
    folders: BTreeMap<FolderPath, SecurityDescriptor>,
}

struct Fault {
    operation: &'static str,
    occurrence: usize,
    after: bool,
}
#[derive(Default)]
struct Fake {
    state: RefCell<State>,
    calls: RefCell<Vec<&'static str>>,
    faults: RefCell<Vec<Fault>>,
    drift: RefCell<Option<(&'static str, usize, TaskPath)>>,
}

fn acl() -> SecurityDescriptor {
    SecurityDescriptor::from_sddl("D:(A;;GA;;;SY)").expect("fixture ACL")
}
fn missing() -> Error {
    Error::new(ErrorKind::NotFound, "fixture object absent")
}
impl Fake {
    fn invoke<T>(
        &self,
        name: &'static str,
        operation: impl FnOnce(&mut State) -> Result<T>,
    ) -> Result<T> {
        self.calls.borrow_mut().push(name);
        let occurrence = self
            .calls
            .borrow()
            .iter()
            .filter(|call| **call == name)
            .count();
        let drift = self
            .drift
            .borrow()
            .as_ref()
            .filter(|(operation, at, _)| *operation == name && *at == occurrence)
            .cloned();
        if let Some((_, _, path)) = drift {
            self.drift.borrow_mut().take();
            self.state
                .borrow_mut()
                .tasks
                .get_mut(&path)
                .expect("drift fixture")
                .registration
                .uri = Some("foreign-owner".into());
        }
        let index = self
            .faults
            .borrow()
            .iter()
            .position(|fault| fault.operation == name && fault.occurrence == occurrence);
        let fault = index.map(|index| self.faults.borrow_mut().remove(index));
        if fault.as_ref().is_some_and(|fault| !fault.after) {
            return Err(Error::new(
                ErrorKind::AccessDenied,
                "injected before operation",
            ));
        }
        let result = operation(&mut self.state.borrow_mut());
        if fault.is_some() {
            return Err(Error::new(
                ErrorKind::SchedulerUnavailable,
                "injected response loss",
            ));
        }
        result
    }
    fn fail(&self, operation: &'static str, occurrence: usize, after: bool) {
        self.faults.borrow_mut().push(Fault {
            operation,
            occurrence,
            after,
        });
    }
}

fn registered(path: &TaskPath, definition: &TaskDefinition) -> Result<RegisteredTask> {
    let mut xml_definition = definition.clone();
    xml_definition.registration.security_descriptor = None;
    Ok(RegisteredTask {
        path: path.clone(),
        state: TaskState::Ready,
        enabled: definition.settings.enabled,
        last_result: 0,
        missed_runs: 0,
        last_run: None,
        next_run: None,
        snapshot: TaskSnapshot::parse(crate::xml::to_string(&xml_definition)?.into_bytes())?,
    })
}

impl Backend for Fake {
    fn get_task(&self, path: &TaskPath) -> Result<RegisteredTask> {
        self.invoke("get_task", |state| {
            registered(path, state.tasks.get(path).ok_or_else(missing)?)
        })
    }
    fn list_tasks(&self, path: &FolderPath, _: ListOptions) -> Result<Vec<RegisteredTask>> {
        self.invoke("list_tasks", |state| {
            if !state.folders.contains_key(path) {
                return Err(missing());
            }
            state
                .tasks
                .iter()
                .map(|(path, definition)| registered(path, definition))
                .collect()
        })
    }
    fn list_folders(&self, path: &FolderPath, _: bool) -> Result<Vec<TaskFolder>> {
        self.invoke("list_folders", |state| {
            if !state.folders.contains_key(path) {
                return Err(missing());
            }
            Ok(state
                .folders
                .keys()
                .filter(|folder| *folder != path)
                .map(|path| TaskFolder {
                    path: path.clone(),
                    security_descriptor: None,
                })
                .collect())
        })
    }
    fn task_security(&self, path: &TaskPath, _: SecurityInformation) -> Result<SecurityDescriptor> {
        self.invoke("task_security", |state| {
            Ok(state
                .tasks
                .get(path)
                .ok_or_else(missing)?
                .registration
                .security_descriptor
                .clone()
                .unwrap_or_else(acl))
        })
    }
    fn folder_security(
        &self,
        path: &FolderPath,
        _: SecurityInformation,
    ) -> Result<SecurityDescriptor> {
        self.invoke("folder_security", |state| {
            state.folders.get(path).cloned().ok_or_else(missing)
        })
    }
    fn register(
        &self,
        path: &TaskPath,
        mut definition: TaskDefinition,
        _: RegistrationOptions,
    ) -> Result<()> {
        self.invoke("register", |state| {
            definition
                .registration
                .security_descriptor
                .get_or_insert_with(acl);
            state.tasks.insert(path.clone(), definition);
            Ok(())
        })
    }
    fn register_raw(
        &self,
        path: &TaskPath,
        raw: RawTaskXml,
        _: LogonType,
        _: RegistrationOptions,
    ) -> Result<()> {
        self.invoke("register_raw", |state| {
            let mut definition = raw.definition()?;
            definition
                .registration
                .security_descriptor
                .get_or_insert_with(acl);
            state.tasks.insert(path.clone(), definition);
            Ok(())
        })
    }
    fn create_folder(
        &self,
        path: &FolderPath,
        security: Option<SecurityDescriptor>,
    ) -> Result<TaskFolder> {
        self.invoke("create_folder", |state| {
            state
                .folders
                .insert(path.clone(), security.unwrap_or_else(acl));
            Ok(TaskFolder {
                path: path.clone(),
                security_descriptor: None,
            })
        })
    }
    fn delete_folder(&self, path: &FolderPath) -> Result<()> {
        self.invoke("delete_folder", |state| {
            if state.tasks.keys().any(|task| task.folder() == *path) {
                return Err(Error::new(ErrorKind::Conflict, "folder not empty"));
            }
            state.folders.remove(path).ok_or_else(missing)?;
            Ok(())
        })
    }
    fn delete_task(&self, path: &TaskPath) -> Result<()> {
        self.invoke("delete_task", |state| {
            state.tasks.remove(path).ok_or_else(missing)?;
            Ok(())
        })
    }
    fn set_enabled(&self, path: &TaskPath, enabled: bool) -> Result<()> {
        self.invoke("set_enabled", |state| {
            state
                .tasks
                .get_mut(path)
                .ok_or_else(missing)?
                .settings
                .enabled = enabled;
            Ok(())
        })
    }
    fn set_task_security(
        &self,
        path: &TaskPath,
        descriptor: SecurityDescriptor,
        _: SecurityInformation,
    ) -> Result<()> {
        self.invoke("set_task_security", |state| {
            state
                .tasks
                .get_mut(path)
                .ok_or_else(missing)?
                .registration
                .security_descriptor = Some(descriptor);
            Ok(())
        })
    }
    fn set_folder_security(
        &self,
        path: &FolderPath,
        descriptor: SecurityDescriptor,
        _: SecurityInformation,
    ) -> Result<()> {
        self.invoke("set_folder_security", |state| {
            *state.folders.get_mut(path).ok_or_else(missing)? = descriptor;
            Ok(())
        })
    }
    fn stop_all(&self, _: &TaskPath) -> Result<()> {
        self.invoke("stop_all", |_| Ok(()))
    }
}

fn fixture(count: usize) -> (Fake, TaskManifest) {
    let backend = Fake::default();
    let namespace: FolderPath = "\\Test".parse().expect("fixture namespace");
    backend
        .state
        .borrow_mut()
        .folders
        .insert(namespace.clone(), acl());
    let mut manifest = TaskManifest::new(uuid::Uuid::nil(), "fault-tests", namespace.clone());
    for index in 0..count {
        manifest.tasks.push(ManagedTask {
            path: namespace
                .task(&format!("Task{index}"))
                .expect("fixture path"),
            definition: TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe"))),
            credentials: Default::default(),
        });
    }
    (backend, manifest)
}

#[test]
fn successful_apply_is_idempotent() {
    let (backend, manifest) = fixture(2);
    let first = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("first apply");
    assert!(first.succeeded());
    assert!(!first.journal.is_empty());
    let second = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("second apply");
    assert!(second.plan.is_empty());
}

#[test]
fn registration_response_loss_is_observed_and_compensated() {
    let (backend, manifest) = fixture(1);
    backend.fail("register", 1, true);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("lost response");
    assert_eq!(failure.cause.kind(), ErrorKind::SchedulerUnavailable);
    assert!(failure.report.rollback_complete());
    assert!(backend.state.borrow().tasks.is_empty());
}

#[test]
fn committed_registration_is_rolled_back_when_readback_fails() {
    let (backend, manifest) = fixture(1);
    backend.fail("get_task", 3, false);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("registration readback failed");
    assert!(failure.report.rollback_complete());
    assert!(backend.state.borrow().tasks.is_empty());
    assert!(
        failure
            .report
            .journal
            .iter()
            .any(|entry| entry.operation == "register" && entry.outcome == StepOutcome::Succeeded)
    );
    assert!(
        failure
            .report
            .journal
            .iter()
            .any(|entry| entry.operation == "get_task" && entry.error.is_some())
    );
}

#[test]
fn failed_compensation_does_not_skip_independent_earlier_changes() {
    let (backend, manifest) = fixture(3);
    backend.fail("register", 3, false);
    backend.fail("delete_task", 1, false);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("third registration fails");
    assert_eq!(failure.report.rollback_failures.len(), 1);
    assert_eq!(failure.report.rolled_back.len(), 1);
    assert_eq!(backend.state.borrow().tasks.len(), 1);
    assert!(!failure.report.rollback_complete());
}

#[test]
fn permission_failure_during_preflight_has_no_mutations() {
    let (backend, manifest) = fixture(1);
    backend.fail("folder_security", 1, false);
    // Force a folder ACL change so the preflight must read its descriptor.
    let mut manifest = manifest;
    manifest.folders.push(crate::manifest::ManagedFolder {
        path: manifest.namespace.clone(),
        security_descriptor: Some(acl()),
    });
    apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("preflight denied");
    assert!(!backend.calls.borrow().contains(&"register"));
}

#[test]
fn stop_side_effect_is_not_reported_as_fully_reversible() {
    let (backend, mut manifest) = fixture(1);
    apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("initial apply");
    manifest.tasks[0].definition.actions = Default::default();
    manifest.tasks[0]
        .definition
        .actions
        .push(Action::Exec(ExecAction::new("changed.exe")))
        .expect("one action");
    backend.fail("register", 2, true);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions {
            stop_running: true,
            ..ApplyOptions::default()
        },
        &mut NoCredentials,
    )
    .expect_err("response lost after stop");
    assert_eq!(failure.report.irreversible_effects.len(), 1);
    assert!(!failure.report.rollback_complete());
}

fn apply_fixture(backend: &Fake, manifest: &TaskManifest) {
    apply_backend(
        backend,
        manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("initial apply");
    backend.calls.borrow_mut().clear();
}

#[test]
fn every_mutation_has_before_and_after_failure_evidence() {
    for variant in 0..8 {
        for after in [false, true] {
            let (backend, mut manifest) = fixture(1);
            let mut options = ApplyOptions::default();
            if variant != 0 && variant != 7 {
                apply_fixture(&backend, &manifest);
            }
            let operation = match variant {
                0 => "register",
                1 => {
                    manifest.tasks[0].definition.registration.description = Some("updated".into());
                    "register"
                }
                2 => {
                    backend
                        .state
                        .borrow_mut()
                        .tasks
                        .get_mut(&manifest.tasks[0].path)
                        .expect("existing")
                        .registration
                        .uri = None;
                    backend
                        .state
                        .borrow_mut()
                        .tasks
                        .get_mut(&manifest.tasks[0].path)
                        .expect("existing")
                        .registration
                        .source = None;
                    options.adopt = true;
                    "register"
                }
                3 => {
                    manifest.tasks.clear();
                    options.prune = true;
                    "delete_task"
                }
                4 => {
                    manifest.tasks[0].definition.settings.enabled = false;
                    "set_enabled"
                }
                5 => {
                    manifest.tasks[0]
                        .definition
                        .registration
                        .security_descriptor =
                        Some(SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("changed ACL"));
                    "set_task_security"
                }
                6 => {
                    manifest.folders.push(crate::manifest::ManagedFolder {
                        path: manifest.namespace.clone(),
                        security_descriptor: Some(
                            SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("changed ACL"),
                        ),
                    });
                    "set_folder_security"
                }
                7 => {
                    backend.state.borrow_mut().folders.clear();
                    "create_folder"
                }
                _ => unreachable!(),
            };
            let before = backend.state.borrow().clone();
            backend.fail(operation, 1, after);
            let failure = apply_backend(&backend, &manifest, options, &mut NoCredentials)
                .expect_err("injected mutation failure");
            assert!(
                failure
                    .report
                    .journal
                    .iter()
                    .any(|entry| entry.operation == operation),
                "missing journal variant={variant}, after={after}"
            );
            if after && variant >= 5 {
                // An ACL/folder has no ownership marker that can establish who
                // performed an ambiguous write. Preserve it for manual review.
                assert!(!failure.report.unresolved.is_empty());
                assert!(!failure.report.rollback_complete());
            } else {
                assert_eq!(
                    *backend.state.borrow(),
                    before,
                    "variant={variant}, after={after}"
                );
                assert!(failure.report.unresolved.is_empty());
            }
        }
    }
}

#[test]
fn drift_during_preparation_is_rejected_before_mutation() {
    let (backend, mut manifest) = fixture(1);
    apply_fixture(&backend, &manifest);
    manifest.tasks[0].definition.registration.description = Some("updated".into());
    *backend.drift.borrow_mut() = Some(("get_task", 2, manifest.tasks[0].path.clone()));
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("external change");
    assert_eq!(failure.cause.kind(), ErrorKind::Conflict);
    assert!(!backend.calls.borrow().contains(&"register"));
}

#[test]
fn rollback_preserves_foreign_changes_and_continues() {
    let (backend, manifest) = fixture(3);
    backend.fail("register", 3, false);
    *backend.drift.borrow_mut() = Some(("register", 3, manifest.tasks[1].path.clone()));
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("external change while third registration fails");
    assert_eq!(
        failure.report.rollback_failures[0].error.kind(),
        ErrorKind::Conflict
    );
    assert_eq!(failure.report.rolled_back.len(), 1);
    assert_eq!(backend.state.borrow().tasks.len(), 1);
    assert_eq!(
        backend.state.borrow().tasks[&manifest.tasks[1].path]
            .registration
            .uri
            .as_deref(),
        Some("foreign-owner")
    );
}

#[test]
fn acl_restore_is_attempted_even_when_enabled_restore_fails() {
    let (backend, mut manifest) = fixture(1);
    apply_fixture(&backend, &manifest);
    manifest.tasks[0].definition.registration.description = Some("updated".into());
    backend.fail("register", 1, true);
    backend.fail("set_enabled", 1, false);
    backend.fail("set_task_security", 1, false);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("restore failed");
    for operation in ["set_enabled", "set_task_security"] {
        assert!(
            failure
                .report
                .journal
                .iter()
                .any(|entry| entry.phase == ApplyPhase::Rollback
                    && entry.operation == operation
                    && entry.error.is_some())
        );
    }
    assert!(!failure.report.rollback_complete());
}

#[test]
fn source_payload_survives_apply_and_path_alone_never_grants_ownership() {
    let (backend, mut manifest) = fixture(1);
    manifest.tasks[0].definition.registration.source = Some("application-origin".into());
    apply_fixture(&backend, &manifest);
    let path = &manifest.tasks[0].path;
    assert_eq!(
        backend.state.borrow().tasks[path]
            .registration
            .source
            .as_deref(),
        Some(format!("{}\napplication-origin", manifest.ownership_uri(path)).as_str())
    );
    let second = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("idempotent owned task");
    assert!(second.plan.is_empty());
    backend
        .state
        .borrow_mut()
        .tasks
        .get_mut(path)
        .expect("task")
        .registration
        .source = None;
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("task path is not ownership");
    assert_eq!(failure.cause.kind(), ErrorKind::Conflict);
    assert!(failure.report.applied.is_empty());
}

#[test]
fn exporting_an_owned_definition_back_into_the_manifest_converges() {
    let (backend, mut manifest) = fixture(1);
    manifest.tasks[0].definition.registration.source = Some("original-source".into());
    apply_fixture(&backend, &manifest);
    manifest.tasks[0].definition = backend.state.borrow().tasks[&manifest.tasks[0].path].clone();
    let second = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("exported definition remains owned");
    assert!(second.plan.is_empty());
    assert!(
        manifest.tasks[0]
            .definition
            .registration
            .source
            .as_ref()
            .expect("source")
            .ends_with("\noriginal-source")
    );
}

#[test]
fn missing_credentials_prevents_any_registration() {
    let (backend, mut manifest) = fixture(1);
    manifest.tasks[0].definition.principal.identity =
        crate::model::PrincipalIdentity::User("fixture-user".into());
    manifest.tasks[0].definition.principal.logon_type = LogonType::Password;
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("missing password");
    assert_eq!(failure.cause.kind(), ErrorKind::Authentication);
    assert!(!backend.calls.borrow().contains(&"register"));
}

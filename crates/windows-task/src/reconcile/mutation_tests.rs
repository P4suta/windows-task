//! Behavioral contracts prompted by the first recorded mutation run.
use super::*;

#[test]
fn registration_preserves_existing_acl_unless_an_explicit_replacement_is_requested() {
    for replace in [false, true] {
        let (backend, mut manifest) = fixture(1);
        let original = SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("old access");
        let replacement = SecurityDescriptor::from_sddl("D:(A;;GW;;;SY)").expect("new access");
        manifest.tasks[0]
            .definition
            .registration
            .security_descriptor = Some(original.clone());
        apply_fixture(&backend, &manifest);
        manifest.tasks[0].definition.registration.description = Some("new definition".into());
        manifest.tasks[0]
            .definition
            .registration
            .security_descriptor = replace.then_some(replacement.clone());
        apply_fixture(&backend, &manifest);
        assert_eq!(
            backend.state.borrow().tasks[&manifest.tasks[0].path]
                .registration
                .security_descriptor,
            Some(if replace { replacement } else { original }),
            "registration must distinguish preserved and explicitly replaced access"
        );
        let report = apply_backend(
            &backend,
            &manifest,
            ApplyOptions::default(),
            &mut NoCredentials,
        )
        .expect("repeat apply");
        assert!(report.plan.is_empty());
    }
}

#[test]
fn canonical_plan_creates_shallow_folders_first_and_changes_task_acl_before_folder_acl() {
    let (backend, mut manifest) = fixture(1);
    let parent = manifest.namespace.join("A").expect("parent");
    let nested = parent.join("B").expect("child");
    let sibling = manifest.namespace.join("Z").expect("sibling");
    for path in [nested.clone(), sibling.clone(), parent.clone()] {
        manifest.folders.push(crate::manifest::ManagedFolder {
            path,
            security_descriptor: Some(acl()),
        });
    }
    manifest.tasks[0]
        .definition
        .registration
        .security_descriptor = Some(acl());
    let initial = plan(&manifest, &[], false).expect("initial plan");
    let creates: Vec<_> = initial
        .changes
        .iter()
        .filter_map(|planned| {
            if let Change::CreateFolder(path) = &planned.change {
                Some(path.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        creates,
        [manifest.namespace.clone(), parent, sibling, nested]
    );
    apply_fixture(&backend, &manifest);
    let changed = SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("changed access");
    manifest.tasks[0]
        .definition
        .registration
        .security_descriptor = Some(changed.clone());
    for folder in &mut manifest.folders {
        folder.security_descriptor = Some(changed.clone());
    }
    let state = inspect_backend(&backend, &manifest).expect("current state");
    let update = plan_state(&manifest, &state, PlanOptions::default()).expect("ACL plan");
    assert!(
        matches!(update.changes[0].change, Change::SetTaskSecurity(_)),
        "task access must be configured before a containing folder can restrict it"
    );
}

#[test]
fn an_empty_failure_report_is_not_a_successful_no_op() {
    let (backend, mut manifest) = fixture(0);
    let report = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("valid no-op");
    assert!(report.succeeded());
    assert_eq!(
        serde_json::to_value(&report).expect("report")["status"],
        "succeeded"
    );
    manifest.namespace = FolderPath::root();
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("invalid namespace before planning");
    assert!(failure.report.plan.is_empty());
    assert_eq!(
        serde_json::to_value(&failure.report).expect("failure report")["status"],
        "failed"
    );
    assert!(
        !failure.report.succeeded(),
        "preflight failure must not look like a successful no-op"
    );
}

#[test]
fn new_folder_compensation_restores_the_observed_sacl_too() {
    let (backend, mut manifest) = fixture(2);
    apply_fixture(&backend, &manifest);
    let folder = manifest.namespace.join("Audit").expect("new folder");
    manifest.folders.push(crate::manifest::ManagedFolder {
        path: folder,
        security_descriptor: Some(
            SecurityDescriptor::from_sddl("S:(AU;SA;GR;;;WD)").expect("audit ACL"),
        ),
    });
    manifest.tasks.truncate(1);
    backend.fail("delete_task", 1, false);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions {
            prune: true,
            ..ApplyOptions::default()
        },
        &mut NoCredentials,
    )
    .expect_err("later prune fails");
    assert!(failure.report.rollback_complete());
    assert_eq!(
        *backend.folder_security_writes.borrow(),
        [SecurityInformation::SACL, SecurityInformation::all()]
    );
}

#[test]
fn plan_order_is_independent_of_manifest_input_order() {
    let (_, manifest) = fixture(4);
    let first = plan(&manifest, &[], false).expect("first order");
    let mut reordered = manifest;
    reordered.tasks.reverse();
    assert_eq!(
        plan(&reordered, &[], false).expect("reordered input"),
        first
    );
}

#[test]
fn inspection_and_acl_verification_reject_each_independent_state_change() {
    let (backend, mut manifest) = fixture(1);
    apply_fixture(&backend, &manifest);
    let state = inspect_backend(&backend, &manifest).expect("inspection");
    let path = manifest.tasks[0].path.clone();
    let change = Change::SetTaskSecurity(path.clone());
    let before = observe(&backend, &manifest, &change, false).expect("snapshot");
    assert!(recovery::agrees_with_inspection(&state, &change, &before));
    for enabled_drift in [false, true] {
        let mut after = before.clone();
        let Observed::Task {
            definition,
            enabled,
            ..
        } = &mut after
        else {
            panic!("task")
        };
        if enabled_drift {
            *enabled = !*enabled;
        } else {
            definition.registration.description = Some("external edit".into());
        }
        assert!(!recovery::agrees_with_inspection(&state, &change, &after));
    }
    manifest.tasks[0]
        .definition
        .registration
        .security_descriptor =
        Some(SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("desired ACL"));
    assert!(
        !recovery::expected_after(&backend, &before, &before, &manifest, &change),
        "unchanged definition cannot excuse incorrect ACL"
    );
    backend
        .state
        .borrow_mut()
        .tasks
        .get_mut(&path)
        .expect("task")
        .registration
        .security_descriptor = manifest.tasks[0]
        .definition
        .registration
        .security_descriptor
        .clone();
    assert!(recovery::expected_after(
        &backend, &before, &before, &manifest, &change
    ));
}

#[test]
fn security_observation_includes_base_sections_and_only_the_matching_folder() {
    let (_, mut manifest) = fixture(1);
    let base = SecurityInformation::OWNER | SecurityInformation::GROUP | SecurityInformation::DACL;
    assert_eq!(
        recovery::information(
            &manifest,
            &Change::UpdateTask(manifest.tasks[0].path.clone())
        ),
        base
    );
    let child = manifest.namespace.join("Audit").expect("child");
    manifest.folders.extend([
        crate::manifest::ManagedFolder {
            path: manifest.namespace.clone(),
            security_descriptor: Some(acl()),
        },
        crate::manifest::ManagedFolder {
            path: child.clone(),
            security_descriptor: Some(
                SecurityDescriptor::from_sddl("S:(AU;SA;GR;;;WD)").expect("SACL"),
            ),
        },
    ]);
    assert_eq!(
        recovery::information(
            &manifest,
            &Change::SetFolderSecurity(manifest.namespace.clone())
        ),
        base
    );
    assert_eq!(
        recovery::information(&manifest, &Change::SetFolderSecurity(child)),
        SecurityInformation::all()
    );
}

#[test]
fn missing_backup_data_requires_explicit_irreversible_permission() {
    for folder in [false, true] {
        for allow in [false, true] {
            let (backend, mut manifest) = fixture(1);
            apply_fixture(&backend, &manifest);
            let change = if folder {
                manifest.folders.push(crate::manifest::ManagedFolder {
                    path: manifest.namespace.clone(),
                    security_descriptor: Some(acl()),
                });
                Change::SetFolderSecurity(manifest.namespace.clone())
            } else {
                manifest.tasks[0]
                    .definition
                    .registration
                    .security_descriptor = Some(acl());
                Change::SetTaskSecurity(manifest.tasks[0].path.clone())
            };
            let plan = Plan {
                changes: vec![PlannedChange {
                    change,
                    rollback: RollbackSafety::Reversible,
                }],
                irreversible_effects: Vec::new(),
            };
            backend.fail(
                if folder {
                    "folder_security"
                } else {
                    "task_security"
                },
                1,
                false,
            );
            match prepare(&backend, &manifest, &plan, allow, &mut NoCredentials) {
                Ok(prepared) => {
                    assert!(allow);
                    assert!(prepared.task_security.is_empty());
                    assert!(prepared.folder_security.is_empty());
                }
                Err(error) => {
                    assert!(!allow);
                    assert_eq!(error.kind(), ErrorKind::AccessDenied);
                }
            }
        }
    }
}

#[test]
fn task_backup_credentials_and_security_require_explicit_irreversible_permission() {
    for password_failure in [false, true] {
        for allow in [false, true] {
            let (backend, mut manifest) = fixture(1);
            apply_fixture(&backend, &manifest);
            if password_failure {
                backend
                    .state
                    .borrow_mut()
                    .tasks
                    .get_mut(&manifest.tasks[0].path)
                    .expect("task")
                    .principal
                    .logon_type = LogonType::Password;
                backend
                    .state
                    .borrow_mut()
                    .tasks
                    .get_mut(&manifest.tasks[0].path)
                    .expect("task")
                    .principal
                    .identity = crate::model::PrincipalIdentity::User("fixture-user".into());
            } else {
                backend.fail("task_security", 1, false);
            }
            manifest.tasks[0].definition.registration.description = Some("update".into());
            let plan = Plan {
                changes: vec![PlannedChange {
                    change: Change::UpdateTask(manifest.tasks[0].path.clone()),
                    rollback: RollbackSafety::Reversible,
                }],
                irreversible_effects: Vec::new(),
            };
            match prepare(&backend, &manifest, &plan, allow, &mut NoCredentials) {
                Ok(prepared) => {
                    assert!(allow);
                    let backup = &prepared.task_backups[&manifest.tasks[0].path];
                    if password_failure {
                        assert!(backup.password.is_none());
                    } else {
                        assert!(backup.security.is_none());
                    }
                }
                Err(error) => {
                    assert!(!allow);
                    assert_eq!(
                        error.kind(),
                        if password_failure {
                            ErrorKind::Irreversible
                        } else {
                            ErrorKind::AccessDenied
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn conditional_ace_text_does_not_request_unrelated_security_sections() {
    for expression in [
        "S:O:G:",
        "))S:O:G:",
        "日本語 S:",
        r#"\"))S:O:G:"#,
        r"\\))S:O:G:",
    ] {
        let text = format!("D:(XA;;GR;;;WD;(@User.label == \"{expression}\"))");
        let descriptor = SecurityDescriptor::from_sddl(text).expect("conditional SDDL");
        assert_eq!(
            security_information_for_sddl(&descriptor),
            SecurityInformation::DACL
        );
    }
}

#[test]
fn stopping_a_new_task_during_compensation_is_also_irreversible() {
    let (backend, manifest) = fixture(2);
    backend.fail("register", 2, false);
    let failure = apply_backend(
        &backend,
        &manifest,
        ApplyOptions {
            stop_running: true,
            ..ApplyOptions::default()
        },
        &mut NoCredentials,
    )
    .expect_err("second registration fails");
    assert!(backend.state.borrow().tasks.is_empty());
    assert_eq!(failure.report.rolled_back.len(), 1);
    assert_eq!(
        failure.report.irreversible_effects,
        [manifest.tasks[0].path.clone()]
    );
    assert!(
        !failure.report.rollback_complete(),
        "stopped instances cannot be restored by deleting a definition"
    );
}

#[test]
fn sequential_changes_to_one_target_share_preconditions_and_reverse_in_order() {
    for fail_after_changes in [false, true] {
        let (backend, mut manifest) = fixture(2);
        apply_fixture(&backend, &manifest);
        let before = backend.state.borrow().clone();
        manifest.tasks[0].definition.settings.enabled = false;
        manifest.tasks[0]
            .definition
            .registration
            .security_descriptor =
            Some(SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("new task ACL"));
        let folder = manifest.namespace.join("New").expect("child folder");
        manifest.folders.push(crate::manifest::ManagedFolder {
            path: folder.clone(),
            security_descriptor: Some(acl()),
        });
        // The final prune failure occurs after both task changes and both folder changes.
        manifest.tasks.truncate(1);
        if fail_after_changes {
            backend.fail("delete_task", 1, false);
        }
        let result = apply_backend(
            &backend,
            &manifest,
            ApplyOptions {
                prune: true,
                ..ApplyOptions::default()
            },
            &mut NoCredentials,
        );
        if fail_after_changes {
            let failure = result.expect_err("final prune is denied");
            assert!(
                failure.report.rollback_complete(),
                "{:?}",
                failure.report.rollback_failures
            );
            assert_eq!(*backend.state.borrow(), before);
        } else {
            let report = result.expect("multiple changes on same target");
            assert!(report.succeeded());
            assert_eq!(report.applied.len(), 5);
            assert!(backend.state.borrow().folders.contains_key(&folder));
            assert!(
                !backend.state.borrow().tasks[&manifest.tasks[0].path]
                    .settings
                    .enabled
            );
            assert!(
                apply_backend(
                    &backend,
                    &manifest,
                    ApplyOptions {
                        prune: true,
                        ..ApplyOptions::default()
                    },
                    &mut NoCredentials
                )
                .expect("second apply")
                .plan
                .is_empty()
            );
        }
    }
}

#[test]
fn native_registration_contract_preserves_credentials_flags_and_compensation_mode() {
    for suppress in [false, true] {
        let (backend, mut manifest) = fixture(1);
        manifest.tasks[0].credentials.registration = Some("fixture-reference".into());
        manifest.tasks[0].definition.principal.identity =
            crate::model::PrincipalIdentity::User("fixture-user".into());
        manifest.tasks[0].definition.principal.logon_type = LogonType::Password;
        manifest.tasks[0]
            .definition
            .registration
            .security_descriptor = Some(acl());
        let options = ApplyOptions {
            ignore_registration_triggers: suppress,
            ..ApplyOptions::default()
        };
        let mut requests = Vec::new();
        let mut resolver = |_: &TaskPath, reference: Option<&str>, purpose: CredentialPurpose| {
            assert_eq!(reference, Some("fixture-reference"));
            requests.push(purpose);
            Ok(Password::new("controlled-test-password"))
        };
        apply_backend(&backend, &manifest, options, &mut resolver).expect("initial registration");
        {
            let records = backend.registrations.borrow();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].mode, RegistrationMode::Create);
            assert_eq!(records[0].suppress_triggers, suppress);
            assert!(records[0].has_password);
            assert!(records[0].preserve_acl);
            assert!(!records[0].raw)
        };
        let before = backend.state.borrow().clone();
        backend.registrations.borrow_mut().clear();
        backend.calls.borrow_mut().clear();
        manifest.tasks[0].definition.registration.description = Some("update".into());
        backend.fail("register", 1, true);
        let failure = apply_backend(&backend, &manifest, options, &mut resolver)
            .expect_err("lost update reply");
        assert!(failure.report.rollback_complete());
        assert_eq!(*backend.state.borrow(), before);
        assert_eq!(
            requests
                .iter()
                .filter(|purpose| **purpose == CredentialPurpose::Rollback)
                .count(),
            1
        );
        let records = backend.registrations.borrow();
        assert_eq!(records.len(), 2);
        for record in records.iter() {
            assert_eq!(record.mode, RegistrationMode::CreateOrUpdate);
            assert_eq!(record.suppress_triggers, suppress);
            assert!(
                record.has_password,
                "both desired and rollback passwords reach native registration"
            );
        }
        assert!(!records[0].raw);
        assert!(records[1].raw);
    }
}

#[test]
fn reports_distinguish_each_incomplete_state() {
    let (backend, manifest) = fixture(1);
    let success = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("successful fixture");
    let change = success.applied[0].clone();
    for variant in 0..4 {
        let mut report = success.clone();
        match variant {
            0 => {
                report.applied.clear();
            }
            1 => {
                report.rolled_back.push(change.clone());
            }
            2 => report.rollback_failures.push(RollbackFailure {
                change: change.clone(),
                phase: ApplyPhase::Rollback,
                error: Error::new(ErrorKind::AccessDenied, "denied").with_native_code(5),
            }),
            _ => report.unresolved.push(change.clone()),
        }
        assert!(!report.succeeded(), "incomplete report variant {variant}");
    }
    let cause = Error::new(ErrorKind::Conflict, "changed externally");
    let mut failure = ApplyFailure {
        cause: cause.clone(),
        report: success,
    };
    assert_eq!(failure.to_string(), cause.to_string());
    assert_eq!(
        std::error::Error::source(&failure)
            .expect("source")
            .to_string(),
        cause.to_string()
    );
    failure.report.rollback_failures.push(RollbackFailure {
        change,
        phase: ApplyPhase::Rollback,
        error: cause,
    });
    assert!(
        failure
            .to_string()
            .ends_with("; 1 rollback step(s) also failed")
    );
}

#[test]
fn stop_plan_marks_only_existing_task_replacements() {
    let (_, manifest) = fixture(1);
    let path = manifest.tasks[0].path.clone();
    let changes = vec![
        Change::CreateFolder(manifest.namespace.clone()),
        Change::SetFolderSecurity(manifest.namespace),
        Change::CreateTask(path.clone()),
        Change::UpdateTask(path.clone()),
        Change::AdoptTask(path.clone()),
        Change::DeleteTask(path.clone()),
        Change::SetEnabled {
            path: path.clone(),
            enabled: false,
        },
        Change::SetTaskSecurity(path.clone()),
    ];
    let plan = Plan {
        changes: changes
            .into_iter()
            .map(|change| PlannedChange {
                change,
                rollback: RollbackSafety::Reversible,
            })
            .collect(),
        irreversible_effects: Vec::new(),
    };
    assert!(!plan.contains_irreversible_changes());
    let stopped = plan.clone().with_stop_running();
    assert_eq!(
        stopped.irreversible_effects,
        [path.clone(), path.clone(), path]
    );
    assert!(stopped.contains_irreversible_changes());
    let mut unsafe_plan = plan;
    unsafe_plan.changes[0].rollback = RollbackSafety::RequiresCredential;
    assert!(unsafe_plan.contains_irreversible_changes());
    assert!(!Plan::default().contains_irreversible_changes());
}

#[test]
fn inspection_errors_and_invalid_manifests_never_become_empty_successes() {
    let (backend, manifest) = fixture(1);
    backend.fail("list_tasks", 1, false);
    assert_eq!(
        inspect_backend(&backend, &manifest)
            .expect_err("denied listing")
            .kind(),
        ErrorKind::AccessDenied
    );
    let mut invalid = manifest.clone();
    invalid.tasks.push(invalid.tasks[0].clone());
    let before = backend.calls.borrow().len();
    assert_eq!(
        plan(&invalid, &[], false)
            .expect_err("duplicate task")
            .kind(),
        ErrorKind::InvalidDefinition
    );
    apply_backend(
        &backend,
        &invalid,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect_err("invalid manifest");
    assert_eq!(
        backend.calls.borrow().len(),
        before,
        "validation precedes backend calls"
    );
    assert!(
        !plan(&manifest, &[], false)
            .expect("offline plan")
            .is_empty()
    );
}

#[test]
fn pruning_and_adoption_respect_owned_and_desired_sets_independently() {
    let (backend, manifest) = fixture(2);
    apply_fixture(&backend, &manifest);
    let mut current = inspect_backend(&backend, &manifest).expect("snapshot");
    current.tasks[1].owned = false;
    let mut desired = manifest.clone();
    desired.tasks.truncate(1);
    assert!(
        plan_state(
            &desired,
            &current,
            PlanOptions {
                prune: true,
                adopt: false
            }
        )
        .expect("foreign extra task is retained")
        .is_empty()
    );
    current.tasks[1].owned = true;
    assert_eq!(
        plan_state(
            &desired,
            &current,
            PlanOptions {
                prune: true,
                adopt: false
            }
        )
        .expect("owned extra task is pruned")
        .changes[0]
            .change,
        Change::DeleteTask(manifest.tasks[1].path.clone())
    );
    current.tasks[0].owned = false;
    assert_eq!(
        plan_state(
            &desired,
            &current,
            PlanOptions {
                prune: false,
                adopt: true
            }
        )
        .expect("explicit adoption")
        .changes[0]
            .change,
        Change::AdoptTask(manifest.tasks[0].path.clone())
    );
}

#[test]
fn observations_keep_denied_unknown_missing_and_present_distinct() {
    use recovery::{Observed, observe};
    for folder in [false, true] {
        for allow in [false, true] {
            let (backend, manifest) = fixture(1);
            apply_fixture(&backend, &manifest);
            let change = if folder {
                Change::SetFolderSecurity(manifest.namespace.clone())
            } else {
                Change::UpdateTask(manifest.tasks[0].path.clone())
            };
            backend.fail(
                if folder {
                    "folder_security"
                } else {
                    "task_security"
                },
                1,
                false,
            );
            let result = observe(&backend, &manifest, &change, allow);
            if allow {
                assert!(matches!(
                    result.expect("explicit unknown ACL allowance"),
                    Observed::Folder(None) | Observed::Task { security: None, .. }
                ));
            } else {
                assert_eq!(
                    result.expect_err("ACL denial").kind(),
                    ErrorKind::AccessDenied
                );
            }
        }
    }
    let (backend, manifest) = fixture(1);
    backend.fail("get_task", 1, false);
    let change = Change::CreateTask(manifest.tasks[0].path.clone());
    assert_eq!(
        observe(&backend, &manifest, &change, true)
            .expect_err("denied is not missing")
            .kind(),
        ErrorKind::AccessDenied
    );
    assert_eq!(
        observe(&backend, &manifest, &change, false).expect("absent task"),
        Observed::Missing
    );
    assert_eq!(Observed::Folder(None), Observed::Folder(None));
    assert_ne!(Observed::Folder(None), Observed::Folder(Some(acl())));
}

#[test]
fn response_loss_verification_rejects_unrelated_definition_enabled_and_acl_drift() {
    use recovery::{Observed, expected_after, observe};
    let (backend, mut manifest) = fixture(1);
    manifest.tasks[0]
        .definition
        .registration
        .security_descriptor = Some(acl());
    apply_fixture(&backend, &manifest);
    let path = manifest.tasks[0].path.clone();
    let change = Change::UpdateTask(path.clone());
    let before = observe(&backend, &manifest, &change, false).expect("snapshot");
    assert!(expected_after(
        &backend, &before, &before, &manifest, &change
    ));
    for variant in 0..3 {
        let mut after = before.clone();
        let Observed::Task {
            definition,
            enabled,
            ..
        } = &mut after
        else {
            panic!("task snapshot")
        };
        match variant {
            0 => definition.registration.description = Some("foreign edit".into()),
            1 => *enabled = !*enabled,
            _ => {
                backend
                    .state
                    .borrow_mut()
                    .tasks
                    .get_mut(&path)
                    .expect("task")
                    .registration
                    .security_descriptor =
                    Some(SecurityDescriptor::from_sddl("D:(A;;GR;;;SY)").expect("different access"))
            }
        }
        assert!(
            !expected_after(&backend, &before, &after, &manifest, &change),
            "drift variant {variant}"
        );
    }
    assert!(!expected_after(
        &backend,
        &before,
        &Observed::Missing,
        &manifest,
        &change
    ));
}

#[test]
fn journal_records_preconditions_and_exact_security_sections() {
    let (backend, mut manifest) = fixture(1);
    let descriptor = SecurityDescriptor::from_sddl("O:SYG:BAD:(A;;GA;;;SY)S:(AU;SA;GR;;;WD)")
        .expect("all sections");
    assert_eq!(
        security_information_for_sddl(&descriptor),
        SecurityInformation::all()
    );
    manifest.tasks[0]
        .definition
        .registration
        .security_descriptor = Some(descriptor);
    assert_eq!(
        recovery::information(
            &manifest,
            &Change::UpdateTask(manifest.tasks[0].path.clone())
        ),
        SecurityInformation::all()
    );
    let report = apply_backend(
        &backend,
        &manifest,
        ApplyOptions::default(),
        &mut NoCredentials,
    )
    .expect("apply");
    for (phase, outcome) in [
        (ApplyPhase::Precondition, StepOutcome::Succeeded),
        (ApplyPhase::Mutation, StepOutcome::Attempted),
    ] {
        assert!(
            report
                .journal
                .iter()
                .any(|entry| entry.phase == phase && entry.outcome == outcome)
        );
    }
}

//! Fluent assembly for [`TaskDefinition`].

use super::{Action, Actions, Principal, TaskDefinition, TaskSettings, Trigger, Triggers};
use crate::{Error, ErrorKind, Result};

/// Assembles a [`TaskDefinition`] that is portable-valid by construction.
///
/// The builder owns no scheduling rules of its own: [`TaskBuilder::build`]
/// applies the native collection limits and then the same
/// [`TaskDefinition::validate`] used everywhere else, so a definition that
/// builds is a definition that validates.
///
/// ```
/// use windows_task::model::{
///     DailyTrigger, ExecAction, Principal, ServiceAccount, TaskDateTime, TaskDefinition,
/// };
///
/// let first = TaskDateTime::wall_clock(2026, 9, 5, 3, 30, 0)?;
/// let definition = TaskDefinition::builder(ExecAction::new(r"C:\Acme\backup.exe"))
///     .description("Nightly backup")
///     .run_as(Principal::service_account(ServiceAccount::LocalSystem))
///     .trigger(DailyTrigger::new(first))
///     .build()?;
///
/// assert_eq!(definition.actions.len(), 1);
/// # Ok::<(), windows_task::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct TaskBuilder {
    definition: TaskDefinition,
    actions: Vec<Action>,
    triggers: Vec<Trigger>,
}

impl TaskBuilder {
    pub(super) fn new(action: Action) -> Self {
        Self {
            definition: TaskDefinition::default(),
            actions: vec![action],
            triggers: Vec::new(),
        }
    }

    /// Appends one action. Actions run in the order they are added.
    #[must_use]
    pub fn action(mut self, action: impl Into<Action>) -> Self {
        self.actions.push(action.into());
        self
    }

    /// Appends one trigger.
    #[must_use]
    pub fn trigger(mut self, trigger: impl Into<Trigger>) -> Self {
        self.triggers.push(trigger.into());
        self
    }

    /// Replaces the security principal. The [`Principal`] constructors pair an
    /// identity with the logon type that matches it.
    #[must_use]
    pub fn run_as(mut self, principal: Principal) -> Self {
        self.definition.principal = principal;
        self
    }

    /// Replaces the runtime settings.
    #[must_use]
    pub fn settings(mut self, settings: TaskSettings) -> Self {
        self.definition.settings = settings;
        self
    }

    /// Sets the description shown by Task Scheduler.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.definition.registration.description = Some(description.into());
        self
    }

    /// Sets the author recorded in the registration metadata.
    #[must_use]
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.definition.registration.author = Some(author.into());
        self
    }

    /// Applies the native action/trigger limits and portable validation.
    ///
    /// Warnings do not fail the build; call [`TaskDefinition::validate`] on the
    /// result to inspect them. Errors are returned with every diagnostic
    /// retained on [`Error::diagnostics`].
    pub fn build(self) -> Result<TaskDefinition> {
        let definition = TaskDefinition {
            actions: Actions::try_from(self.actions)?,
            triggers: Triggers::try_from(self.triggers)?,
            ..self.definition
        };
        let report = definition.validate();
        if report.is_valid() {
            return Ok(definition);
        }
        let message = report.errors().next().map_or_else(
            || "the task definition is not valid".to_owned(),
            |first| format!("{}: {}", first.path, first.message),
        );
        Err(Error::new(ErrorKind::InvalidDefinition, message)
            .with_operation("TaskBuilder::build")
            .with_validation(report))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DiagnosticCode, ErrorKind,
        model::{
            Action, DailyTrigger, EmailAction, ExecAction, LogonTrigger, Principal, RunLevel,
            ServiceAccount, TaskDateTime, TaskDefinition, Trigger, WeeklyTrigger,
        },
    };

    fn anchor() -> TaskDateTime {
        TaskDateTime::wall_clock(2026, 9, 5, 6, 0, 0).expect("fixed anchor")
    }

    #[test]
    fn a_built_definition_passes_the_same_validation_it_was_checked_against() {
        let definition = TaskDefinition::builder(ExecAction::new("agent.exe").args(["--once"]))
            .description("fixture")
            .author("windows-task")
            .run_as(
                Principal::service_account(ServiceAccount::LocalSystem)
                    .run_level(RunLevel::HighestAvailable),
            )
            .trigger(DailyTrigger::new(anchor()))
            .trigger(LogonTrigger::new())
            .action(ExecAction::new("agent.exe").args(["--verify"]))
            .build()
            .expect("valid definition");

        assert!(definition.validate().is_valid());
        assert_eq!(definition.actions.len(), 2);
        assert_eq!(definition.triggers.len(), 2);
        assert_eq!(
            definition.registration.description.as_deref(),
            Some("fixture")
        );
        // Every trigger reaching a definition through the builder schedules.
        assert!(
            definition
                .triggers
                .as_slice()
                .iter()
                .filter_map(Trigger::common)
                .all(|common| common.enabled)
        );
    }

    #[test]
    fn build_rejects_invalid_input_and_keeps_every_diagnostic() {
        let error = TaskDefinition::builder(ExecAction::new(""))
            .trigger(WeeklyTrigger::new(anchor(), []))
            .build()
            .expect_err("empty command and empty weekday selection");

        assert_eq!(error.kind(), ErrorKind::InvalidDefinition);
        let codes: Vec<_> = error
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect();
        assert!(codes.contains(&DiagnosticCode::EmptyValue), "{codes:?}");
        assert!(
            codes.contains(&DiagnosticCode::IncompleteTrigger),
            "{codes:?}"
        );
        // The message names a concrete field rather than restating the kind.
        assert!(error.to_string().contains("actions[0].command"), "{error}");
    }

    #[test]
    fn warnings_do_not_fail_a_build() {
        let definition = TaskDefinition::builder(Action::Email(EmailAction::default()))
            .build()
            .expect("a deprecated action is a warning, not an error");
        let report = definition.validate();
        assert!(report.is_valid());
        assert_eq!(report.warnings().count(), 1);
    }

    #[test]
    fn action_and_trigger_limits_surface_as_classified_errors() {
        let mut builder = TaskDefinition::builder(ExecAction::new("agent.exe"));
        for _ in 0..crate::MAX_ACTIONS {
            builder = builder.action(ExecAction::new("agent.exe"));
        }
        assert_eq!(
            builder.build().expect_err("action limit").kind(),
            ErrorKind::InvalidDefinition
        );

        let mut builder = TaskDefinition::builder(ExecAction::new("agent.exe"));
        for _ in 0..=crate::MAX_TRIGGERS {
            builder = builder.trigger(DailyTrigger::new(anchor()));
        }
        assert_eq!(
            builder.build().expect_err("trigger limit").kind(),
            ErrorKind::InvalidDefinition
        );
    }

    #[test]
    fn every_principal_constructor_agrees_with_its_logon_type() {
        let principals = [
            Principal::service_account(ServiceAccount::LocalSystem),
            Principal::service_account(ServiceAccount::LocalService),
            Principal::service_account(ServiceAccount::NetworkService),
            Principal::user("ACME\\operator"),
            Principal::user_with_password("ACME\\operator"),
            Principal::s4u("ACME\\operator"),
            Principal::group("BUILTIN\\Administrators"),
            Principal::default(),
        ];
        for principal in principals {
            let definition = TaskDefinition::builder(ExecAction::new("agent.exe"))
                .run_as(principal.clone())
                .build()
                .expect("a constructed principal is internally consistent");
            assert!(
                !definition.validate().diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == DiagnosticCode::PrincipalLogonMismatch
                }),
                "{principal:?}"
            );
        }
    }
}

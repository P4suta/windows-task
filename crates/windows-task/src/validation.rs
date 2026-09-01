use std::collections::BTreeSet;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::model::{
    Action, LogonType, PrincipalIdentity, TaskDefinition, TaskSchemaVersion, Trigger, duplicate_ids,
};

/// Severity of a validation or diagnostic finding.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum DiagnosticLevel {
    /// Informational context.
    Info,
    /// The operation is legal but surprising, deprecated, or risky.
    Warning,
    /// The requested operation cannot be performed correctly.
    Error,
}

/// Stable machine-readable diagnostic identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// At least one action is required.
    MissingAction,
    /// Two known elements use the same identifier.
    DuplicateId,
    /// A required string is empty.
    EmptyValue,
    /// A numeric value is outside the Task Scheduler range.
    OutOfRange,
    /// A trigger lacks a required boundary or selection.
    IncompleteTrigger,
    /// The configured identity and logon type disagree.
    PrincipalLogonMismatch,
    /// A password must be supplied separately for this operation.
    PasswordRequired,
    /// A legacy action can be preserved but not registered on modern engines.
    DeprecatedAction,
    /// The requested XML schema is too old for a selected field.
    SchemaTooOld,
    /// Updating a task can fire its registration trigger.
    RegistrationTriggerSideEffect,
    /// An opaque extension cannot be validated semantically.
    OpaqueExtension,
    /// An OS capability is missing.
    UnsupportedCapability,
    /// Task history is disabled or inaccessible.
    HistoryUnavailable,
    /// RPC or DCOM connectivity failed.
    RemoteConnectivity,
    /// The caller lacks a required right.
    InsufficientRights,
    /// Current ownership metadata conflicts with desired state.
    OwnershipConflict,
    /// A rollback would require a password that cannot be recovered.
    IrreversibleChange,
    /// Catch-all for forward-compatible diagnostic producers.
    Other(String),
}

/// One validation or environment diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct Diagnostic {
    /// Severity.
    pub level: DiagnosticLevel,
    /// Stable code.
    pub code: DiagnosticCode,
    /// Manifest/model path or environment check name.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
    /// Concrete remediation when one is known.
    pub remediation: Option<String>,
}

impl Diagnostic {
    fn error(code: DiagnosticCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            code,
            path: path.into(),
            message: message.into(),
            remediation: None,
        }
    }

    fn warning(code: DiagnosticCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Warning,
            code,
            path: path.into(),
            message: message.into(),
            remediation: None,
        }
    }

    /// Attaches a remediation hint.
    #[must_use]
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }
}

/// Portable validation result for a task or desired-state document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ValidationReport {
    /// Findings in deterministic model order.
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    /// Whether no errors were found.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    }

    /// Iterates error findings only.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    }

    /// Iterates warning findings only.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Warning)
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }
}

/// Read-only environment and connectivity diagnostics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct DiagnosticReport {
    /// Computer or endpoint that was inspected.
    pub target: Option<String>,
    /// Findings in check order.
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    /// Whether every required check succeeded.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    }
}

pub(crate) fn validate_definition(definition: &TaskDefinition) -> ValidationReport {
    let mut report = ValidationReport::default();
    if definition.actions.is_empty() {
        report.push(Diagnostic::error(
            DiagnosticCode::MissingAction,
            "actions",
            "a task must contain at least one action",
        ));
    }

    for duplicate in duplicate_ids(
        definition
            .actions
            .as_slice()
            .iter()
            .filter_map(Action::id)
            .map(str::to_owned),
    ) {
        report.push(Diagnostic::error(
            DiagnosticCode::DuplicateId,
            "actions",
            format!("duplicate action id {duplicate:?}"),
        ));
    }
    for duplicate in duplicate_ids(
        definition
            .triggers
            .as_slice()
            .iter()
            .filter_map(Trigger::common)
            .filter_map(|value| value.id.clone()),
    ) {
        report.push(Diagnostic::error(
            DiagnosticCode::DuplicateId,
            "triggers",
            format!("duplicate trigger id {duplicate:?}"),
        ));
    }

    if definition.settings.priority > 10 {
        report.push(Diagnostic::error(
            DiagnosticCode::OutOfRange,
            "settings.priority",
            "scheduler priority must be between 0 and 10",
        ));
    }
    validate_principal(definition, &mut report);
    validate_actions(definition, &mut report);
    validate_triggers(definition, &mut report);
    validate_schema(definition, &mut report);

    if !definition.extensions.is_empty() {
        report.push(Diagnostic::warning(
            DiagnosticCode::OpaqueExtension,
            "extensions",
            "opaque XML extensions are preserved but cannot be validated semantically",
        ));
    }
    report
}

fn validate_principal(definition: &TaskDefinition, report: &mut ValidationReport) {
    let principal = &definition.principal;
    if principal.id.is_empty() {
        report.push(Diagnostic::error(
            DiagnosticCode::EmptyValue,
            "principal.id",
            "principal id cannot be empty",
        ));
    }
    let identity_matches = match principal.logon_type {
        LogonType::None => true,
        LogonType::Password
        | LogonType::InteractiveToken
        | LogonType::S4u
        | LogonType::InteractiveTokenOrPassword => {
            matches!(principal.identity, PrincipalIdentity::User(_))
        }
        LogonType::Group => matches!(principal.identity, PrincipalIdentity::Group(_)),
        LogonType::ServiceAccount => {
            matches!(principal.identity, PrincipalIdentity::ServiceAccount(_))
        }
    };
    if !identity_matches {
        report.push(Diagnostic::error(
            DiagnosticCode::PrincipalLogonMismatch,
            "principal",
            "principal identity does not match its logon type",
        ));
    }

    let mut privileges = BTreeSet::new();
    for (index, privilege) in principal.required_privileges.iter().enumerate() {
        if !privileges.insert(privilege) {
            report.push(Diagnostic::error(
                DiagnosticCode::DuplicateId,
                format!("principal.required_privileges[{index}]"),
                format!("duplicate privilege {:?}", privilege.as_str()),
            ));
        }
    }
}

fn validate_actions(definition: &TaskDefinition, report: &mut ValidationReport) {
    for (index, action) in definition.actions.as_slice().iter().enumerate() {
        match action {
            Action::Exec(action) if action.command.is_empty() => report.push(Diagnostic::error(
                DiagnosticCode::EmptyValue,
                format!("actions[{index}].command"),
                "an exec command cannot be empty",
            )),
            Action::Email(_) | Action::ShowMessage(_) => report.push(
                Diagnostic::warning(
                    DiagnosticCode::DeprecatedAction,
                    format!("actions[{index}]"),
                    "Email and ShowMessage actions are retained for round trips but unsupported by modern Windows",
                )
                .with_remediation("replace the legacy action with Exec or ComHandler"),
            ),
            Action::Unknown(_) => report.push(Diagnostic::warning(
                DiagnosticCode::OpaqueExtension,
                format!("actions[{index}]"),
                "an unknown action cannot be validated semantically",
            )),
            Action::Exec(_) | Action::ComHandler(_) => {}
        }
    }
}

fn validate_triggers(definition: &TaskDefinition, report: &mut ValidationReport) {
    for (index, trigger) in definition.triggers.as_slice().iter().enumerate() {
        let path = format!("triggers[{index}]");
        if let Some(common) = trigger.common() {
            if common
                .repetition
                .as_ref()
                .is_some_and(|repetition| repetition.interval.as_std().is_zero())
            {
                report.push(Diagnostic::error(
                    DiagnosticCode::OutOfRange,
                    format!("{path}.repetition.interval"),
                    "a repetition interval must be greater than zero",
                ));
            }
        }
        match trigger {
            Trigger::Time(value) if value.common.start_boundary.is_none() => {
                report.push(Diagnostic::error(
                    DiagnosticCode::IncompleteTrigger,
                    format!("{path}.start_boundary"),
                    "a time trigger requires a start boundary",
                ))
            }
            Trigger::Daily(value) if value.days_interval == 0 => report.push(Diagnostic::error(
                DiagnosticCode::OutOfRange,
                format!("{path}.days_interval"),
                "daily interval must be at least one",
            )),
            Trigger::Weekly(value)
                if value.weeks_interval == 0 || value.days_of_week.is_empty() =>
            {
                report.push(Diagnostic::error(
                    DiagnosticCode::IncompleteTrigger,
                    path,
                    "a weekly trigger requires a non-zero interval and at least one weekday",
                ));
            }
            Trigger::Monthly(value)
                if value.months.is_empty()
                    || (value.days_of_month.is_empty() && !value.run_on_last_day)
                    || value
                        .days_of_month
                        .iter()
                        .any(|day| !(1..=31).contains(day)) =>
            {
                report.push(Diagnostic::error(
                    DiagnosticCode::IncompleteTrigger,
                    path,
                    "a monthly trigger needs valid days and at least one month",
                ));
            }
            Trigger::MonthlyDow(value)
                if value.months.is_empty()
                    || value.days_of_week.is_empty()
                    || (value.weeks_of_month.is_empty() && !value.run_on_last_week) =>
            {
                report.push(Diagnostic::error(
                    DiagnosticCode::IncompleteTrigger,
                    path,
                    "a monthly day-of-week trigger needs months, weekdays, and week ordinals",
                ));
            }
            Trigger::Event(value) if value.subscription.is_empty() => {
                report.push(Diagnostic::error(
                    DiagnosticCode::EmptyValue,
                    format!("{path}.subscription"),
                    "an event trigger requires an XPath subscription",
                ))
            }
            Trigger::Registration(_) => report.push(
                Diagnostic::warning(
                    DiagnosticCode::RegistrationTriggerSideEffect,
                    path,
                    "registering or updating this task can execute it",
                )
                .with_remediation(
                    "keep ignore_registration_triggers enabled during reconciliation",
                ),
            ),
            Trigger::Unknown(_) => report.push(Diagnostic::warning(
                DiagnosticCode::OpaqueExtension,
                path,
                "an unknown trigger cannot be validated semantically",
            )),
            Trigger::Boot(_)
            | Trigger::Idle(_)
            | Trigger::Time(_)
            | Trigger::Event(_)
            | Trigger::Logon(_)
            | Trigger::SessionStateChange(_)
            | Trigger::Daily(_)
            | Trigger::Weekly(_)
            | Trigger::Monthly(_)
            | Trigger::MonthlyDow(_) => {}
        }
    }
}

fn validate_schema(definition: &TaskDefinition, report: &mut ValidationReport) {
    let generation = match definition.schema_version {
        TaskSchemaVersion::V1_2 => 2,
        TaskSchemaVersion::V1_3 => 3,
        TaskSchemaVersion::V1_4 => 4,
        TaskSchemaVersion::V1_5 => 5,
        TaskSchemaVersion::V1_6 | TaskSchemaVersion::Unknown(_) => 6,
    };
    if definition.settings.maintenance.is_some() && generation < 4 {
        report.push(Diagnostic::error(
            DiagnosticCode::SchemaTooOld,
            "settings.maintenance",
            "maintenance settings require task schema 1.4 or later",
        ));
    }
    if definition.settings.volatile && generation < 6 {
        report.push(Diagnostic::error(
            DiagnosticCode::SchemaTooOld,
            "settings.volatile",
            "volatile tasks require task schema 1.6 or later",
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Action, ExecAction, LogonType, PrincipalIdentity, TaskDefinition};

    #[test]
    fn reports_missing_action() {
        assert!(!TaskDefinition::default().validate().is_valid());
    }

    #[test]
    fn accepts_minimal_exec_task() {
        let definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        assert!(definition.validate().is_valid());
    }

    #[test]
    fn rejects_principal_logon_mismatch() {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        definition.principal.logon_type = LogonType::Group;
        definition.principal.identity = PrincipalIdentity::User("someone".into());
        assert!(!definition.validate().is_valid());
    }
}

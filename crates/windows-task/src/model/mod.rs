//! Portable, fully owned Task Scheduler domain model.

mod action;
mod principal;
mod settings;
mod time;
mod trigger;

use std::collections::BTreeSet;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{MAX_ACTIONS, MAX_TRIGGERS, ValidationReport};

pub use action::{
    Action, ComHandlerAction, EmailAction, EmailHeader, ExecAction, ShowMessageAction,
    UnknownAction, expand_action_templates, quote_windows_argument,
};
pub use principal::{
    InvalidPrivilege, LogonType, Principal, PrincipalIdentity, ProcessTokenSidType,
    RequiredPrivilege, RunLevel, ServiceAccount,
};
pub use settings::{
    IdleSettings, MaintenanceSettings, MultipleInstancesPolicy, NetworkSettings, RestartPolicy,
    TaskSettings,
};
pub use time::{
    ParseTaskDateTimeError, ParseTaskDurationError, TaskDateTime, TaskDuration, TaskLimit,
    TimeBasis,
};
pub use trigger::{
    BootTrigger, DailyTrigger, EventTrigger, IdleTrigger, LogonTrigger, Month, MonthlyDowTrigger,
    MonthlyTrigger, RegistrationTrigger, Repetition, SessionStateChange, SessionStateChangeTrigger,
    TimeTrigger, Trigger, TriggerCommon, UnknownTrigger, WeekOfMonth, Weekday, WeeklyTrigger,
};

/// Task Scheduler XML schema generation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum TaskSchemaVersion {
    /// Task Scheduler 2.0 / schema 1.2.
    V1_2,
    /// Windows 7 additions / schema 1.3.
    V1_3,
    /// Windows 8 additions / schema 1.4.
    V1_4,
    /// Windows 10 additions / schema 1.5.
    V1_5,
    /// Current schema 1.6 additions.
    V1_6,
    /// A future schema retained by the raw XML layer.
    Unknown(String),
}

impl TaskSchemaVersion {
    /// Parses the value of the Task XML `version` attribute.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "1.2" => Self::V1_2,
            "1.3" => Self::V1_3,
            "1.4" => Self::V1_4,
            "1.5" => Self::V1_5,
            "1.6" => Self::V1_6,
            other => Self::Unknown(other.into()),
        }
    }

    /// Returns the Task XML version text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::V1_2 => "1.2",
            Self::V1_3 => "1.3",
            Self::V1_4 => "1.4",
            Self::V1_5 => "1.5",
            Self::V1_6 => "1.6",
            Self::Unknown(value) => value,
        }
    }
}

impl Default for TaskSchemaVersion {
    fn default() -> Self {
        Self::V1_6
    }
}

/// Registration metadata shown by Task Scheduler.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct RegistrationInfo {
    /// Author or owning application.
    pub author: Option<String>,
    /// Registration timestamp.
    pub date: Option<TaskDateTime>,
    /// Free-form description.
    pub description: Option<String>,
    /// Documentation URI or text.
    pub documentation: Option<String>,
    /// Source that produced this task.
    pub source: Option<String>,
    /// Version of the task definition.
    pub version: Option<String>,
    /// Registration URI. Windows can rewrite this to the task path; reconcile
    /// stores ownership in Source and accepts legacy URI markers when present.
    pub uri: Option<String>,
    /// Optional task security descriptor.
    pub security_descriptor: Option<SecurityDescriptor>,
}

/// Validated SDDL security descriptor with a lossless textual representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct SecurityDescriptor(String);

impl SecurityDescriptor {
    #[cfg(all(feature = "client", any(windows, feature = "reconcile", test)))]
    pub(crate) fn access_equivalent(&self, other: &Self) -> bool {
        fn key(value: &str) -> String {
            // Normalize only the Windows auto-inheritance status bit. Preserve
            // protection, requested inheritance, ACE flags and ACE ordering.
            // Conditional expressions are opaque and are compared verbatim.
            if value.contains(['\'', '"']) {
                return value.into();
            }
            value
                .replace("D:PAI(", "D:P(")
                .replace("D:AI(", "D:(")
                .replace("S:PAI(", "S:P(")
                .replace("S:AI(", "S:(")
        }
        key(self.as_sddl()) == key(other.as_sddl())
    }

    /// Creates an SDDL descriptor. Native semantic validation also occurs when
    /// the descriptor is applied on Windows.
    pub fn from_sddl(value: impl Into<String>) -> Result<Self, InvalidSecurityDescriptor> {
        let value = value.into();
        if value.is_empty() || value.contains('\0') {
            Err(InvalidSecurityDescriptor)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the SDDL representation.
    #[must_use]
    pub fn as_sddl(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SecurityDescriptor {
    type Error = InvalidSecurityDescriptor;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_sddl(value)
    }
}

impl From<SecurityDescriptor> for String {
    fn from(value: SecurityDescriptor) -> Self {
        value.0
    }
}

/// An empty or NUL-containing SDDL value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("a security descriptor must be non-empty SDDL without NUL")]
pub struct InvalidSecurityDescriptor;

/// Opaque XML retained at a stable extension point.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct XmlExtension {
    /// Slash-separated local-name path of the containing element.
    pub parent: String,
    /// Position among the containing element's children.
    pub ordinal: usize,
    /// Complete original XML element.
    pub xml: String,
}

/// Validated action collection enforcing Task Scheduler's 32-action limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "Vec<Action>", into = "Vec<Action>")
)]
pub struct Actions(Vec<Action>);

impl Actions {
    /// Creates a checked action collection.
    pub fn new(values: Vec<Action>) -> Result<Self, CollectionLimitError> {
        Self::try_from(values)
    }

    /// Adds one action if the collection remains within the native limit.
    pub fn push(&mut self, action: Action) -> Result<(), CollectionLimitError> {
        if self.0.len() == MAX_ACTIONS {
            return Err(CollectionLimitError::actions(self.0.len() + 1));
        }
        self.0.push(action);
        Ok(())
    }

    /// Returns all actions.
    #[must_use]
    pub fn as_slice(&self) -> &[Action] {
        &self.0
    }

    /// Returns whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of actions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl TryFrom<Vec<Action>> for Actions {
    type Error = CollectionLimitError;

    fn try_from(values: Vec<Action>) -> Result<Self, Self::Error> {
        if values.len() > MAX_ACTIONS {
            Err(CollectionLimitError::actions(values.len()))
        } else {
            Ok(Self(values))
        }
    }
}

impl From<Actions> for Vec<Action> {
    fn from(value: Actions) -> Self {
        value.0
    }
}

impl IntoIterator for Actions {
    type Item = Action;
    type IntoIter = std::vec::IntoIter<Action>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Validated trigger collection enforcing Task Scheduler's 48-trigger limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "Vec<Trigger>", into = "Vec<Trigger>")
)]
pub struct Triggers(Vec<Trigger>);

impl Triggers {
    /// Creates a checked trigger collection.
    pub fn new(values: Vec<Trigger>) -> Result<Self, CollectionLimitError> {
        Self::try_from(values)
    }

    /// Adds one trigger if the collection remains within the native limit.
    pub fn push(&mut self, trigger: Trigger) -> Result<(), CollectionLimitError> {
        if self.0.len() == MAX_TRIGGERS {
            return Err(CollectionLimitError::triggers(self.0.len() + 1));
        }
        self.0.push(trigger);
        Ok(())
    }

    /// Returns all triggers.
    #[must_use]
    pub fn as_slice(&self) -> &[Trigger] {
        &self.0
    }

    /// Returns whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of triggers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl TryFrom<Vec<Trigger>> for Triggers {
    type Error = CollectionLimitError;

    fn try_from(values: Vec<Trigger>) -> Result<Self, Self::Error> {
        if values.len() > MAX_TRIGGERS {
            Err(CollectionLimitError::triggers(values.len()))
        } else {
            Ok(Self(values))
        }
    }
}

impl From<Triggers> for Vec<Trigger> {
    fn from(value: Triggers) -> Self {
        value.0
    }
}

impl IntoIterator for Triggers {
    type Item = Trigger;
    type IntoIter = std::vec::IntoIter<Trigger>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A collection exceeded a Task Scheduler native maximum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{kind} contains {actual} entries; the maximum is {maximum}")]
pub struct CollectionLimitError {
    kind: &'static str,
    actual: usize,
    maximum: usize,
}

impl CollectionLimitError {
    fn actions(actual: usize) -> Self {
        Self {
            kind: "actions",
            actual,
            maximum: MAX_ACTIONS,
        }
    }

    fn triggers(actual: usize) -> Self {
        Self {
            kind: "triggers",
            actual,
            maximum: MAX_TRIGGERS,
        }
    }
}

/// Complete portable task definition.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct TaskDefinition {
    /// Task XML schema version.
    pub schema_version: TaskSchemaVersion,
    /// Registration metadata.
    pub registration: RegistrationInfo,
    /// Trigger collection.
    pub triggers: Triggers,
    /// Security principal.
    pub principal: Principal,
    /// Runtime settings.
    pub settings: TaskSettings,
    /// Opaque task data available to COM handlers.
    pub data: Option<String>,
    /// Ordered action collection.
    pub actions: Actions,
    /// Future-schema fragments retained by the XML layer.
    pub extensions: Vec<XmlExtension>,
}

impl TaskDefinition {
    /// Creates a definition containing one action and default settings.
    #[must_use]
    pub fn new(action: Action) -> Self {
        Self {
            actions: Actions(vec![action]),
            ..Self::default()
        }
    }

    /// Performs complete portable and cross-field validation.
    #[must_use]
    pub fn validate(&self) -> ValidationReport {
        crate::validation::validate_definition(self)
    }
}

impl Default for TaskDefinition {
    fn default() -> Self {
        Self {
            schema_version: TaskSchemaVersion::default(),
            registration: RegistrationInfo::default(),
            triggers: Triggers::default(),
            principal: Principal::default(),
            settings: TaskSettings::default(),
            data: None,
            actions: Actions::default(),
            extensions: Vec::new(),
        }
    }
}

pub(crate) fn duplicate_ids(values: impl Iterator<Item = String>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for id in values {
        if !seen.insert(id.clone()) {
            duplicates.insert(id);
        }
    }
    duplicates
}

#[cfg(all(test, feature = "client"))]
mod security_tests {
    use super::SecurityDescriptor;
    #[test]
    fn comparison_preserves_protection_rights_and_ace_order() {
        let descriptor = |text| SecurityDescriptor::from_sddl(text).expect("fixture SDDL");
        let expected = descriptor("D:P(A;;FA;;;SY)(D;;FR;;;BA)");
        assert!(expected.access_equivalent(&descriptor("D:PAI(A;;FA;;;SY)(D;;FR;;;BA)")));
        for other in [
            "D:(A;;FA;;;SY)(D;;FR;;;BA)",
            "D:P(D;;FR;;;BA)(A;;FA;;;SY)",
            "D:P(A;;FR;;;SY)(D;;FR;;;BA)",
        ] {
            assert!(!expected.access_equivalent(&descriptor(other)));
        }
    }
}

use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{TaskDateTime, TaskDuration, TaskLimit};

/// Settings shared by every trigger.
///
/// [`Default`] matches Task XML, where an absent `Enabled` element means the
/// trigger participates in scheduling. Struct update syntax is therefore safe:
/// `TriggerCommon { start_boundary: Some(at), ..TriggerCommon::default() }`
/// produces an enabled trigger, exactly like the equivalent Task XML.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct TriggerCommon {
    /// Optional identifier, unique within the task.
    pub id: Option<String>,
    /// First boundary at which the trigger can fire.
    pub start_boundary: Option<TaskDateTime>,
    /// Last boundary at which the trigger can fire.
    pub end_boundary: Option<TaskDateTime>,
    /// Whether the trigger participates in scheduling.
    pub enabled: bool,
    /// Per-trigger execution time limit.
    pub execution_time_limit: Option<TaskLimit>,
    /// Optional repeated execution after each firing.
    pub repetition: Option<Repetition>,
}

impl Default for TriggerCommon {
    /// Mirrors the Task XML schema: an omitted `Enabled` element is enabled.
    fn default() -> Self {
        Self {
            id: None,
            start_boundary: None,
            end_boundary: None,
            enabled: true,
            execution_time_limit: None,
            repetition: None,
        }
    }
}

impl TriggerCommon {
    /// Creates common settings starting at `at`.
    #[must_use]
    pub fn starting_at(at: TaskDateTime) -> Self {
        Self {
            start_boundary: Some(at),
            ..Self::default()
        }
    }
}

/// Repetition applied after a trigger fires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct Repetition {
    /// Non-zero interval between repetitions.
    pub interval: TaskDuration,
    /// Optional repetition window; `None` means indefinite.
    pub duration: Option<TaskDuration>,
    /// Stops an executing instance when the repetition window ends.
    #[cfg_attr(feature = "serde", serde(default))]
    pub stop_at_duration_end: bool,
}

/// A boot trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct BootTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Delay after boot.
    pub delay: Option<TaskDuration>,
}

impl BootTrigger {
    /// Creates an enabled trigger that fires after system boot.
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: TriggerCommon::default(),
            delay: None,
        }
    }
}

impl Default for BootTrigger {
    fn default() -> Self {
        Self::new()
    }
}

impl From<BootTrigger> for Trigger {
    fn from(value: BootTrigger) -> Self {
        Self::Boot(value)
    }
}

/// A registration trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct RegistrationTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Delay after registration or update.
    pub delay: Option<TaskDuration>,
}

impl RegistrationTrigger {
    /// Creates an enabled trigger that fires after registration or update.
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: TriggerCommon::default(),
            delay: None,
        }
    }
}

impl Default for RegistrationTrigger {
    fn default() -> Self {
        Self::new()
    }
}

impl From<RegistrationTrigger> for Trigger {
    fn from(value: RegistrationTrigger) -> Self {
        Self::Registration(value)
    }
}

/// An idle trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct IdleTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
}

impl IdleTrigger {
    /// Creates an enabled trigger that fires when the computer becomes idle.
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: TriggerCommon::default(),
        }
    }
}

impl Default for IdleTrigger {
    fn default() -> Self {
        Self::new()
    }
}

impl From<IdleTrigger> for Trigger {
    fn from(value: IdleTrigger) -> Self {
        Self::Idle(value)
    }
}

/// A one-time trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct TimeTrigger {
    /// Shared trigger settings, including a required start boundary.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
}

impl TimeTrigger {
    /// Creates an enabled one-time trigger at the required start boundary.
    #[must_use]
    pub fn new(at: TaskDateTime) -> Self {
        Self {
            common: TriggerCommon::starting_at(at),
            random_delay: None,
        }
    }
}

impl From<TimeTrigger> for Trigger {
    fn from(value: TimeTrigger) -> Self {
        Self::Time(value)
    }
}

/// A Windows Event Log subscription trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct EventTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// XPath event subscription.
    pub subscription: String,
    /// Named XPath value queries exposed as task variables.
    #[cfg_attr(feature = "serde", serde(default))]
    pub value_queries: BTreeMap<String, String>,
    /// Delay after a matching event.
    pub delay: Option<TaskDuration>,
    /// Event matching period.
    pub period_of_occurrence: Option<TaskDuration>,
    /// Number of matching events required in the period.
    pub number_of_occurrences: Option<u32>,
    /// Maximum wait for the occurrence pattern.
    pub matching_element: Option<String>,
}

impl EventTrigger {
    /// Creates an enabled trigger for one Windows Event Log XPath subscription.
    #[must_use]
    pub fn new(subscription: impl Into<String>) -> Self {
        Self {
            common: TriggerCommon::default(),
            subscription: subscription.into(),
            value_queries: BTreeMap::new(),
            delay: None,
            period_of_occurrence: None,
            number_of_occurrences: None,
            matching_element: None,
        }
    }
}

impl From<EventTrigger> for Trigger {
    fn from(value: EventTrigger) -> Self {
        Self::Event(value)
    }
}

/// A user logon trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct LogonTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Optional user or group; `None` means any user.
    pub user_id: Option<String>,
    /// Delay after logon.
    pub delay: Option<TaskDuration>,
}

impl LogonTrigger {
    /// Creates an enabled trigger that fires when any user logs on. Set
    /// `user_id` to restrict the trigger to one user or group.
    #[must_use]
    pub fn new() -> Self {
        Self {
            common: TriggerCommon::default(),
            user_id: None,
            delay: None,
        }
    }
}

impl Default for LogonTrigger {
    fn default() -> Self {
        Self::new()
    }
}

impl From<LogonTrigger> for Trigger {
    fn from(value: LogonTrigger) -> Self {
        Self::Logon(value)
    }
}

/// A Terminal Services session state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum SessionStateChange {
    /// A console connection occurred.
    ConsoleConnect,
    /// A console disconnect occurred.
    ConsoleDisconnect,
    /// A remote connection occurred.
    RemoteConnect,
    /// A remote disconnect occurred.
    RemoteDisconnect,
    /// The workstation became locked.
    SessionLock,
    /// The workstation became unlocked.
    SessionUnlock,
}

/// A session-state-change trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct SessionStateChangeTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// State transition that fires the task.
    pub state_change: SessionStateChange,
    /// Optional user filter.
    pub user_id: Option<String>,
    /// Delay after the transition.
    pub delay: Option<TaskDuration>,
}

impl SessionStateChangeTrigger {
    /// Creates an enabled trigger for one Terminal Services state change.
    #[must_use]
    pub fn new(state_change: SessionStateChange) -> Self {
        Self {
            common: TriggerCommon::default(),
            state_change,
            user_id: None,
            delay: None,
        }
    }
}

impl From<SessionStateChangeTrigger> for Trigger {
    fn from(value: SessionStateChangeTrigger) -> Self {
        Self::SessionStateChange(value)
    }
}

/// Day of the week used by calendar triggers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Weekday {
    /// Sunday.
    Sunday,
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
}

/// Calendar month.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum Month {
    /// January.
    January,
    /// February.
    February,
    /// March.
    March,
    /// April.
    April,
    /// May.
    May,
    /// June.
    June,
    /// July.
    July,
    /// August.
    August,
    /// September.
    September,
    /// October.
    October,
    /// November.
    November,
    /// December.
    December,
}

/// Week ordinal used by a monthly day-of-week trigger.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum WeekOfMonth {
    /// First matching day.
    First,
    /// Second matching day.
    Second,
    /// Third matching day.
    Third,
    /// Fourth matching day.
    Fourth,
}

/// A daily calendar trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct DailyTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Number of days between firings; must be at least one.
    pub days_interval: u16,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
}

impl DailyTrigger {
    /// Creates an enabled trigger that fires every day from `first`.
    #[must_use]
    pub fn new(first: TaskDateTime) -> Self {
        Self {
            common: TriggerCommon::starting_at(first),
            days_interval: 1,
            random_delay: None,
        }
    }
}

impl From<DailyTrigger> for Trigger {
    fn from(value: DailyTrigger) -> Self {
        Self::Daily(value)
    }
}

/// A weekly calendar trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct WeeklyTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Number of weeks between firings; must be at least one.
    pub weeks_interval: u16,
    /// Days that fire within each selected week.
    pub days_of_week: BTreeSet<Weekday>,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
}

impl WeeklyTrigger {
    /// Creates an enabled trigger that fires on the selected weekdays of every
    /// week from `first`.
    #[must_use]
    pub fn new(first: TaskDateTime, days_of_week: impl IntoIterator<Item = Weekday>) -> Self {
        Self {
            common: TriggerCommon::starting_at(first),
            weeks_interval: 1,
            days_of_week: days_of_week.into_iter().collect(),
            random_delay: None,
        }
    }
}

impl From<WeeklyTrigger> for Trigger {
    fn from(value: WeeklyTrigger) -> Self {
        Self::Weekly(value)
    }
}

/// A monthly day-number trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct MonthlyTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Selected day numbers in the range 1 through 31.
    pub days_of_month: BTreeSet<u8>,
    /// Selected months.
    pub months: BTreeSet<Month>,
    /// Also fires on the last day of selected months.
    #[cfg_attr(feature = "serde", serde(default))]
    pub run_on_last_day: bool,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
}

impl MonthlyTrigger {
    /// Creates an enabled trigger that fires on the selected day numbers of the
    /// selected months, at the time of day given by `first`.
    #[must_use]
    pub fn new(
        first: TaskDateTime,
        days_of_month: impl IntoIterator<Item = u8>,
        months: impl IntoIterator<Item = Month>,
    ) -> Self {
        Self {
            common: TriggerCommon::starting_at(first),
            days_of_month: days_of_month.into_iter().collect(),
            months: months.into_iter().collect(),
            run_on_last_day: false,
            random_delay: None,
        }
    }
}

impl From<MonthlyTrigger> for Trigger {
    fn from(value: MonthlyTrigger) -> Self {
        Self::Monthly(value)
    }
}

/// A monthly day-of-week trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct MonthlyDowTrigger {
    /// Shared trigger settings.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub common: TriggerCommon,
    /// Selected week ordinals.
    pub weeks_of_month: BTreeSet<WeekOfMonth>,
    /// Selected weekdays.
    pub days_of_week: BTreeSet<Weekday>,
    /// Selected months.
    pub months: BTreeSet<Month>,
    /// Also fires in the last matching week of each selected month.
    #[cfg_attr(feature = "serde", serde(default))]
    pub run_on_last_week: bool,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
}

impl MonthlyDowTrigger {
    /// Creates an enabled trigger that fires on the selected weekdays within the
    /// selected week ordinals of the selected months, at the time of day given
    /// by `first`.
    #[must_use]
    pub fn new(
        first: TaskDateTime,
        weeks_of_month: impl IntoIterator<Item = WeekOfMonth>,
        days_of_week: impl IntoIterator<Item = Weekday>,
        months: impl IntoIterator<Item = Month>,
    ) -> Self {
        Self {
            common: TriggerCommon::starting_at(first),
            weeks_of_month: weeks_of_month.into_iter().collect(),
            days_of_week: days_of_week.into_iter().collect(),
            months: months.into_iter().collect(),
            run_on_last_week: false,
            random_delay: None,
        }
    }
}

impl From<MonthlyDowTrigger> for Trigger {
    fn from(value: MonthlyDowTrigger) -> Self {
        Self::MonthlyDow(value)
    }
}

/// A future or custom trigger retained as XML.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct UnknownTrigger {
    /// XML local name.
    pub kind: String,
    /// Original XML element including start and end tags.
    pub xml: String,
}

impl From<UnknownTrigger> for Trigger {
    fn from(value: UnknownTrigger) -> Self {
        Self::Unknown(value)
    }
}

/// A Task Scheduler trigger.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(tag = "type", rename_all = "snake_case")
)]
pub enum Trigger {
    /// Starts after system boot.
    Boot(BootTrigger),
    /// Starts after task registration or update.
    Registration(RegistrationTrigger),
    /// Starts when the computer is idle.
    Idle(IdleTrigger),
    /// Starts once at a boundary, optionally with repetition.
    Time(TimeTrigger),
    /// Starts after a matching Windows event.
    Event(EventTrigger),
    /// Starts after user logon.
    Logon(LogonTrigger),
    /// Starts after a Terminal Services state change.
    SessionStateChange(SessionStateChangeTrigger),
    /// Starts on a daily cadence.
    Daily(DailyTrigger),
    /// Starts on a weekly cadence.
    Weekly(WeeklyTrigger),
    /// Starts on selected day numbers of selected months.
    Monthly(MonthlyTrigger),
    /// Starts on selected weekdays and week ordinals of selected months.
    MonthlyDow(MonthlyDowTrigger),
    /// A future or provider-specific trigger retained without interpretation.
    Unknown(UnknownTrigger),
}

impl Trigger {
    /// Returns common settings for known trigger kinds.
    #[must_use]
    pub fn common(&self) -> Option<&TriggerCommon> {
        match self {
            Self::Boot(value) => Some(&value.common),
            Self::Registration(value) => Some(&value.common),
            Self::Idle(value) => Some(&value.common),
            Self::Time(value) => Some(&value.common),
            Self::Event(value) => Some(&value.common),
            Self::Logon(value) => Some(&value.common),
            Self::SessionStateChange(value) => Some(&value.common),
            Self::Daily(value) => Some(&value.common),
            Self::Weekly(value) => Some(&value.common),
            Self::Monthly(value) => Some(&value.common),
            Self::MonthlyDow(value) => Some(&value.common),
            Self::Unknown(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DailyTrigger, Trigger, TriggerCommon};
    use crate::model::{Action, ExecAction, TaskDateTime, TaskDefinition};

    fn daily_definition(enabled: bool) -> TaskDefinition {
        let boundary = TaskDateTime::wall_clock(2026, 9, 5, 6, 0, 0).expect("fixed anchor");
        let mut trigger = DailyTrigger::new(boundary);
        trigger.common.enabled = enabled;
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("agent.exe")));
        definition
            .triggers
            .push(trigger.into())
            .expect("one trigger");
        definition.settings.enabled = enabled;
        definition
    }

    fn only_trigger(definition: &TaskDefinition) -> &TriggerCommon {
        definition
            .triggers
            .as_slice()
            .first()
            .and_then(Trigger::common)
            .expect("one known trigger")
    }

    // Task XML treats an absent `Enabled` element as enabled. Every other input
    // path must agree, otherwise the same omission means opposite things
    // depending on how the task was written.
    #[test]
    fn an_omitted_enabled_flag_means_enabled_on_every_input_path() {
        assert!(TriggerCommon::default().enabled);
        assert!(
            DailyTrigger::new(TaskDateTime::wall_clock(2026, 9, 5, 6, 0, 0).expect("anchor"))
                .common
                .enabled
        );

        let xml = crate::xml::to_string(&daily_definition(true)).expect("canonical XML");
        assert!(xml.contains("<Enabled>true</Enabled>"));
        let stripped = xml.replace("<Enabled>true</Enabled>", "");
        let decoded = crate::xml::from_bytes(stripped.as_bytes()).expect("XML without Enabled");
        assert!(only_trigger(&decoded).enabled);
        assert!(decoded.settings.enabled);
    }

    // An explicit `false` must survive; the corrected default must not swallow it.
    #[test]
    fn an_explicit_disabled_flag_survives_every_round_trip() {
        let definition = daily_definition(false);
        let xml = crate::xml::to_string(&definition).expect("canonical XML");
        assert!(xml.contains("<Enabled>false</Enabled>"));
        let decoded = crate::xml::from_bytes(xml.as_bytes()).expect("XML round trip");
        assert!(!only_trigger(&decoded).enabled);
        assert!(!decoded.settings.enabled);
        assert_eq!(decoded, definition);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_manifest_that_omits_enabled_registers_a_scheduling_trigger() {
        let manifest = crate::manifest::TaskManifest::from_slice(
            MINIMAL_MANIFEST.as_bytes(),
            crate::manifest::DocumentFormat::Toml,
        )
        .expect("minimal manifest");
        let task = manifest.tasks.first().expect("one managed task");
        assert!(only_trigger(&task.definition).enabled);
        assert!(task.definition.settings.enabled);
        assert!(task.definition.validate().is_valid());
    }

    // Kept in sync with `examples/desired-state-minimal.toml`: the shortest
    // document a user can write and still get documented defaults.
    #[cfg(feature = "serde")]
    const MINIMAL_MANIFEST: &str = include_str!("../../examples/desired-state-minimal.toml");
}

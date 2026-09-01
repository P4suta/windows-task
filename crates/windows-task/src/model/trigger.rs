use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{TaskDateTime, TaskDuration, TaskLimit};

/// Settings shared by every trigger.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
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

impl TriggerCommon {
    /// Creates enabled trigger settings.
    #[must_use]
    pub fn enabled() -> Self {
        Self {
            enabled: true,
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
    pub run_on_last_day: bool,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
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
    pub run_on_last_week: bool,
    /// Maximum random delay.
    pub random_delay: Option<TaskDuration>,
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

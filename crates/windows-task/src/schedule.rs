//! High-level schedule recipes and exact POSIX cron compilation.

#[cfg(feature = "cron")]
use std::{collections::BTreeSet, str::FromStr};

#[cfg(all(feature = "cron", feature = "serde"))]
use serde::{Deserialize, Serialize};

use crate::model::{
    BootTrigger, DailyTrigger, EventTrigger, LogonTrigger, Repetition, TaskDateTime, TaskDuration,
    TimeTrigger, Trigger, TriggerCommon, Weekday, WeeklyTrigger,
};

#[cfg(feature = "cron")]
use crate::{
    MAX_TRIGGERS,
    model::{Month, MonthlyDowTrigger, MonthlyTrigger, WeekOfMonth},
};

/// Creates a one-time trigger.
#[must_use]
pub fn once(at: TaskDateTime) -> Trigger {
    Trigger::Time(TimeTrigger {
        common: TriggerCommon {
            start_boundary: Some(at),
            ..TriggerCommon::enabled()
        },
        random_delay: None,
    })
}

/// Creates a daily trigger at the supplied first boundary.
#[must_use]
pub fn daily(first: TaskDateTime) -> Trigger {
    Trigger::Daily(DailyTrigger {
        common: TriggerCommon {
            start_boundary: Some(first),
            ..TriggerCommon::enabled()
        },
        days_interval: 1,
        random_delay: None,
    })
}

/// Creates a weekly trigger.
#[must_use]
pub fn weekly(first: TaskDateTime, days: impl IntoIterator<Item = Weekday>) -> Trigger {
    Trigger::Weekly(WeeklyTrigger {
        common: TriggerCommon {
            start_boundary: Some(first),
            ..TriggerCommon::enabled()
        },
        weeks_interval: 1,
        days_of_week: days.into_iter().collect(),
        random_delay: None,
    })
}

/// Creates a logon trigger for one user or for any user.
#[must_use]
pub fn at_logon(user_id: Option<String>) -> Trigger {
    Trigger::Logon(LogonTrigger {
        common: TriggerCommon::enabled(),
        user_id,
        delay: None,
    })
}

/// Creates a system boot trigger.
#[must_use]
pub fn at_startup() -> Trigger {
    Trigger::Boot(BootTrigger {
        common: TriggerCommon::enabled(),
        delay: None,
    })
}

/// Creates a Windows Event Log XPath trigger.
#[must_use]
pub fn on_event(subscription: impl Into<String>) -> Trigger {
    Trigger::Event(EventTrigger {
        common: TriggerCommon::enabled(),
        subscription: subscription.into(),
        value_queries: std::collections::BTreeMap::new(),
        delay: None,
        period_of_occurrence: None,
        number_of_occurrences: None,
        matching_element: None,
    })
}

/// Creates a trigger that repeats forever from a boundary.
pub fn every(first: TaskDateTime, interval: TaskDuration) -> Result<Trigger, ScheduleError> {
    if interval.as_std().is_zero() {
        return Err(ScheduleError::Invalid(
            "repeat interval must be greater than zero".into(),
        ));
    }
    Ok(Trigger::Time(TimeTrigger {
        common: TriggerCommon {
            start_boundary: Some(first),
            repetition: Some(Repetition {
                interval,
                duration: None,
                stop_at_duration_end: false,
            }),
            ..TriggerCommon::enabled()
        },
        random_delay: None,
    }))
}

/// Parsed POSIX five-field cron expression.
#[cfg(feature = "cron")]
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct CronSchedule {
    original: String,
    minutes: Field,
    hours: Field,
    days_of_month: Field,
    months: Field,
    days_of_week: Field,
}

#[cfg(feature = "cron")]
impl CronSchedule {
    /// Parses `minute hour day-of-month month day-of-week` using POSIX DOM/DOW
    /// OR semantics. Lists, ranges, steps, and English month/day names work.
    pub fn parse(expression: impl Into<String>) -> Result<Self, ScheduleError> {
        let original = expression.into();
        let fields: Vec<_> = original.split_ascii_whitespace().collect();
        if fields.len() != 5 {
            return Err(ScheduleError::Invalid(
                "cron requires exactly five fields: minute hour day-of-month month day-of-week"
                    .into(),
            ));
        }
        Ok(Self {
            minutes: Field::parse(fields[0], 0, 59, &[], false)?,
            hours: Field::parse(fields[1], 0, 23, &[], false)?,
            days_of_month: Field::parse(fields[2], 1, 31, &[], false)?,
            months: Field::parse(fields[3], 1, 12, &MONTH_NAMES, false)?,
            days_of_week: Field::parse(fields[4], 0, 7, &WEEKDAY_NAMES, true)?,
            original,
        })
    }

    /// Returns the original normalized expression text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Compiles this expression to native calendar triggers anchored to the
    /// date and offset of `first`. It fails instead of approximating or
    /// producing duplicate runs.
    pub fn compile(&self, first: &TaskDateTime) -> Result<Vec<Trigger>, ScheduleError> {
        if !self.days_of_month.wildcard && !self.days_of_week.wildcard {
            return Err(ScheduleError::NotRepresentable {
                reason: "POSIX OR semantics for simultaneous day-of-month and day-of-week restrictions would duplicate overlapping runs".into(),
                required_triggers: None,
            });
        }

        let times = self.hours.values.iter().flat_map(|hour| {
            self.minutes
                .values
                .iter()
                .map(move |minute| (*hour, *minute))
        });
        let mut triggers = Vec::new();
        for (hour, minute) in times {
            let boundary = boundary_at(first, hour, minute)?;
            triggers.push(self.calendar_trigger(boundary));
            if triggers.len() > MAX_TRIGGERS {
                return Err(ScheduleError::NotRepresentable {
                    reason: "expanded time selections exceed Task Scheduler's trigger limit".into(),
                    required_triggers: Some(triggers.len()),
                });
            }
        }
        Ok(triggers)
    }

    fn calendar_trigger(&self, boundary: TaskDateTime) -> Trigger {
        let common = TriggerCommon {
            start_boundary: Some(boundary),
            ..TriggerCommon::enabled()
        };
        if self.days_of_month.wildcard && self.days_of_week.wildcard && self.months.wildcard {
            return Trigger::Daily(DailyTrigger {
                common,
                days_interval: 1,
                random_delay: None,
            });
        }
        if !self.days_of_week.wildcard && self.months.wildcard {
            return Trigger::Weekly(WeeklyTrigger {
                common,
                weeks_interval: 1,
                days_of_week: self
                    .days_of_week
                    .values
                    .iter()
                    .map(|value| weekday(*value))
                    .collect(),
                random_delay: None,
            });
        }
        if !self.days_of_week.wildcard {
            return Trigger::MonthlyDow(MonthlyDowTrigger {
                common,
                weeks_of_month: [
                    WeekOfMonth::First,
                    WeekOfMonth::Second,
                    WeekOfMonth::Third,
                    WeekOfMonth::Fourth,
                ]
                .into_iter()
                .collect(),
                days_of_week: self
                    .days_of_week
                    .values
                    .iter()
                    .map(|value| weekday(*value))
                    .collect(),
                months: self
                    .months
                    .values
                    .iter()
                    .map(|value| month(*value))
                    .collect(),
                run_on_last_week: true,
                random_delay: None,
            });
        }
        Trigger::Monthly(MonthlyTrigger {
            common,
            days_of_month: self.days_of_month.values.iter().copied().collect(),
            months: self
                .months
                .values
                .iter()
                .map(|value| month(*value))
                .collect(),
            run_on_last_day: false,
            random_delay: None,
        })
    }
}

#[cfg(feature = "cron")]
impl FromStr for CronSchedule {
    type Err = ScheduleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[cfg(feature = "cron")]
impl TryFrom<String> for CronSchedule {
    type Error = ScheduleError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

#[cfg(feature = "cron")]
impl From<CronSchedule> for String {
    fn from(value: CronSchedule) -> Self {
        value.original
    }
}

/// Cron or high-level schedule compilation failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ScheduleError {
    /// Invalid recipe or cron syntax.
    #[error("invalid schedule: {0}")]
    Invalid(String),
    /// A valid schedule cannot be represented exactly by native triggers.
    #[error("schedule is not exactly representable: {reason}")]
    NotRepresentable {
        /// Explanation of the semantic mismatch.
        reason: String,
        /// Expanded trigger count when the limit caused failure.
        required_triggers: Option<usize>,
    },
}

#[cfg(feature = "cron")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct Field {
    values: BTreeSet<u8>,
    wildcard: bool,
}

#[cfg(feature = "cron")]
impl Field {
    fn parse(
        text: &str,
        minimum: u8,
        maximum: u8,
        names: &[(&str, u8)],
        sunday_alias: bool,
    ) -> Result<Self, ScheduleError> {
        let wildcard = text == "*";
        let mut values = BTreeSet::new();
        for part in text.split(',') {
            let (range, step_text) = part
                .split_once('/')
                .map_or((part, None), |(range, step)| (range, Some(step)));
            let step = step_text.map_or(Ok(1), |step| {
                step.parse::<u8>().map_err(|_| {
                    ScheduleError::Invalid(format!("invalid step {step:?} in {text:?}"))
                })
            })?;
            if step == 0 {
                return Err(ScheduleError::Invalid(format!(
                    "invalid zero step in {text:?}"
                )));
            }
            let (start, end) = if range == "*" {
                (minimum, maximum)
            } else if let Some((start, end)) = range.split_once('-') {
                (
                    field_value(start, names, sunday_alias)?,
                    field_value(end, names, sunday_alias)?,
                )
            } else {
                let value = field_value(range, names, sunday_alias)?;
                (value, if step_text.is_some() { maximum } else { value })
            };
            if start < minimum || end > maximum || start > end {
                return Err(ScheduleError::Invalid(format!(
                    "field {part:?} is outside {minimum}..={maximum}"
                )));
            }
            for value in (start..=end).step_by(usize::from(step)) {
                values.insert(if sunday_alias && value == 7 { 0 } else { value });
            }
        }
        if values.is_empty() {
            return Err(ScheduleError::Invalid(format!("empty cron field {text:?}")));
        }
        Ok(Self { values, wildcard })
    }
}

#[cfg(feature = "cron")]
fn field_value(text: &str, names: &[(&str, u8)], sunday_alias: bool) -> Result<u8, ScheduleError> {
    if let Some((_, value)) = names
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(text))
    {
        return Ok(*value);
    }
    let value = text
        .parse::<u8>()
        .map_err(|_| ScheduleError::Invalid(format!("invalid cron value {text:?}")))?;
    Ok(if sunday_alias && value == 7 { 7 } else { value })
}

#[cfg(feature = "cron")]
fn boundary_at(first: &TaskDateTime, hour: u8, minute: u8) -> Result<TaskDateTime, ScheduleError> {
    let text = first.as_str();
    let Some(time_index) = text.find('T') else {
        return Err(ScheduleError::Invalid(
            "cron anchor has no time separator".into(),
        ));
    };
    let suffix_index = text[time_index + 1..]
        .find(['+', '-', 'Z'])
        .map_or(text.len(), |index| time_index + 1 + index);
    let suffix = &text[suffix_index..];
    TaskDateTime::parse(format!(
        "{}T{hour:02}:{minute:02}:00{suffix}",
        &text[..time_index]
    ))
    .map_err(|error| ScheduleError::Invalid(error.to_string()))
}

#[cfg(feature = "cron")]
fn weekday(value: u8) -> Weekday {
    match value {
        0 => Weekday::Sunday,
        1 => Weekday::Monday,
        2 => Weekday::Tuesday,
        3 => Weekday::Wednesday,
        4 => Weekday::Thursday,
        5 => Weekday::Friday,
        6 => Weekday::Saturday,
        _ => unreachable!("cron weekday was validated"),
    }
}

#[cfg(feature = "cron")]
fn month(value: u8) -> Month {
    match value {
        1 => Month::January,
        2 => Month::February,
        3 => Month::March,
        4 => Month::April,
        5 => Month::May,
        6 => Month::June,
        7 => Month::July,
        8 => Month::August,
        9 => Month::September,
        10 => Month::October,
        11 => Month::November,
        12 => Month::December,
        _ => unreachable!("cron month was validated"),
    }
}

#[cfg(feature = "cron")]
const MONTH_NAMES: [(&str, u8); 12] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

#[cfg(feature = "cron")]
const WEEKDAY_NAMES: [(&str, u8); 7] = [
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];

#[cfg(all(test, feature = "cron"))]
mod tests {
    use super::{CronSchedule, ScheduleError};
    use crate::model::{TaskDateTime, Trigger};

    #[test]
    fn compiles_weekday_cron() {
        let cron = CronSchedule::parse("30 8 * * mon-fri").expect("cron");
        let first = TaskDateTime::parse("2026-09-02T00:00:00").expect("anchor");
        let triggers = cron.compile(&first).expect("native schedule");
        assert_eq!(triggers.len(), 1);
        assert!(matches!(triggers[0], Trigger::Weekly(_)));
    }

    #[test]
    fn rejects_posix_or_overlap() {
        let cron = CronSchedule::parse("0 0 1 * mon").expect("cron");
        let first = TaskDateTime::parse("2026-09-02T00:00:00").expect("anchor");
        assert!(matches!(
            cron.compile(&first),
            Err(ScheduleError::NotRepresentable { .. })
        ));
    }

    #[test]
    fn enforces_native_trigger_limit() {
        let cron = CronSchedule::parse("0,15,30,45 * * * *").expect("cron");
        let first = TaskDateTime::parse("2026-09-02T00:00:00Z").expect("anchor");
        assert!(matches!(
            cron.compile(&first),
            Err(ScheduleError::NotRepresentable { .. })
        ));
    }

    #[test]
    fn expands_a_stepped_single_value_to_the_field_max() {
        let cron = CronSchedule::parse("5/10 0 * * *").expect("cron");
        let first = TaskDateTime::parse("2026-09-02T00:00:00Z").expect("anchor");
        let triggers = cron.compile(&first).expect("native schedule");
        assert_eq!(triggers.len(), 6);
    }
}

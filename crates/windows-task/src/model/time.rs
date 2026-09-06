use std::{fmt, str::FromStr, time::Duration};

use jiff::civil::DateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Whether a task boundary follows the target's wall clock or a fixed offset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum TimeBasis {
    /// Repeats according to the target computer's local wall clock and DST.
    LocalWallClock,
    /// Denotes an instant with a numeric UTC offset or `Z`.
    FixedOffset,
}

/// A validated Task Scheduler boundary timestamp.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct TaskDateTime {
    text: String,
    basis: TimeBasis,
}

impl TaskDateTime {
    /// Parses an ISO-8601 timestamp accepted by Task Scheduler.
    pub fn parse(value: impl Into<String>) -> Result<Self, ParseTaskDateTimeError> {
        let value = value.into();
        let offset_start = numeric_offset_start(&value);
        let civil_text = if value.ends_with('Z') {
            &value[..value.len() - 1]
        } else if let Some(index) = offset_start {
            validate_offset(&value[index..])?;
            &value[..index]
        } else {
            value.as_str()
        };
        DateTime::from_str(civil_text).map_err(|error| ParseTaskDateTimeError {
            message: format!("invalid task boundary {value:?}: {error}"),
        })?;
        Ok(Self {
            basis: if value.ends_with('Z') || offset_start.is_some() {
                TimeBasis::FixedOffset
            } else {
                TimeBasis::LocalWallClock
            },
            text: value,
        })
    }

    /// Creates a boundary that follows the target computer's wall clock,
    /// including its DST transitions. This is the basis Task Scheduler
    /// applies to a timestamp written without a UTC offset.
    pub fn wall_clock(
        year: i16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, ParseTaskDateTimeError> {
        Self::parse(civil_text(year, month, day, hour, minute, second))
    }

    /// Creates a boundary at a fixed UTC instant, which does not shift with
    /// the target computer's DST transitions.
    pub fn utc(
        year: i16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, ParseTaskDateTimeError> {
        Self::parse(format!(
            "{}Z",
            civil_text(year, month, day, hour, minute, second)
        ))
    }

    /// Returns the scheduler-compatible ISO-8601 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the timestamp's recurrence basis.
    #[must_use]
    pub const fn basis(&self) -> TimeBasis {
        self.basis
    }
}

impl FromStr for TaskDateTime {
    type Err = ParseTaskDateTimeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for TaskDateTime {
    type Error = ParseTaskDateTimeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<TaskDateTime> for String {
    fn from(value: TaskDateTime) -> Self {
        value.text
    }
}

impl fmt::Display for TaskDateTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text.fmt(formatter)
    }
}

/// Error returned for an invalid task boundary.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ParseTaskDateTimeError {
    message: String,
}

/// A fixed non-negative duration with Task Scheduler ISO-8601 formatting.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct TaskDuration(Duration);

impl TaskDuration {
    /// Constructs a duration from seconds.
    #[must_use]
    pub const fn from_secs(seconds: u64) -> Self {
        Self(Duration::from_secs(seconds))
    }

    /// Constructs a duration from whole minutes.
    #[must_use]
    pub const fn from_mins(minutes: u64) -> Self {
        Self(Duration::from_secs(minutes.saturating_mul(60)))
    }

    /// Constructs a duration from whole hours.
    #[must_use]
    pub const fn from_hours(hours: u64) -> Self {
        Self(Duration::from_secs(hours.saturating_mul(3_600)))
    }

    /// Constructs a duration from whole days.
    #[must_use]
    pub const fn from_days(days: u64) -> Self {
        Self(Duration::from_secs(days.saturating_mul(86_400)))
    }

    /// Constructs a duration from a standard duration.
    #[must_use]
    pub const fn from_std(duration: Duration) -> Self {
        Self(duration)
    }

    /// Returns the standard duration.
    #[must_use]
    pub const fn as_std(self) -> Duration {
        self.0
    }

    /// Parses the fixed-unit subset of ISO-8601 used by Task Scheduler.
    pub fn parse(value: &str) -> Result<Self, ParseTaskDurationError> {
        parse_iso_duration(value).map(Self)
    }
}

impl FromStr for TaskDuration {
    type Err = ParseTaskDurationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for TaskDuration {
    type Error = ParseTaskDurationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<TaskDuration> for String {
    fn from(value: TaskDuration) -> Self {
        value.to_string()
    }
}

impl fmt::Display for TaskDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.0.as_secs();
        let days = total / 86_400;
        let hours = total % 86_400 / 3_600;
        let minutes = total % 3_600 / 60;
        let seconds = total % 60;
        formatter.write_str("P")?;
        if days != 0 {
            write!(formatter, "{days}D")?;
        }
        let has_time =
            hours != 0 || minutes != 0 || seconds != 0 || self.0.subsec_nanos() != 0 || days == 0;
        if has_time {
            formatter.write_str("T")?;
            if hours != 0 {
                write!(formatter, "{hours}H")?;
            }
            if minutes != 0 {
                write!(formatter, "{minutes}M")?;
            }
            if self.0.subsec_nanos() == 0 {
                if seconds != 0 || (hours == 0 && minutes == 0) {
                    write!(formatter, "{seconds}S")?;
                }
            } else {
                let mut fraction = format!("{:09}", self.0.subsec_nanos());
                while fraction.ends_with('0') {
                    fraction.pop();
                }
                write!(formatter, "{seconds}.{fraction}S")?;
            }
        }
        Ok(())
    }
}

/// Error returned for an unsupported or malformed duration.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ParseTaskDurationError {
    message: String,
}

/// A scheduler time limit with an explicit unlimited sentinel.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum TaskLimit {
    /// Stop after this finite duration.
    Finite(TaskDuration),
    /// Do not impose a time limit.
    Unlimited,
}

impl Default for TaskLimit {
    fn default() -> Self {
        Self::Finite(TaskDuration::from_secs(72 * 60 * 60))
    }
}

fn civil_text(year: i16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> String {
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

fn numeric_offset_start(value: &str) -> Option<usize> {
    let time = value.find('T')?;
    value[time + 1..]
        .rfind(['+', '-'])
        .map(|relative| time + 1 + relative)
}

fn validate_offset(offset: &str) -> Result<(), ParseTaskDateTimeError> {
    let hours = offset.get(1..3).and_then(|value| value.parse::<u8>().ok());
    let minutes = offset.get(4..6).and_then(|value| value.parse::<u8>().ok());
    let valid = offset.len() == 6
        && matches!(offset.as_bytes()[0], b'+' | b'-')
        && offset.as_bytes()[3] == b':'
        && hours.is_some_and(|hours| hours <= 14)
        && minutes.is_some_and(|minutes| minutes <= 59)
        && !(hours == Some(14) && minutes != Some(0));
    if valid {
        Ok(())
    } else {
        Err(ParseTaskDateTimeError {
            message: format!("invalid numeric UTC offset {offset:?}"),
        })
    }
}

fn parse_iso_duration(value: &str) -> Result<Duration, ParseTaskDurationError> {
    let Some(body) = value.strip_prefix('P') else {
        return Err(duration_error(value));
    };
    if body.is_empty() {
        return Err(duration_error(value));
    }
    let (date, time) = match body.split_once('T') {
        Some((date, time)) if !time.is_empty() && !time.contains('T') => (date, Some(time)),
        Some(_) => return Err(duration_error(value)),
        None => (body, None),
    };
    let has_days = !date.is_empty();
    let days = if has_days {
        let digits = date
            .strip_suffix('D')
            .ok_or_else(|| duration_error(value))?;
        parse_unsigned(digits).ok_or_else(|| duration_error(value))?
    } else {
        0
    };
    let mut hours = 0;
    let mut minutes = 0;
    let mut seconds = 0;
    let mut nanos = 0;
    let mut components = 0_u8;
    if let Some(time) = time {
        let mut start = 0;
        let mut last_order = 0;
        for (index, suffix) in time.char_indices() {
            let order = match suffix {
                'H' => 1,
                'M' => 2,
                'S' => 3,
                character if character.is_ascii_digit() || character == '.' => continue,
                _ => return Err(duration_error(value)),
            };
            if order <= last_order {
                return Err(duration_error(value));
            }
            let component = &time[start..index];
            match suffix {
                'H' => hours = parse_unsigned(component).ok_or_else(|| duration_error(value))?,
                'M' => minutes = parse_unsigned(component).ok_or_else(|| duration_error(value))?,
                'S' => {
                    (seconds, nanos) =
                        parse_seconds(component).ok_or_else(|| duration_error(value))?;
                }
                _ => unreachable!("time suffix was matched above"),
            }
            components += 1;
            last_order = order;
            start = index + suffix.len_utf8();
        }
        if components == 0 || start != time.len() {
            return Err(duration_error(value));
        }
    } else if !has_days {
        return Err(duration_error(value));
    }
    let total = days
        .checked_mul(86_400)
        .and_then(|sum| sum.checked_add(hours.checked_mul(3_600)?))
        .and_then(|sum| sum.checked_add(minutes.checked_mul(60)?))
        .and_then(|sum| sum.checked_add(seconds))
        .ok_or_else(|| duration_error(value))?;
    Ok(Duration::new(total, nanos))
}

fn parse_unsigned(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        value.parse().ok()
    }
}

fn parse_seconds(value: &str) -> Option<(u64, u32)> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let seconds = parse_unsigned(whole)?;
    if value.contains('.')
        && (fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<9}").parse().ok()?
    };
    Some((seconds, nanos))
}

fn duration_error(value: &str) -> ParseTaskDurationError {
    ParseTaskDurationError {
        message: format!("invalid fixed ISO-8601 task duration {value:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskDateTime, TaskDuration, TimeBasis};

    #[test]
    fn fixed_durations_reject_calendar_units_overflow_and_ambiguous_fractions() {
        for input in [
            "",
            "P",
            "1S",
            "-PT1S",
            "P1Y",
            "P1M",
            "P1W",
            "P-1D",
            "P1.5D",
            "PT1HT1M",
            "PT1H1H",
            "PT1.2H",
            "PT1.2M",
            "PT1M2",
            "PT.S",
            "PT1.1234567890S",
            "PT1.2.3S",
            "PT1Q",
            "P18446744073709551615D",
            "PT18446744073709551615H",
            "PT18446744073709551616S",
        ] {
            TaskDuration::parse(input).expect_err("unsupported fixed duration");
        }
        for duration in [
            std::time::Duration::ZERO,
            std::time::Duration::from_nanos(1),
            std::time::Duration::new(3661, 123_400_000),
            std::time::Duration::from_secs(u64::MAX),
        ] {
            let task = TaskDuration::from_std(duration);
            assert_eq!(
                TaskDuration::parse(&task.to_string())
                    .expect("canonical duration")
                    .as_std(),
                duration
            );
            assert_eq!(
                TaskDuration::try_from(String::from(task)).expect("owned string conversion"),
                task
            );
        }
    }

    #[test]
    fn timestamp_calendar_and_offset_boundaries_remain_distinct() {
        for input in [
            "",
            "2026-02-29T00:00:00",
            "2026-04-31T00:00:00",
            "2026-09-05T25:00:00",
            "2026-09-05T00:00:00+15:00",
            "2026-09-05T00:00:00-14:01",
            "2026-09-05T00:00:00+09:60",
            "2026-09-05T00:00:00+9:00",
            "2026-09-05T00:00:00+09",
        ] {
            TaskDateTime::parse(input).expect_err("invalid calendar or offset");
        }
        for input in [
            "2024-02-29T00:00:00",
            "2026-09-05T00:00:00+14:00",
            "2026-09-05T00:00:00-14:00",
            "1969-12-31T23:59:59.5Z",
        ] {
            let timestamp = TaskDateTime::try_from(input.to_owned()).expect("boundary timestamp");
            assert_eq!(timestamp.to_string(), input);
            assert_eq!(String::from(timestamp), input);
        }
    }

    #[test]
    fn separates_wall_clock_and_offset_boundaries() {
        let local = TaskDateTime::parse("2026-09-02T08:30:00").expect("local boundary");
        let fixed = TaskDateTime::parse("2026-09-02T08:30:00+09:00").expect("fixed boundary");
        assert_eq!(local.basis(), TimeBasis::LocalWallClock);
        assert_eq!(fixed.basis(), TimeBasis::FixedOffset);
        TaskDateTime::parse("2026-09-02T08:30:00+14:01").expect_err("XSD time zones stop at 14:00");
    }

    #[test]
    fn duration_round_trips() {
        for input in ["PT0S", "P0D", "PT15M", "P2D", "P2DT3H4M5.25S"] {
            let duration = TaskDuration::parse(input).expect("duration");
            assert_eq!(
                TaskDuration::parse(&duration.to_string()).expect("canonical"),
                duration
            );
        }
    }

    #[test]
    fn duration_rejects_empty_or_out_of_order_time_parts() {
        for input in ["P3DT", "PT1M2H", "PT1.S", "PT"] {
            TaskDuration::parse(input)
                .expect_err("empty or out-of-order time parts must be rejected");
        }
        assert_eq!(TaskDuration::from_secs(3 * 86_400).to_string(), "P3D");
    }
}

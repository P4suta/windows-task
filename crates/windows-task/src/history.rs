//! Typed Task Scheduler Operational event history.

use std::{
    collections::BTreeMap,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, ErrorKind, Result, TaskPath};

/// Windows Event Log channel used by Task Scheduler 2.0.
pub const OPERATIONAL_CHANNEL: &str = "Microsoft-Windows-TaskScheduler/Operational";

/// Maximum rendered event XML accepted from the Event Log service.
pub const MAX_EVENT_XML_BYTES: usize = 1024 * 1024;

/// Known Task Scheduler Operational event categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum HistoryEventKind {
    /// A task was registered or updated.
    Registered,
    /// A task was deleted.
    Deleted,
    /// A launch request was accepted.
    LaunchRequested,
    /// An instance started.
    Started,
    /// An action started.
    ActionStarted,
    /// An action completed.
    ActionCompleted,
    /// An instance completed.
    Completed,
    /// An instance was stopped.
    Stopped,
    /// A launch or action failed.
    Failed,
    /// An event id unknown to this crate.
    Unknown(u32),
}

/// One event from `Microsoft-Windows-TaskScheduler/Operational`.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct HistoryEvent {
    /// Event Log record id.
    pub record_id: u64,
    /// Provider event id.
    pub event_id: u32,
    /// Typed event category.
    pub kind: HistoryEventKind,
    /// Timestamp reported by Event Log.
    pub timestamp: SystemTime,
    /// Task path when present.
    pub task_path: Option<TaskPath>,
    /// Run instance GUID when present.
    pub instance_id: Option<Uuid>,
    /// Native result code when present.
    pub result_code: Option<i32>,
    /// Named event payload fields, including unknown future fields.
    pub fields: BTreeMap<String, String>,
    /// Rendered provider message when metadata is available.
    pub message: Option<String>,
}

/// History query filter.
#[derive(Clone, Debug, Default)]
pub struct HistoryQuery {
    /// Task path filter.
    pub task: Option<TaskPath>,
    /// Run instance filter.
    pub instance_id: Option<Uuid>,
    /// Oldest allowed timestamp.
    pub since: Option<SystemTime>,
    /// Maximum returned records.
    pub limit: Option<usize>,
    /// Reads oldest-first when true.
    pub forward: bool,
}

/// Confidence attached to a run completion result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ResultConfidence {
    /// Correlated by instance GUID through event history.
    Exact,
    /// Inferred from registered-task polling because history was unavailable.
    PollingFallback,
}

/// Final result observed for a task run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct RunOutcome {
    /// Correlated instance.
    pub instance_id: Uuid,
    /// Native result or process exit code.
    pub result_code: i32,
    /// How confidently this result identifies the requested run.
    pub confidence: ResultConfidence,
}

/// Decodes one XML document rendered by `EvtRender(EvtRenderEventXml)`.
///
/// This is public so callers that archive `.evtx` data through another
/// transport can still use the crate's stable event model.
pub fn from_event_xml(xml: &str) -> Result<HistoryEvent> {
    if xml.len() > MAX_EVENT_XML_BYTES {
        return Err(history_xml_error("rendered event exceeds the 1 MiB limit"));
    }

    #[derive(Default)]
    struct Parsed {
        event_id: Option<u32>,
        record_id: Option<u64>,
        timestamp: Option<SystemTime>,
        fields: BTreeMap<String, String>,
        message: Option<String>,
        unnamed_data: usize,
    }

    #[derive(Debug)]
    struct Frame {
        name: String,
        data_name: Option<String>,
        text: String,
    }

    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut parsed = Parsed::default();
    let mut stack = Vec::<Frame>::new();
    let mut nodes = 0_usize;
    loop {
        use quick_xml::events::Event;
        match reader
            .read_event()
            .map_err(|error| history_xml_error(error.to_string()))?
        {
            Event::Start(element) => {
                nodes += 1;
                if nodes > 10_000 {
                    return Err(history_xml_error("event XML element count exceeds 10,000"));
                }
                if stack.len() >= 64 {
                    return Err(history_xml_error("event XML nesting exceeds 64 levels"));
                }
                let name = local_name(element.name().as_ref())?;
                let data_name = attribute_value(&reader, &element, "Name")?;
                if name == "TimeCreated" {
                    if let Some(value) = attribute_value(&reader, &element, "SystemTime")? {
                        parsed.timestamp = Some(parse_system_time(&value)?);
                    }
                }
                stack.push(Frame {
                    name,
                    data_name,
                    text: String::new(),
                });
            }
            Event::Empty(element) => {
                nodes += 1;
                if nodes > 10_000 {
                    return Err(history_xml_error("event XML element count exceeds 10,000"));
                }
                let name = local_name(element.name().as_ref())?;
                if name == "TimeCreated" {
                    if let Some(value) = attribute_value(&reader, &element, "SystemTime")? {
                        parsed.timestamp = Some(parse_system_time(&value)?);
                    }
                }
            }
            Event::Text(text) => {
                let decoded = text
                    .xml10_content()
                    .map_err(|error| history_xml_error(error.to_string()))?;
                let decoded = quick_xml::escape::unescape(&decoded)
                    .map_err(|error| history_xml_error(error.to_string()))?;
                if let Some(frame) = stack.last_mut() {
                    frame.text.push_str(&decoded);
                }
            }
            Event::CData(text) => {
                if let Some(frame) = stack.last_mut() {
                    frame.text.push_str(
                        &text
                            .xml10_content()
                            .map_err(|error| history_xml_error(error.to_string()))?,
                    );
                }
            }
            Event::End(_) => {
                let frame = stack
                    .pop()
                    .ok_or_else(|| history_xml_error("unexpected event XML end element"))?;
                let value = frame.text.trim();
                let parent_is_rendering = stack
                    .last()
                    .is_some_and(|parent| parent.name == "RenderingInfo");
                match frame.name.as_str() {
                    "EventID" => {
                        parsed.event_id = Some(value.parse().map_err(|error| {
                            history_xml_error(format!("invalid EventID {value:?}: {error}"))
                        })?);
                    }
                    "EventRecordID" => {
                        parsed.record_id = Some(value.parse().map_err(|error| {
                            history_xml_error(format!("invalid EventRecordID {value:?}: {error}"))
                        })?);
                    }
                    "Data" => {
                        let key = frame.data_name.unwrap_or_else(|| {
                            let key = format!("Data{}", parsed.unnamed_data);
                            parsed.unnamed_data += 1;
                            key
                        });
                        parsed.fields.insert(key, value.to_owned());
                    }
                    "Message" if parent_is_rendering => {
                        parsed.message = (!value.is_empty()).then(|| value.to_owned());
                    }
                    _ => {}
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(history_xml_error(
                    "DTD declarations and entity references are not allowed",
                ));
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(history_xml_error(
            "event XML ended before all elements closed",
        ));
    }

    let event_id = parsed
        .event_id
        .ok_or_else(|| history_xml_error("event XML has no EventID"))?;
    let record_id = parsed
        .record_id
        .ok_or_else(|| history_xml_error("event XML has no EventRecordID"))?;
    let timestamp = parsed
        .timestamp
        .ok_or_else(|| history_xml_error("event XML has no TimeCreated/SystemTime"))?;
    let task_path = field(&parsed.fields, &["TaskName", "TaskPath"])
        .filter(|value| !value.is_empty())
        .map(TaskPath::from_str)
        .transpose()
        .map_err(|error| history_xml_error(format!("invalid task path in event: {error}")))?;
    let instance_id = field(
        &parsed.fields,
        &["InstanceId", "InstanceID", "InstanceGuid", "TaskInstanceId"],
    )
    .filter(|value| !value.is_empty())
    .map(|value| Uuid::parse_str(value.trim_matches(['{', '}'])))
    .transpose()
    .map_err(|error| history_xml_error(format!("invalid instance GUID in event: {error}")))?;
    let result_code = field(
        &parsed.fields,
        &["ResultCode", "Result", "ErrorCode", "ActionResult"],
    )
    .filter(|value| !value.is_empty())
    .map(parse_result_code)
    .transpose()?;

    Ok(HistoryEvent {
        record_id,
        event_id,
        kind: event_kind(event_id),
        timestamp,
        task_path,
        instance_id,
        result_code,
        fields: parsed.fields,
        message: parsed.message,
    })
}

fn attribute_value(
    reader: &quick_xml::Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
    wanted: &str,
) -> Result<Option<String>> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| history_xml_error(error.to_string()))?;
        if local_name(attribute.key.as_ref())? == wanted {
            return attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                .map(|value| Some(value.into_owned()))
                .map_err(|error| history_xml_error(error.to_string()));
        }
    }
    Ok(None)
}

fn local_name(bytes: &[u8]) -> Result<String> {
    let bytes = bytes.rsplit(|byte| *byte == b':').next().unwrap_or(bytes);
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| history_xml_error(format!("event XML name is not UTF-8: {error}")))
}

fn field<'a>(fields: &'a BTreeMap<String, String>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|wanted| {
        fields
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.as_str())
    })
}

fn parse_result_code(value: &str) -> Result<i32> {
    let value = value.trim();
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16)
            .map(|code| i32::from_ne_bytes(code.to_ne_bytes()))
            .map_err(|error| history_xml_error(format!("invalid result code {value:?}: {error}")));
    }
    value.parse::<i64>().map_or_else(
        |error| {
            Err(history_xml_error(format!(
                "invalid result code {value:?}: {error}"
            )))
        },
        |code| {
            if let Ok(signed) = i32::try_from(code) {
                Ok(signed)
            } else {
                u32::try_from(code)
                    .map(|unsigned| i32::from_ne_bytes(unsigned.to_ne_bytes()))
                    .map_err(|_| {
                        history_xml_error(format!("result code {value:?} is out of range"))
                    })
            }
        },
    )
}

fn parse_system_time(value: &str) -> Result<SystemTime> {
    let timestamp = jiff::Timestamp::from_str(value).map_err(|error| {
        history_xml_error(format!("invalid event timestamp {value:?}: {error}"))
    })?;
    let total_nanoseconds = timestamp.as_nanosecond();
    let magnitude = total_nanoseconds.unsigned_abs();
    let seconds = u64::try_from(magnitude / 1_000_000_000)
        .map_err(|_| history_xml_error("event timestamp exceeds SystemTime"))?;
    let nanoseconds =
        u32::try_from(magnitude % 1_000_000_000).expect("subsecond nanoseconds fit u32");
    let duration = Duration::new(seconds, nanoseconds);
    if total_nanoseconds >= 0 {
        UNIX_EPOCH
            .checked_add(duration)
            .ok_or_else(|| history_xml_error("event timestamp exceeds SystemTime"))
    } else {
        UNIX_EPOCH
            .checked_sub(duration)
            .ok_or_else(|| history_xml_error("event timestamp precedes SystemTime"))
    }
}

const fn event_kind(event_id: u32) -> HistoryEventKind {
    match event_id {
        106 | 140 => HistoryEventKind::Registered,
        141 => HistoryEventKind::Deleted,
        107 | 110 | 118 | 119 => HistoryEventKind::LaunchRequested,
        100 => HistoryEventKind::Started,
        200 => HistoryEventKind::ActionStarted,
        201 => HistoryEventKind::ActionCompleted,
        102 => HistoryEventKind::Completed,
        111 | 142 => HistoryEventKind::Stopped,
        101 | 103 | 104 | 202 | 203 | 311 => HistoryEventKind::Failed,
        other => HistoryEventKind::Unknown(other),
    }
}

fn history_xml_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Xml, message).with_operation("parse Task Scheduler history event")
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, time::UNIX_EPOCH};

    use uuid::Uuid;

    use super::{HistoryEventKind, from_event_xml, parse_system_time};
    use crate::TaskPath;

    #[test]
    fn parses_task_scheduler_event_and_preserves_unknown_fields() {
        let xml = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
          <System><EventID>102</EventID><EventRecordID>42</EventRecordID>
          <TimeCreated SystemTime="2025-01-02T03:04:05.1234567Z"/></System>
          <EventData><Data Name="TaskName">\Acme\Backup</Data>
          <Data Name="InstanceId">{550e8400-e29b-41d4-a716-446655440000}</Data>
          <Data Name="ResultCode">0x80070005</Data><Data Name="Future">kept</Data></EventData>
          <RenderingInfo><Message>Task finished.</Message></RenderingInfo></Event>"#;
        let event = from_event_xml(xml).expect("event parses");
        assert_eq!(event.record_id, 42);
        assert_eq!(event.kind, HistoryEventKind::Completed);
        assert_eq!(
            event.task_path,
            Some(TaskPath::from_str("\\Acme\\Backup").expect("path"))
        );
        assert_eq!(
            event.instance_id,
            Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid"))
        );
        assert_eq!(
            event.result_code,
            Some(i32::from_ne_bytes(0x8007_0005_u32.to_ne_bytes()))
        );
        assert_eq!(event.fields.get("Future").map(String::as_str), Some("kept"));
        assert!(event.timestamp > UNIX_EPOCH);
        assert_eq!(event.message.as_deref(), Some("Task finished."));
    }

    #[test]
    fn parses_fractional_timestamp_before_the_epoch() {
        let timestamp = parse_system_time("1969-12-31T23:59:59.5Z").expect("timestamp");
        assert_eq!(
            timestamp,
            UNIX_EPOCH - std::time::Duration::from_millis(500)
        );
    }
}

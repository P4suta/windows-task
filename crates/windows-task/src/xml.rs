//! Bounded Task XML parsing, canonical writing, and raw lossless snapshots.

use std::{collections::BTreeMap, str::FromStr};

use quick_xml::{Reader, escape::unescape, events::Event};

use crate::{
    Error, ErrorKind, Result,
    model::{
        Action, Actions, BootTrigger, ComHandlerAction, DailyTrigger, EmailAction, EmailHeader,
        EventTrigger, ExecAction, IdleSettings, IdleTrigger, LogonTrigger, LogonType,
        MaintenanceSettings, Month, MonthlyDowTrigger, MonthlyTrigger, MultipleInstancesPolicy,
        NetworkSettings, Principal, PrincipalIdentity, ProcessTokenSidType, RegistrationInfo,
        RegistrationTrigger, Repetition, RequiredPrivilege, RestartPolicy, RunLevel,
        SecurityDescriptor, ServiceAccount, SessionStateChange, SessionStateChangeTrigger,
        ShowMessageAction, TaskDateTime, TaskDefinition, TaskDuration, TaskLimit,
        TaskSchemaVersion, TaskSettings, TimeTrigger, Trigger, TriggerCommon, Triggers,
        UnknownAction, UnknownTrigger, WeekOfMonth, Weekday, WeeklyTrigger, XmlExtension,
    },
};

/// Default maximum decoded XML size.
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Default maximum XML nesting depth.
pub const DEFAULT_MAX_DEPTH: usize = 64;
/// Default maximum element count.
pub const DEFAULT_MAX_NODES: usize = 100_000;

const TASK_NAMESPACE: &str = "http://schemas.microsoft.com/windows/2004/02/mit/task";

/// Resource limits for untrusted Task XML.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseLimits {
    /// Maximum decoded byte count.
    pub max_bytes: usize,
    /// Maximum nesting depth.
    pub max_depth: usize,
    /// Maximum element count.
    pub max_nodes: usize,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
        }
    }
}

/// Encoding detected for a raw task document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XmlEncoding {
    /// UTF-8, with or without a BOM.
    Utf8,
    /// Little-endian UTF-16.
    Utf16LittleEndian,
    /// Big-endian UTF-16.
    Utf16BigEndian,
}

/// Original Task XML bytes retained without normalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawTaskXml {
    bytes: Vec<u8>,
    encoding: XmlEncoding,
}

impl RawTaskXml {
    /// Validates and retains a raw Task XML document using default limits.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        Self::with_limits(bytes, ParseLimits::default())
    }

    /// Validates and retains a raw Task XML document with caller limits.
    pub fn with_limits(bytes: impl Into<Vec<u8>>, limits: ParseLimits) -> Result<Self> {
        let bytes = bytes.into();
        let (encoding, decoded) = decode_xml(&bytes, limits.max_bytes)?;
        parse_dom(&decoded, limits)?;
        Ok(Self { bytes, encoding })
    }

    /// Returns original bytes exactly as supplied.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the detected encoding.
    #[must_use]
    pub const fn encoding(&self) -> XmlEncoding {
        self.encoding
    }

    /// Decodes the document without changing it.
    pub fn decoded(&self) -> Result<String> {
        decode_xml(&self.bytes, usize::MAX).map(|(_, text)| text)
    }

    /// Parses a complete typed definition using default limits.
    pub fn definition(&self) -> Result<TaskDefinition> {
        let decoded = self.decoded()?;
        let root = parse_dom(&decoded, ParseLimits::default())?;
        definition_from_node(&root)
    }
}

/// Snapshot that always retains raw XML even when a future schema is not typed.
#[derive(Clone, Debug)]
pub struct TaskSnapshot {
    raw: RawTaskXml,
    typed: Result<TaskDefinition>,
}

impl TaskSnapshot {
    /// Parses a snapshot while retaining its original representation.
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let raw = RawTaskXml::new(bytes)?;
        let typed = raw.definition();
        Ok(Self { raw, typed })
    }

    /// Returns the exact original XML.
    #[must_use]
    pub const fn raw(&self) -> &RawTaskXml {
        &self.raw
    }

    /// Returns the typed definition or its semantic parse error.
    pub fn definition(&self) -> Result<&TaskDefinition> {
        self.typed.as_ref().map_err(Clone::clone)
    }

    /// Consumes the snapshot and returns the typed parse result.
    pub fn into_definition(self) -> Result<TaskDefinition> {
        self.typed
    }
}

/// Parses UTF-8 or UTF-16 Task XML to the complete typed model.
pub fn from_bytes(bytes: &[u8]) -> Result<TaskDefinition> {
    RawTaskXml::new(bytes.to_vec())?.definition()
}

/// Writes canonical UTF-8 Task XML with an XML declaration.
pub fn to_string(definition: &TaskDefinition) -> Result<String> {
    encoded_string(definition, "UTF-8")
}

fn encoded_string(definition: &TaskDefinition, encoding: &str) -> Result<String> {
    if !definition.validate().is_valid() {
        return Err(Error::new(
            ErrorKind::InvalidDefinition,
            "cannot serialize an invalid task definition",
        ));
    }
    Ok(write_definition(definition, encoding))
}

/// Writes canonical UTF-16LE Task XML with a BOM for native registration.
pub fn to_utf16le(definition: &TaskDefinition) -> Result<Vec<u8>> {
    let text = encoded_string(definition, "UTF-16")?;
    let mut output = Vec::with_capacity(text.len() * 2 + 2);
    output.extend_from_slice(&[0xFF, 0xFE]);
    for unit in text.encode_utf16() {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(output)
}

/// Removes an optional XML declaration before text is passed through a UTF-16
/// `BSTR`. The COM API already carries decoded characters, so retaining a byte
/// encoding declaration can make Task Scheduler reject otherwise valid XML.
#[cfg(any(windows, test))]
pub(crate) fn without_declaration(text: &str) -> &str {
    text.strip_prefix("<?xml")
        .and_then(|rest| rest.find("?>").map(|end| &rest[end + 2..]))
        .unwrap_or(text)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    name: String,
    attributes: BTreeMap<String, String>,
    children: Vec<Self>,
    text: String,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children(&self, name: &str) -> impl Iterator<Item = &Self> {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|child| child.text.as_str())
    }
}

fn decode_xml(bytes: &[u8], max_bytes: usize) -> Result<(XmlEncoding, String)> {
    if bytes.len() > max_bytes {
        return Err(xml_error(format!(
            "XML exceeds the {max_bytes}-byte parse limit"
        )));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&bytes[2..], true).map(|text| (XmlEncoding::Utf16LittleEndian, text));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&bytes[2..], false).map(|text| (XmlEncoding::Utf16BigEndian, text));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec())
        .map(|text| (XmlEncoding::Utf8, text))
        .map_err(|error| xml_error(format!("Task XML is not valid UTF-8: {error}")))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String> {
    if bytes.len() % 2 != 0 {
        return Err(xml_error(
            "UTF-16 Task XML contains an incomplete code unit",
        ));
    }
    let units = bytes.chunks_exact(2).map(|chunk| {
        if little_endian {
            u16::from_le_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], chunk[1]])
        }
    });
    std::char::decode_utf16(units)
        .collect::<std::result::Result<String, _>>()
        .map_err(|error| xml_error(format!("Task XML contains invalid UTF-16: {error}")))
}

fn parse_dom(text: &str, limits: ParseLimits) -> Result<Node> {
    if text.len() > limits.max_bytes {
        return Err(xml_error(format!(
            "decoded XML exceeds the {}-byte parse limit",
            limits.max_bytes
        )));
    }
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    let mut nodes = 0usize;

    loop {
        match reader
            .read_event()
            .map_err(|error| xml_error(error.to_string()))?
        {
            Event::Start(start) => {
                nodes += 1;
                if nodes > limits.max_nodes {
                    return Err(xml_error("XML element count exceeds the parse limit"));
                }
                if stack.len() >= limits.max_depth {
                    return Err(xml_error("XML nesting exceeds the parse limit"));
                }
                let mut attributes = BTreeMap::new();
                for attribute in start.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|error| xml_error(error.to_string()))?;
                    let key = local_name(attribute.key.as_ref())?;
                    let value = attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|error| xml_error(error.to_string()))?;
                    attributes.insert(key, value.into_owned());
                }
                stack.push(Node {
                    name: local_name(start.name().as_ref())?,
                    attributes,
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Event::Empty(empty) => {
                nodes += 1;
                if nodes > limits.max_nodes {
                    return Err(xml_error("XML element count exceeds the parse limit"));
                }
                if stack.len() >= limits.max_depth {
                    return Err(xml_error("XML nesting exceeds the parse limit"));
                }
                let mut attributes = BTreeMap::new();
                for attribute in empty.attributes().with_checks(true) {
                    let attribute = attribute.map_err(|error| xml_error(error.to_string()))?;
                    attributes.insert(
                        local_name(attribute.key.as_ref())?,
                        attribute
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .map_err(|error| xml_error(error.to_string()))?
                            .into_owned(),
                    );
                }
                let node = Node {
                    name: local_name(empty.name().as_ref())?,
                    attributes,
                    children: Vec::new(),
                    text: String::new(),
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else if root.replace(node).is_some() {
                    return Err(xml_error("Task XML contains multiple root elements"));
                }
            }
            Event::Text(event) => {
                let decoded = event
                    .xml10_content()
                    .map_err(|error| xml_error(error.to_string()))?;
                let decoded = unescape(&decoded).map_err(|error| xml_error(error.to_string()))?;
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&decoded);
                } else if !decoded.trim().is_empty() {
                    return Err(xml_error("text is not allowed outside the root element"));
                }
            }
            Event::CData(event) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(
                        &event
                            .xml10_content()
                            .map_err(|error| xml_error(error.to_string()))?,
                    );
                }
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| xml_error("unexpected XML end element"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else if root.replace(node).is_some() {
                    return Err(xml_error("Task XML contains multiple root elements"));
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(xml_error(
                    "DTD declarations and entity references are not allowed",
                ));
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
    if !stack.is_empty() {
        return Err(xml_error("Task XML ended before all elements were closed"));
    }
    root.ok_or_else(|| xml_error("Task XML has no root element"))
}

fn local_name(qualified: &[u8]) -> Result<String> {
    let qualified = std::str::from_utf8(qualified)
        .map_err(|error| xml_error(format!("XML name is not UTF-8: {error}")))?;
    Ok(qualified.rsplit(':').next().unwrap_or(qualified).into())
}

fn xml_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Xml, message)
}

fn definition_from_node(root: &Node) -> Result<TaskDefinition> {
    if root.name != "Task" {
        return Err(xml_error(format!(
            "expected Task root, found {}",
            root.name
        )));
    }
    let schema_version = root
        .attributes
        .get("version")
        .map_or(TaskSchemaVersion::V1_2, |value| {
            TaskSchemaVersion::parse(value)
        });
    let registration = root
        .child("RegistrationInfo")
        .map(parse_registration)
        .transpose()?
        .unwrap_or_default();
    let principal = root
        .child("Principals")
        .and_then(|principals| principals.child("Principal"))
        .map(parse_principal)
        .transpose()?
        .unwrap_or_default();
    let settings = root
        .child("Settings")
        .map(parse_settings)
        .transpose()?
        .unwrap_or_default();
    let triggers = root
        .child("Triggers")
        .map(parse_triggers)
        .transpose()?
        .unwrap_or_default();
    let actions = root
        .child("Actions")
        .map(parse_actions)
        .transpose()?
        .unwrap_or_default();
    let mut extensions = extensions_for(
        root,
        "Task",
        &[
            "RegistrationInfo",
            "Triggers",
            "Principals",
            "Settings",
            "Data",
            "Actions",
        ],
    );
    if let Some(node) = root.child("RegistrationInfo") {
        extensions.extend(extensions_for(
            node,
            "Task/RegistrationInfo",
            &[
                "Author",
                "Date",
                "Description",
                "Documentation",
                "Source",
                "URI",
                "Version",
                "SecurityDescriptor",
            ],
        ));
    }
    if let Some(node) = root.child("Settings") {
        extensions.extend(extensions_for(
            node,
            "Task/Settings",
            &[
                "AllowStartOnDemand",
                "RestartOnFailure",
                "MultipleInstancesPolicy",
                "DisallowStartIfOnBatteries",
                "StopIfGoingOnBatteries",
                "AllowHardTerminate",
                "StartWhenAvailable",
                "NetworkSettings",
                "RunOnlyIfNetworkAvailable",
                "ExecutionTimeLimit",
                "Enabled",
                "DeleteExpiredTaskAfter",
                "Priority",
                "IdleSettings",
                "RunOnlyIfIdle",
                "WakeToRun",
                "Hidden",
                "UseUnifiedSchedulingEngine",
                "DisallowStartOnRemoteAppSession",
                "MaintenanceSettings",
                "Volatile",
            ],
        ));
    }
    Ok(TaskDefinition {
        schema_version,
        registration,
        triggers,
        principal,
        settings,
        data: root.child_text("Data").map(str::to_owned),
        actions,
        extensions,
    })
}

fn parse_registration(node: &Node) -> Result<RegistrationInfo> {
    Ok(RegistrationInfo {
        author: owned_text(node, "Author"),
        date: node
            .child_text("Date")
            .map(TaskDateTime::parse)
            .transpose()
            .map_err(|error| xml_error(error.to_string()))?,
        description: owned_text(node, "Description"),
        documentation: owned_text(node, "Documentation"),
        source: owned_text(node, "Source"),
        version: owned_text(node, "Version"),
        uri: owned_text(node, "URI"),
        security_descriptor: node
            .child_text("SecurityDescriptor")
            .map(|text| SecurityDescriptor::from_sddl(text.to_owned()))
            .transpose()
            .map_err(|error| xml_error(error.to_string()))?,
    })
}

fn parse_principal(node: &Node) -> Result<Principal> {
    let identity = if let Some(group) = node.child_text("GroupId") {
        PrincipalIdentity::Group(group.into())
    } else if let Some(user) = node.child_text("UserId") {
        match user.to_ascii_uppercase().as_str() {
            "SYSTEM" | "S-1-5-18" => PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem),
            "LOCAL SERVICE" | "NT AUTHORITY\\LOCAL SERVICE" | "S-1-5-19" => {
                PrincipalIdentity::ServiceAccount(ServiceAccount::LocalService)
            }
            "NETWORK SERVICE" | "NT AUTHORITY\\NETWORK SERVICE" | "S-1-5-20" => {
                PrincipalIdentity::ServiceAccount(ServiceAccount::NetworkService)
            }
            _ => PrincipalIdentity::User(user.into()),
        }
    } else {
        PrincipalIdentity::None
    };
    let logon_type = match node.child_text("LogonType") {
        Some("None") => LogonType::None,
        Some("Password") => LogonType::Password,
        Some("InteractiveToken") => LogonType::InteractiveToken,
        Some("S4U") => LogonType::S4u,
        // These values are not part of the Task Scheduler XML schema, but
        // accepting them keeps older documents readable. Canonical output
        // omits them and registration supplies the native TASK_LOGON_TYPE.
        Some("Group") => LogonType::Group,
        Some("ServiceAccount") => LogonType::ServiceAccount,
        Some("InteractiveTokenOrPassword") => LogonType::InteractiveTokenOrPassword,
        Some(value) => return Err(xml_error(format!("unknown LogonType {value:?}"))),
        None => match &identity {
            PrincipalIdentity::Group(_) => LogonType::Group,
            PrincipalIdentity::ServiceAccount(_) => LogonType::ServiceAccount,
            PrincipalIdentity::None | PrincipalIdentity::User(_) => LogonType::None,
        },
    };
    let run_level = match node.child_text("RunLevel").unwrap_or("LeastPrivilege") {
        "LeastPrivilege" => RunLevel::LeastPrivilege,
        "HighestAvailable" => RunLevel::HighestAvailable,
        value => return Err(xml_error(format!("unknown RunLevel {value:?}"))),
    };
    let process_token_sid_type = match node.child_text("ProcessTokenSidType").unwrap_or("Default") {
        "Default" => ProcessTokenSidType::Default,
        "None" => ProcessTokenSidType::None,
        "Unrestricted" => ProcessTokenSidType::Unrestricted,
        value => return Err(xml_error(format!("unknown ProcessTokenSidType {value:?}"))),
    };
    let required_privileges = node
        .child("RequiredPrivileges")
        .into_iter()
        .flat_map(|container| container.children("Privilege"))
        .map(|privilege| {
            RequiredPrivilege::new(privilege.text.clone())
                .map_err(|error| xml_error(error.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Principal {
        id: node
            .attributes
            .get("id")
            .cloned()
            .unwrap_or_else(|| "Author".into()),
        display_name: owned_text(node, "DisplayName"),
        identity,
        logon_type,
        run_level,
        process_token_sid_type,
        required_privileges,
    })
}

fn parse_settings(node: &Node) -> Result<TaskSettings> {
    let mut settings = TaskSettings::default();
    settings.allow_demand_start = child_bool(node, "AllowStartOnDemand")?.unwrap_or(true);
    settings.multiple_instances = match node
        .child_text("MultipleInstancesPolicy")
        .unwrap_or("IgnoreNew")
    {
        "Parallel" => MultipleInstancesPolicy::Parallel,
        "Queue" => MultipleInstancesPolicy::Queue,
        "IgnoreNew" => MultipleInstancesPolicy::IgnoreNew,
        "StopExisting" => MultipleInstancesPolicy::StopExisting,
        value => {
            return Err(xml_error(format!(
                "unknown MultipleInstancesPolicy {value:?}"
            )));
        }
    };
    settings.disallow_start_if_on_batteries =
        child_bool(node, "DisallowStartIfOnBatteries")?.unwrap_or(true);
    settings.stop_if_going_on_batteries =
        child_bool(node, "StopIfGoingOnBatteries")?.unwrap_or(true);
    settings.allow_hard_terminate = child_bool(node, "AllowHardTerminate")?.unwrap_or(true);
    settings.start_when_available = child_bool(node, "StartWhenAvailable")?.unwrap_or(false);
    settings.execution_time_limit = node
        .child_text("ExecutionTimeLimit")
        .map(parse_limit)
        .transpose()?
        .unwrap_or_default();
    settings.enabled = child_bool(node, "Enabled")?.unwrap_or(true);
    settings.delete_expired_after = node
        .child_text("DeleteExpiredTaskAfter")
        .map(parse_duration)
        .transpose()?;
    settings.priority = node
        .child_text("Priority")
        .map(|value| parse_number(value, "Priority"))
        .transpose()?
        .unwrap_or(7);
    settings.wake_to_run = child_bool(node, "WakeToRun")?.unwrap_or(false);
    settings.hidden = child_bool(node, "Hidden")?.unwrap_or(false);
    settings.use_unified_scheduling_engine =
        child_bool(node, "UseUnifiedSchedulingEngine")?.unwrap_or(false);
    settings.disallow_start_on_remote_app_session =
        child_bool(node, "DisallowStartOnRemoteAppSession")?.unwrap_or(false);
    settings.volatile = child_bool(node, "Volatile")?.unwrap_or(false);

    if let Some(restart) = node.child("RestartOnFailure") {
        settings.restart_on_failure = Some(RestartPolicy {
            interval: required_duration(restart, "Interval")?,
            count: required_number(restart, "Count")?,
        });
    }
    if child_bool(node, "RunOnlyIfIdle")?.unwrap_or(false) || node.child("IdleSettings").is_some() {
        let idle = node.child("IdleSettings");
        let defaults = IdleSettings::default();
        settings.idle = Some(IdleSettings {
            duration: idle
                .and_then(|value| value.child_text("Duration"))
                .map(parse_duration)
                .transpose()?
                .unwrap_or(defaults.duration),
            wait_timeout: idle
                .and_then(|value| value.child_text("WaitTimeout"))
                .map(parse_duration)
                .transpose()?
                .unwrap_or(defaults.wait_timeout),
            stop_on_idle_end: idle
                .map(|value| child_bool(value, "StopOnIdleEnd"))
                .transpose()?
                .flatten()
                .unwrap_or(defaults.stop_on_idle_end),
            restart_on_idle: idle
                .map(|value| child_bool(value, "RestartOnIdle"))
                .transpose()?
                .flatten()
                .unwrap_or(defaults.restart_on_idle),
        });
    }
    if child_bool(node, "RunOnlyIfNetworkAvailable")?.unwrap_or(false)
        || node.child("NetworkSettings").is_some()
    {
        let network = node.child("NetworkSettings");
        settings.network = Some(NetworkSettings {
            id: network.and_then(|value| owned_text(value, "Id")),
            name: network.and_then(|value| owned_text(value, "Name")),
        });
    }
    if let Some(maintenance) = node.child("MaintenanceSettings") {
        settings.maintenance = Some(MaintenanceSettings {
            period: required_duration(maintenance, "Period")?,
            deadline: required_duration(maintenance, "Deadline")?,
            exclusive: child_bool(maintenance, "Exclusive")?.unwrap_or(false),
        });
    }
    Ok(settings)
}

fn parse_actions(node: &Node) -> Result<Actions> {
    let actions = node
        .children
        .iter()
        .map(|action| {
            let id = action.attributes.get("id").cloned();
            match action.name.as_str() {
                "Exec" => Ok(Action::Exec(ExecAction {
                    id,
                    command: required_text(action, "Command")?.into(),
                    arguments: owned_text(action, "Arguments"),
                    working_directory: owned_text(action, "WorkingDirectory"),
                    hide_window: child_bool(action, "HideAppWindow")?.unwrap_or(false),
                })),
                "ComHandler" => Ok(Action::ComHandler(ComHandlerAction {
                    id,
                    class_id: uuid::Uuid::parse_str(required_text(action, "ClassId")?).map_err(
                        |error| xml_error(format!("invalid ComHandler ClassId: {error}")),
                    )?,
                    data: owned_text(action, "Data"),
                })),
                "SendEmail" => Ok(Action::Email(EmailAction {
                    id,
                    server: required_text(action, "Server")?.into(),
                    subject: owned_text(action, "Subject"),
                    from: owned_text(action, "From"),
                    to: owned_text(action, "To"),
                    cc: owned_text(action, "Cc"),
                    bcc: owned_text(action, "Bcc"),
                    reply_to: owned_text(action, "ReplyTo"),
                    body: owned_text(action, "Body"),
                    attachments: action
                        .child("Attachments")
                        .into_iter()
                        .flat_map(|container| container.children("File"))
                        .map(|file| file.text.clone())
                        .collect(),
                    headers: action
                        .child("HeaderFields")
                        .into_iter()
                        .flat_map(|container| container.children("HeaderField"))
                        .map(|header| {
                            Ok(EmailHeader {
                                name: required_text(header, "Name")?.into(),
                                value: required_text(header, "Value")?.into(),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                })),
                "ShowMessage" => Ok(Action::ShowMessage(ShowMessageAction {
                    id,
                    title: owned_text(action, "Title"),
                    body: required_text(action, "Body")?.into(),
                })),
                kind => Ok(Action::Unknown(UnknownAction {
                    kind: kind.into(),
                    xml: node_to_xml(action),
                })),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Actions::new(actions).map_err(|error| xml_error(error.to_string()))
}

fn parse_triggers(node: &Node) -> Result<Triggers> {
    let values = node
        .children
        .iter()
        .map(parse_trigger)
        .collect::<Result<Vec<_>>>()?;
    Triggers::new(values).map_err(|error| xml_error(error.to_string()))
}

fn parse_trigger(node: &Node) -> Result<Trigger> {
    let common = parse_trigger_common(node)?;
    Ok(match node.name.as_str() {
        "BootTrigger" => Trigger::Boot(BootTrigger {
            common,
            delay: optional_duration(node, "Delay")?,
        }),
        "RegistrationTrigger" => Trigger::Registration(RegistrationTrigger {
            common,
            delay: optional_duration(node, "Delay")?,
        }),
        "IdleTrigger" => Trigger::Idle(IdleTrigger { common }),
        "TimeTrigger" => Trigger::Time(TimeTrigger {
            common,
            random_delay: optional_duration(node, "RandomDelay")?,
        }),
        "EventTrigger" => Trigger::Event(EventTrigger {
            common,
            subscription: required_text(node, "Subscription")?.into(),
            value_queries: node
                .child("ValueQueries")
                .into_iter()
                .flat_map(|container| container.children("Value"))
                .filter_map(|value| {
                    value
                        .attributes
                        .get("name")
                        .map(|name| (name.clone(), value.text.clone()))
                })
                .collect(),
            delay: optional_duration(node, "Delay")?,
            period_of_occurrence: optional_duration(node, "PeriodOfOccurrence")?,
            number_of_occurrences: node
                .child_text("NumberOfOccurrences")
                .map(|value| parse_number(value, "NumberOfOccurrences"))
                .transpose()?,
            matching_element: owned_text(node, "MatchingElement"),
        }),
        "LogonTrigger" => Trigger::Logon(LogonTrigger {
            common,
            user_id: owned_text(node, "UserId"),
            delay: optional_duration(node, "Delay")?,
        }),
        "SessionStateChangeTrigger" => Trigger::SessionStateChange(SessionStateChangeTrigger {
            common,
            state_change: match required_text(node, "StateChange")? {
                "ConsoleConnect" => SessionStateChange::ConsoleConnect,
                "ConsoleDisconnect" => SessionStateChange::ConsoleDisconnect,
                "RemoteConnect" => SessionStateChange::RemoteConnect,
                "RemoteDisconnect" => SessionStateChange::RemoteDisconnect,
                "SessionLock" => SessionStateChange::SessionLock,
                "SessionUnlock" => SessionStateChange::SessionUnlock,
                value => return Err(xml_error(format!("unknown session state change {value:?}"))),
            },
            user_id: owned_text(node, "UserId"),
            delay: optional_duration(node, "Delay")?,
        }),
        "CalendarTrigger" => parse_calendar_trigger(node, common)?,
        kind => Trigger::Unknown(UnknownTrigger {
            kind: kind.into(),
            xml: node_to_xml(node),
        }),
    })
}

fn parse_trigger_common(node: &Node) -> Result<TriggerCommon> {
    Ok(TriggerCommon {
        id: node.attributes.get("id").cloned(),
        start_boundary: node
            .child_text("StartBoundary")
            .map(TaskDateTime::parse)
            .transpose()
            .map_err(|error| xml_error(error.to_string()))?,
        end_boundary: node
            .child_text("EndBoundary")
            .map(TaskDateTime::parse)
            .transpose()
            .map_err(|error| xml_error(error.to_string()))?,
        enabled: child_bool(node, "Enabled")?.unwrap_or(true),
        execution_time_limit: node
            .child_text("ExecutionTimeLimit")
            .map(parse_limit)
            .transpose()?,
        repetition: node.child("Repetition").map(parse_repetition).transpose()?,
    })
}

fn parse_repetition(node: &Node) -> Result<Repetition> {
    Ok(Repetition {
        interval: required_duration(node, "Interval")?,
        duration: optional_duration(node, "Duration")?,
        stop_at_duration_end: child_bool(node, "StopAtDurationEnd")?.unwrap_or(false),
    })
}

fn parse_calendar_trigger(node: &Node, common: TriggerCommon) -> Result<Trigger> {
    let schedule = node
        .child("ScheduleByDay")
        .or_else(|| node.child("ScheduleByWeek"))
        .or_else(|| node.child("ScheduleByMonth"))
        .or_else(|| node.child("ScheduleByMonthDayOfWeek"))
        .ok_or_else(|| xml_error("CalendarTrigger has no schedule"))?;
    let random_delay = optional_duration(node, "RandomDelay")?;
    match schedule.name.as_str() {
        "ScheduleByDay" => Ok(Trigger::Daily(DailyTrigger {
            common,
            days_interval: required_number(schedule, "DaysInterval")?,
            random_delay,
        })),
        "ScheduleByWeek" => Ok(Trigger::Weekly(WeeklyTrigger {
            common,
            weeks_interval: required_number(schedule, "WeeksInterval")?,
            days_of_week: parse_weekdays(required_child(schedule, "DaysOfWeek")?),
            random_delay,
        })),
        "ScheduleByMonth" => Ok(Trigger::Monthly(MonthlyTrigger {
            common,
            days_of_month: parse_day_numbers(required_child(schedule, "DaysOfMonth")?)?,
            months: parse_months(required_child(schedule, "Months")?),
            run_on_last_day: child_bool(schedule, "RunOnLastDayOfMonth")?.unwrap_or(false),
            random_delay,
        })),
        "ScheduleByMonthDayOfWeek" => Ok(Trigger::MonthlyDow(MonthlyDowTrigger {
            common,
            weeks_of_month: parse_weeks(required_child(schedule, "Weeks")?),
            days_of_week: parse_weekdays(required_child(schedule, "DaysOfWeek")?),
            months: parse_months(required_child(schedule, "Months")?),
            run_on_last_week: child_bool(schedule, "RunOnLastWeekOfMonth")?.unwrap_or(false),
            random_delay,
        })),
        _ => Err(xml_error("unsupported calendar schedule")),
    }
}

fn parse_weekdays(node: &Node) -> std::collections::BTreeSet<Weekday> {
    node.children
        .iter()
        .filter_map(|child| match child.name.as_str() {
            "Sunday" => Some(Weekday::Sunday),
            "Monday" => Some(Weekday::Monday),
            "Tuesday" => Some(Weekday::Tuesday),
            "Wednesday" => Some(Weekday::Wednesday),
            "Thursday" => Some(Weekday::Thursday),
            "Friday" => Some(Weekday::Friday),
            "Saturday" => Some(Weekday::Saturday),
            _ => None,
        })
        .collect()
}

fn parse_months(node: &Node) -> std::collections::BTreeSet<Month> {
    node.children
        .iter()
        .filter_map(|child| match child.name.as_str() {
            "January" => Some(Month::January),
            "February" => Some(Month::February),
            "March" => Some(Month::March),
            "April" => Some(Month::April),
            "May" => Some(Month::May),
            "June" => Some(Month::June),
            "July" => Some(Month::July),
            "August" => Some(Month::August),
            "September" => Some(Month::September),
            "October" => Some(Month::October),
            "November" => Some(Month::November),
            "December" => Some(Month::December),
            _ => None,
        })
        .collect()
}

fn parse_weeks(node: &Node) -> std::collections::BTreeSet<WeekOfMonth> {
    node.children
        .iter()
        .filter_map(|child| match child.name.as_str() {
            "Week" => match child.text.as_str() {
                "1" => Some(WeekOfMonth::First),
                "2" => Some(WeekOfMonth::Second),
                "3" => Some(WeekOfMonth::Third),
                "4" => Some(WeekOfMonth::Fourth),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn parse_day_numbers(node: &Node) -> Result<std::collections::BTreeSet<u8>> {
    node.children("Day")
        .map(|child| parse_number(&child.text, "Day"))
        .collect()
}

fn owned_text(node: &Node, child: &str) -> Option<String> {
    node.child_text(child).map(str::to_owned)
}

fn required_text<'a>(node: &'a Node, child: &str) -> Result<&'a str> {
    node.child_text(child)
        .ok_or_else(|| xml_error(format!("{} requires {child}", node.name)))
}

fn required_child<'a>(node: &'a Node, child: &str) -> Result<&'a Node> {
    node.child(child)
        .ok_or_else(|| xml_error(format!("{} requires {child}", node.name)))
}

fn child_bool(node: &Node, child: &str) -> Result<Option<bool>> {
    node.child_text(child)
        .map(|value| match value {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(xml_error(format!("{child} is not a boolean: {value:?}"))),
        })
        .transpose()
}

fn parse_number<T>(value: &str, name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error| xml_error(format!("invalid {name} value {value:?}: {error}")))
}

fn required_number<T>(node: &Node, child: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    parse_number(required_text(node, child)?, child)
}

fn parse_duration(value: &str) -> Result<TaskDuration> {
    TaskDuration::parse(value).map_err(|error| xml_error(error.to_string()))
}

fn required_duration(node: &Node, child: &str) -> Result<TaskDuration> {
    parse_duration(required_text(node, child)?)
}

fn optional_duration(node: &Node, child: &str) -> Result<Option<TaskDuration>> {
    node.child_text(child).map(parse_duration).transpose()
}

fn parse_limit(value: &str) -> Result<TaskLimit> {
    if value == "PT0S" {
        Ok(TaskLimit::Unlimited)
    } else {
        parse_duration(value).map(TaskLimit::Finite)
    }
}

fn extensions_for(node: &Node, parent: &str, known: &[&str]) -> Vec<XmlExtension> {
    node.children
        .iter()
        .enumerate()
        .filter(|(_, child)| !known.contains(&child.name.as_str()))
        .map(|(ordinal, child)| XmlExtension {
            parent: parent.into(),
            ordinal,
            xml: node_to_xml(child),
        })
        .collect()
}

fn node_to_xml(node: &Node) -> String {
    let mut output = String::new();
    output.push('<');
    output.push_str(&node.name);
    for (key, value) in &node.attributes {
        output.push(' ');
        output.push_str(key);
        output.push_str("=\"");
        output.push_str(&escape_attribute(value));
        output.push('"');
    }
    if node.children.is_empty() && node.text.is_empty() {
        output.push_str(" />");
        return output;
    }
    output.push('>');
    output.push_str(&escape_text(&node.text));
    for child in &node.children {
        output.push_str(&node_to_xml(child));
    }
    output.push_str("</");
    output.push_str(&node.name);
    output.push('>');
    output
}

fn write_definition(definition: &TaskDefinition, encoding: &str) -> String {
    let mut root_children = Vec::new();
    if definition.registration != RegistrationInfo::default()
        || has_extensions(definition, "Task/RegistrationInfo")
    {
        root_children.push(write_registration(definition));
    }
    if !definition.triggers.is_empty() {
        root_children.push(container(
            "Triggers",
            definition
                .triggers
                .as_slice()
                .iter()
                .map(write_trigger)
                .collect(),
            &[],
            "",
        ));
    }
    root_children.push(write_principal(&definition.principal));
    root_children.push(write_settings(definition));
    if let Some(data) = &definition.data {
        root_children.push(element("Data", data));
    }
    root_children.push(write_actions(definition));
    let children = merge_extensions("Task", root_children, &definition.extensions);
    format!(
        "<?xml version=\"1.0\" encoding=\"{encoding}\"?>\r\n<Task version=\"{}\" xmlns=\"{}\">{}</Task>\r\n",
        escape_attribute(definition.schema_version.as_str()),
        TASK_NAMESPACE,
        children.join("")
    )
}

fn write_registration(definition: &TaskDefinition) -> String {
    let registration = &definition.registration;
    let mut children = Vec::new();
    push_optional(
        &mut children,
        "Date",
        registration.date.as_ref().map(ToString::to_string),
    );
    push_optional(&mut children, "Author", registration.author.clone());
    push_optional(&mut children, "Version", registration.version.clone());
    push_optional(
        &mut children,
        "Description",
        registration.description.clone(),
    );
    push_optional(
        &mut children,
        "Documentation",
        registration.documentation.clone(),
    );
    push_optional(&mut children, "URI", registration.uri.clone());
    push_optional(
        &mut children,
        "SecurityDescriptor",
        registration
            .security_descriptor
            .as_ref()
            .map(|value| value.as_sddl().to_owned()),
    );
    push_optional(&mut children, "Source", registration.source.clone());
    container(
        "RegistrationInfo",
        merge_extensions("Task/RegistrationInfo", children, &definition.extensions),
        &[],
        "",
    )
}

fn write_principal(principal: &Principal) -> String {
    let mut children = Vec::new();
    match &principal.identity {
        PrincipalIdentity::None => {}
        PrincipalIdentity::User(user) => children.push(element("UserId", user)),
        PrincipalIdentity::Group(group) => children.push(element("GroupId", group)),
        PrincipalIdentity::ServiceAccount(account) => children.push(element(
            "UserId",
            match account {
                ServiceAccount::LocalSystem => "SYSTEM",
                ServiceAccount::LocalService => "NT AUTHORITY\\LOCAL SERVICE",
                ServiceAccount::NetworkService => "NT AUTHORITY\\NETWORK SERVICE",
            },
        )),
    }
    if let Some(display_name) = &principal.display_name {
        children.push(element("DisplayName", display_name));
    }
    let xml_logon_type = match principal.logon_type {
        LogonType::None | LogonType::Group | LogonType::ServiceAccount => None,
        LogonType::Password => Some("Password"),
        LogonType::InteractiveToken => Some("InteractiveToken"),
        LogonType::S4u => Some("S4U"),
        LogonType::InteractiveTokenOrPassword => Some("InteractiveTokenOrPassword"),
    };
    if let Some(logon_type) = xml_logon_type {
        children.push(element("LogonType", logon_type));
    }
    children.push(element(
        "RunLevel",
        match principal.run_level {
            RunLevel::LeastPrivilege => "LeastPrivilege",
            RunLevel::HighestAvailable => "HighestAvailable",
        },
    ));
    if principal.process_token_sid_type != ProcessTokenSidType::Default {
        children.push(element(
            "ProcessTokenSidType",
            match principal.process_token_sid_type {
                ProcessTokenSidType::Default => "Default",
                ProcessTokenSidType::None => "None",
                ProcessTokenSidType::Unrestricted => "Unrestricted",
            },
        ));
    }
    if !principal.required_privileges.is_empty() {
        children.push(container(
            "RequiredPrivileges",
            principal
                .required_privileges
                .iter()
                .map(|privilege| element("Privilege", privilege.as_str()))
                .collect(),
            &[],
            "",
        ));
    }
    let principal_xml = container("Principal", children, &[("id", principal.id.as_str())], "");
    container("Principals", vec![principal_xml], &[], "")
}

fn write_settings(definition: &TaskDefinition) -> String {
    let settings = &definition.settings;
    let mut children = vec![
        bool_element("AllowStartOnDemand", settings.allow_demand_start),
        bool_element(
            "DisallowStartIfOnBatteries",
            settings.disallow_start_if_on_batteries,
        ),
        bool_element(
            "StopIfGoingOnBatteries",
            settings.stop_if_going_on_batteries,
        ),
    ];
    if let Some(restart) = &settings.restart_on_failure {
        children.push(container(
            "RestartOnFailure",
            vec![
                element("Interval", &restart.interval.to_string()),
                element("Count", &restart.count.to_string()),
            ],
            &[],
            "",
        ));
    }
    children.extend([
        element(
            "MultipleInstancesPolicy",
            match settings.multiple_instances {
                MultipleInstancesPolicy::Parallel => "Parallel",
                MultipleInstancesPolicy::Queue => "Queue",
                MultipleInstancesPolicy::IgnoreNew => "IgnoreNew",
                MultipleInstancesPolicy::StopExisting => "StopExisting",
            },
        ),
        bool_element("StartWhenAvailable", settings.start_when_available),
        bool_element("AllowHardTerminate", settings.allow_hard_terminate),
    ]);
    if let Some(idle) = &settings.idle {
        children.push(container(
            "IdleSettings",
            vec![
                element("Duration", &idle.duration.to_string()),
                element("WaitTimeout", &idle.wait_timeout.to_string()),
                bool_element("StopOnIdleEnd", idle.stop_on_idle_end),
                bool_element("RestartOnIdle", idle.restart_on_idle),
            ],
            &[],
            "",
        ));
    }
    children.push(bool_element("RunOnlyIfIdle", settings.idle.is_some()));
    if let Some(network) = &settings.network {
        let mut network_children = Vec::new();
        push_optional(&mut network_children, "Name", network.name.clone());
        push_optional(&mut network_children, "Id", network.id.clone());
        children.push(container("NetworkSettings", network_children, &[], ""));
    }
    children.extend([
        bool_element("RunOnlyIfNetworkAvailable", settings.network.is_some()),
        bool_element("WakeToRun", settings.wake_to_run),
        bool_element("Enabled", settings.enabled),
        bool_element("Hidden", settings.hidden),
    ]);
    children.extend(optional_element(
        "DeleteExpiredTaskAfter",
        settings.delete_expired_after.map(|value| value.to_string()),
    ));
    children.extend([
        element(
            "ExecutionTimeLimit",
            &limit_text(settings.execution_time_limit),
        ),
        element("Priority", &settings.priority.to_string()),
    ]);
    if settings.use_unified_scheduling_engine {
        children.push(bool_element("UseUnifiedSchedulingEngine", true));
    }
    if settings.disallow_start_on_remote_app_session {
        children.push(bool_element("DisallowStartOnRemoteAppSession", true));
    }
    if let Some(maintenance) = &settings.maintenance {
        children.push(container(
            "MaintenanceSettings",
            vec![
                element("Period", &maintenance.period.to_string()),
                element("Deadline", &maintenance.deadline.to_string()),
                bool_element("Exclusive", maintenance.exclusive),
            ],
            &[],
            "",
        ));
    }
    if settings.volatile {
        children.push(bool_element("Volatile", true));
    }
    container(
        "Settings",
        merge_extensions("Task/Settings", children, &definition.extensions),
        &[],
        "",
    )
}

fn write_actions(definition: &TaskDefinition) -> String {
    let children = definition
        .actions
        .as_slice()
        .iter()
        .map(|action| match action {
            Action::Exec(action) => {
                let mut children = vec![element("Command", &action.command)];
                push_optional(&mut children, "Arguments", action.arguments.clone());
                push_optional(
                    &mut children,
                    "WorkingDirectory",
                    action.working_directory.clone(),
                );
                if action.hide_window {
                    children.push(bool_element("HideAppWindow", true));
                }
                container_with_id("Exec", children, action.id.as_deref())
            }
            Action::ComHandler(action) => {
                let mut children = vec![element(
                    "ClassId",
                    &action.class_id.hyphenated().to_string(),
                )];
                push_optional(&mut children, "Data", action.data.clone());
                container_with_id("ComHandler", children, action.id.as_deref())
            }
            Action::Email(action) => {
                let mut children = vec![element("Server", &action.server)];
                push_optional(&mut children, "Subject", action.subject.clone());
                push_optional(&mut children, "To", action.to.clone());
                push_optional(&mut children, "Cc", action.cc.clone());
                push_optional(&mut children, "Bcc", action.bcc.clone());
                push_optional(&mut children, "ReplyTo", action.reply_to.clone());
                push_optional(&mut children, "From", action.from.clone());
                if !action.headers.is_empty() {
                    children.push(container(
                        "HeaderFields",
                        action
                            .headers
                            .iter()
                            .map(|header| {
                                container(
                                    "HeaderField",
                                    vec![
                                        element("Name", &header.name),
                                        element("Value", &header.value),
                                    ],
                                    &[],
                                    "",
                                )
                            })
                            .collect(),
                        &[],
                        "",
                    ));
                }
                push_optional(&mut children, "Body", action.body.clone());
                if !action.attachments.is_empty() {
                    children.push(container(
                        "Attachments",
                        action
                            .attachments
                            .iter()
                            .map(|file| element("File", file))
                            .collect(),
                        &[],
                        "",
                    ));
                }
                container_with_id("SendEmail", children, action.id.as_deref())
            }
            Action::ShowMessage(action) => {
                let mut children = Vec::new();
                push_optional(&mut children, "Title", action.title.clone());
                children.push(element("Body", &action.body));
                container_with_id("ShowMessage", children, action.id.as_deref())
            }
            Action::Unknown(action) => action.xml.clone(),
        })
        .collect();
    container(
        "Actions",
        children,
        &[("Context", definition.principal.id.as_str())],
        "",
    )
}

fn write_trigger(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Boot(trigger) => write_simple_trigger(
            "BootTrigger",
            &trigger.common,
            optional_element("Delay", trigger.delay.map(|value| value.to_string())),
        ),
        Trigger::Registration(trigger) => write_simple_trigger(
            "RegistrationTrigger",
            &trigger.common,
            optional_element("Delay", trigger.delay.map(|value| value.to_string())),
        ),
        Trigger::Idle(trigger) => write_simple_trigger("IdleTrigger", &trigger.common, Vec::new()),
        Trigger::Time(trigger) => write_simple_trigger(
            "TimeTrigger",
            &trigger.common,
            optional_element(
                "RandomDelay",
                trigger.random_delay.map(|value| value.to_string()),
            ),
        ),
        Trigger::Event(trigger) => {
            let mut specific =
                optional_element("Delay", trigger.delay.map(|value| value.to_string()));
            specific.push(element("Subscription", &trigger.subscription));
            if !trigger.value_queries.is_empty() {
                specific.push(container(
                    "ValueQueries",
                    trigger
                        .value_queries
                        .iter()
                        .map(|(name, value)| {
                            container("Value", Vec::new(), &[("name", name)], value)
                        })
                        .collect(),
                    &[],
                    "",
                ));
            }
            specific.extend(optional_element(
                "PeriodOfOccurrence",
                trigger.period_of_occurrence.map(|value| value.to_string()),
            ));
            specific.extend(optional_element(
                "NumberOfOccurrences",
                trigger.number_of_occurrences.map(|value| value.to_string()),
            ));
            specific.extend(optional_element(
                "MatchingElement",
                trigger.matching_element.clone(),
            ));
            write_simple_trigger("EventTrigger", &trigger.common, specific)
        }
        Trigger::Logon(trigger) => {
            let mut specific = optional_element("UserId", trigger.user_id.clone());
            specific.extend(optional_element(
                "Delay",
                trigger.delay.map(|value| value.to_string()),
            ));
            write_simple_trigger("LogonTrigger", &trigger.common, specific)
        }
        Trigger::SessionStateChange(trigger) => {
            let mut specific = vec![element(
                "StateChange",
                match trigger.state_change {
                    SessionStateChange::ConsoleConnect => "ConsoleConnect",
                    SessionStateChange::ConsoleDisconnect => "ConsoleDisconnect",
                    SessionStateChange::RemoteConnect => "RemoteConnect",
                    SessionStateChange::RemoteDisconnect => "RemoteDisconnect",
                    SessionStateChange::SessionLock => "SessionLock",
                    SessionStateChange::SessionUnlock => "SessionUnlock",
                },
            )];
            specific.extend(optional_element("UserId", trigger.user_id.clone()));
            specific.extend(optional_element(
                "Delay",
                trigger.delay.map(|value| value.to_string()),
            ));
            write_simple_trigger("SessionStateChangeTrigger", &trigger.common, specific)
        }
        Trigger::Daily(trigger) => write_calendar_trigger(
            &trigger.common,
            container(
                "ScheduleByDay",
                vec![element("DaysInterval", &trigger.days_interval.to_string())],
                &[],
                "",
            ),
            trigger.random_delay,
        ),
        Trigger::Weekly(trigger) => write_calendar_trigger(
            &trigger.common,
            container(
                "ScheduleByWeek",
                vec![
                    element("WeeksInterval", &trigger.weeks_interval.to_string()),
                    write_weekdays(&trigger.days_of_week),
                ],
                &[],
                "",
            ),
            trigger.random_delay,
        ),
        Trigger::Monthly(trigger) => write_calendar_trigger(
            &trigger.common,
            container(
                "ScheduleByMonth",
                vec![
                    container(
                        "DaysOfMonth",
                        trigger
                            .days_of_month
                            .iter()
                            .map(|day| element("Day", &day.to_string()))
                            .collect(),
                        &[],
                        "",
                    ),
                    write_months(&trigger.months),
                    bool_element("RunOnLastDayOfMonth", trigger.run_on_last_day),
                ],
                &[],
                "",
            ),
            trigger.random_delay,
        ),
        Trigger::MonthlyDow(trigger) => write_calendar_trigger(
            &trigger.common,
            container(
                "ScheduleByMonthDayOfWeek",
                vec![
                    container(
                        "Weeks",
                        trigger
                            .weeks_of_month
                            .iter()
                            .map(|week| {
                                element(
                                    "Week",
                                    match week {
                                        WeekOfMonth::First => "1",
                                        WeekOfMonth::Second => "2",
                                        WeekOfMonth::Third => "3",
                                        WeekOfMonth::Fourth => "4",
                                    },
                                )
                            })
                            .collect(),
                        &[],
                        "",
                    ),
                    write_weekdays(&trigger.days_of_week),
                    write_months(&trigger.months),
                    bool_element("RunOnLastWeekOfMonth", trigger.run_on_last_week),
                ],
                &[],
                "",
            ),
            trigger.random_delay,
        ),
        Trigger::Unknown(trigger) => trigger.xml.clone(),
    }
}

fn write_simple_trigger(name: &str, common: &TriggerCommon, mut specific: Vec<String>) -> String {
    let mut children = trigger_common_elements(common);
    children.append(&mut specific);
    container_with_id(name, children, common.id.as_deref())
}

fn write_calendar_trigger(
    common: &TriggerCommon,
    schedule: String,
    random_delay: Option<TaskDuration>,
) -> String {
    let mut children = trigger_common_elements(common);
    children.push(schedule);
    children.extend(optional_element(
        "RandomDelay",
        random_delay.map(|value| value.to_string()),
    ));
    container_with_id("CalendarTrigger", children, common.id.as_deref())
}

fn trigger_common_elements(common: &TriggerCommon) -> Vec<String> {
    let mut children = Vec::new();
    if let Some(repetition) = &common.repetition {
        let mut repetition_children = vec![element("Interval", &repetition.interval.to_string())];
        repetition_children.extend(optional_element(
            "Duration",
            repetition.duration.map(|value| value.to_string()),
        ));
        repetition_children.push(bool_element(
            "StopAtDurationEnd",
            repetition.stop_at_duration_end,
        ));
        children.push(container("Repetition", repetition_children, &[], ""));
    }
    children.extend(optional_element(
        "StartBoundary",
        common.start_boundary.as_ref().map(ToString::to_string),
    ));
    children.extend(optional_element(
        "EndBoundary",
        common.end_boundary.as_ref().map(ToString::to_string),
    ));
    children.extend(optional_element(
        "ExecutionTimeLimit",
        common.execution_time_limit.map(limit_text),
    ));
    children.push(bool_element("Enabled", common.enabled));
    children
}

fn write_weekdays(days: &std::collections::BTreeSet<Weekday>) -> String {
    container(
        "DaysOfWeek",
        days.iter()
            .map(|day| {
                empty_element(match day {
                    Weekday::Sunday => "Sunday",
                    Weekday::Monday => "Monday",
                    Weekday::Tuesday => "Tuesday",
                    Weekday::Wednesday => "Wednesday",
                    Weekday::Thursday => "Thursday",
                    Weekday::Friday => "Friday",
                    Weekday::Saturday => "Saturday",
                })
            })
            .collect(),
        &[],
        "",
    )
}

fn write_months(months: &std::collections::BTreeSet<Month>) -> String {
    container(
        "Months",
        months
            .iter()
            .map(|month| {
                empty_element(match month {
                    Month::January => "January",
                    Month::February => "February",
                    Month::March => "March",
                    Month::April => "April",
                    Month::May => "May",
                    Month::June => "June",
                    Month::July => "July",
                    Month::August => "August",
                    Month::September => "September",
                    Month::October => "October",
                    Month::November => "November",
                    Month::December => "December",
                })
            })
            .collect(),
        &[],
        "",
    )
}

fn has_extensions(definition: &TaskDefinition, parent: &str) -> bool {
    definition
        .extensions
        .iter()
        .any(|extension| extension.parent == parent)
}

fn merge_extensions(parent: &str, known: Vec<String>, extensions: &[XmlExtension]) -> Vec<String> {
    let mut output = known;
    let mut matching: Vec<_> = extensions
        .iter()
        .filter(|extension| extension.parent == parent)
        .collect();
    matching.sort_by_key(|extension| extension.ordinal);
    for (offset, extension) in matching.into_iter().enumerate() {
        output.insert(
            (extension.ordinal + offset).min(output.len()),
            extension.xml.clone(),
        );
    }
    output
}

fn container(name: &str, children: Vec<String>, attributes: &[(&str, &str)], text: &str) -> String {
    let mut output = String::new();
    output.push('<');
    output.push_str(name);
    for (key, value) in attributes {
        output.push(' ');
        output.push_str(key);
        output.push_str("=\"");
        output.push_str(&escape_attribute(value));
        output.push('"');
    }
    output.push('>');
    output.push_str(&escape_text(text));
    for child in children {
        output.push_str(&child);
    }
    output.push_str("</");
    output.push_str(name);
    output.push('>');
    output
}

fn container_with_id(name: &str, children: Vec<String>, id: Option<&str>) -> String {
    match id {
        Some(id) => container(name, children, &[("id", id)], ""),
        None => container(name, children, &[], ""),
    }
}

fn element(name: &str, text: &str) -> String {
    container(name, Vec::new(), &[], text)
}

fn empty_element(name: &str) -> String {
    format!("<{name} />")
}

fn bool_element(name: &str, value: bool) -> String {
    element(name, if value { "true" } else { "false" })
}

fn optional_element(name: &str, value: Option<String>) -> Vec<String> {
    value
        .into_iter()
        .map(|value| element(name, &value))
        .collect()
}

fn push_optional(children: &mut Vec<String>, name: &str, value: Option<String>) {
    children.extend(optional_element(name, value));
}

fn limit_text(limit: TaskLimit) -> String {
    match limit {
        TaskLimit::Finite(duration) => duration.to_string(),
        TaskLimit::Unlimited => "PT0S".into(),
    }
}

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use crate::model::{
        Action, ExecAction, LogonType, PrincipalIdentity, ServiceAccount, TaskDefinition,
        XmlExtension,
    };

    use super::{ParseLimits, RawTaskXml, from_bytes, to_string, to_utf16le, without_declaration};

    #[test]
    fn minimal_definition_round_trips() {
        let definition = TaskDefinition::new(Action::Exec(
            ExecAction::new("cmd.exe").args(["/c", "echo hello"]),
        ));
        let xml = to_string(&definition).expect("serialize");
        let decoded = from_bytes(xml.as_bytes()).expect("parse");
        assert_eq!(decoded, definition);
    }

    #[test]
    fn utf16le_round_trips() {
        let definition = TaskDefinition::new(Action::Exec(ExecAction::new("pwsh.exe")));
        let xml = to_utf16le(&definition).expect("UTF-16");
        let declaration = String::from_utf16(
            &xml[2..]
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>(),
        )
        .expect("valid UTF-16");
        assert!(declaration.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));
        assert_eq!(from_bytes(&xml).expect("parse"), definition);
    }

    #[test]
    fn strips_the_encoding_declaration_for_bstr_transport() {
        let definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        let xml = to_string(&definition).expect("serialize");
        let transported = without_declaration(&xml);
        assert!(!transported.contains("encoding="));
        assert!(transported.trim_start().starts_with("<Task "));
        assert_eq!(
            from_bytes(transported.as_bytes()).expect("parse"),
            definition
        );
    }

    #[test]
    fn native_principal_logon_types_are_omitted_and_inferred() {
        let cases = [
            (
                PrincipalIdentity::Group("BUILTIN\\Users".into()),
                LogonType::Group,
            ),
            (
                PrincipalIdentity::ServiceAccount(ServiceAccount::LocalSystem),
                LogonType::ServiceAccount,
            ),
        ];

        for (identity, logon_type) in cases {
            let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
            definition.principal.identity = identity;
            definition.principal.logon_type = logon_type;

            let xml = to_string(&definition).expect("serialize");
            assert!(!xml.contains("<LogonType>"));
            assert_eq!(from_bytes(xml.as_bytes()).expect("parse"), definition);
        }
    }

    #[test]
    fn rejects_doctype_and_bounded_depth() {
        RawTaskXml::new(b"<!DOCTYPE Task><Task />".to_vec()).expect_err("DOCTYPE is rejected");
        let limits = ParseLimits {
            max_bytes: 100,
            max_depth: 1,
            max_nodes: 10,
        };
        RawTaskXml::with_limits(b"<Task><Child /></Task>".to_vec(), limits)
            .expect_err("depth limit is enforced");
    }

    #[test]
    fn root_extension_survives_typed_round_trip() {
        let mut definition = TaskDefinition::new(Action::Exec(ExecAction::new("cmd.exe")));
        definition.extensions.push(XmlExtension {
            parent: "Task".into(),
            ordinal: 1,
            xml: "<FutureSetting xmlns=\"urn:future\"><Value>yes</Value></FutureSetting>".into(),
        });
        let xml = to_string(&definition).expect("serialize");
        let decoded = from_bytes(xml.as_bytes()).expect("parse");
        assert_eq!(decoded.extensions.len(), 1);
        assert_eq!(decoded.extensions[0].parent, "Task");
    }
}

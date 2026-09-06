use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::MAX_RUN_ARGUMENTS;

/// One executable action.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ExecAction {
    /// Optional action identifier, unique within the task.
    pub id: Option<String>,
    /// Executable or document path.
    pub command: String,
    /// Raw Windows command-line tail. This does not include `command`.
    pub arguments: Option<String>,
    /// Optional working directory.
    pub working_directory: Option<String>,
    /// Requests a hidden window on scheduler versions that support it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub hide_window: bool,
}

impl ExecAction {
    /// Creates an executable action.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            id: None,
            command: command.into(),
            arguments: None,
            working_directory: None,
            hide_window: false,
        }
    }

    /// Replaces the raw command-line tail.
    #[must_use]
    pub fn raw_arguments(mut self, arguments: impl Into<String>) -> Self {
        self.arguments = Some(arguments.into());
        self
    }

    /// Quotes a sequence using the `CommandLineToArgvW`/MSVC convention.
    #[must_use]
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.arguments = Some(
            arguments
                .into_iter()
                .map(|argument| quote_windows_argument(argument.as_ref()))
                .collect::<Vec<_>>()
                .join(" "),
        );
        self
    }
}

/// A COM handler action.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ComHandlerAction {
    /// Optional action identifier, unique within the task.
    pub id: Option<String>,
    /// Registered COM class identifier.
    pub class_id: Uuid,
    /// Opaque handler data passed to `ITaskHandler::Start`.
    pub data: Option<String>,
}

/// One named email header from the legacy email action.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct EmailHeader {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

/// Legacy Task Scheduler email action.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct EmailAction {
    /// Optional action identifier.
    pub id: Option<String>,
    /// SMTP server.
    pub server: String,
    /// Subject line.
    pub subject: Option<String>,
    /// Sender.
    pub from: Option<String>,
    /// Primary recipients.
    pub to: Option<String>,
    /// Carbon-copy recipients.
    pub cc: Option<String>,
    /// Blind-carbon-copy recipients.
    pub bcc: Option<String>,
    /// Reply-to address.
    pub reply_to: Option<String>,
    /// Message body.
    pub body: Option<String>,
    /// Attachment paths.
    #[cfg_attr(feature = "serde", serde(default))]
    pub attachments: Vec<String>,
    /// Additional headers.
    #[cfg_attr(feature = "serde", serde(default))]
    pub headers: Vec<EmailHeader>,
}

/// Legacy Task Scheduler message action.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct ShowMessageAction {
    /// Optional action identifier.
    pub id: Option<String>,
    /// Window title.
    pub title: Option<String>,
    /// Message body.
    pub body: String,
}

/// An action not understood by this crate, retained for lossless round trips.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(deny_unknown_fields)
)]
pub struct UnknownAction {
    /// XML local name.
    pub kind: String,
    /// Original XML element including its start and end tags.
    pub xml: String,
}

/// A Task Scheduler action.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(tag = "type", rename_all = "snake_case")
)]
pub enum Action {
    /// Starts an executable, script, or associated document.
    Exec(ExecAction),
    /// Invokes a registered COM handler.
    ComHandler(ComHandlerAction),
    /// Retained legacy email action, unsupported by modern Windows engines.
    Email(EmailAction),
    /// Retained legacy message action, unsupported by modern Windows engines.
    ShowMessage(ShowMessageAction),
    /// A future or custom XML action retained without interpretation.
    Unknown(UnknownAction),
}

impl Action {
    /// Returns the optional identifier shared by all known action kinds.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Exec(action) => action.id.as_deref(),
            Self::ComHandler(action) => action.id.as_deref(),
            Self::Email(action) => action.id.as_deref(),
            Self::ShowMessage(action) => action.id.as_deref(),
            Self::Unknown(_) => None,
        }
    }
}

impl From<ExecAction> for Action {
    fn from(value: ExecAction) -> Self {
        Self::Exec(value)
    }
}

impl From<ComHandlerAction> for Action {
    fn from(value: ComHandlerAction) -> Self {
        Self::ComHandler(value)
    }
}

impl From<EmailAction> for Action {
    fn from(value: EmailAction) -> Self {
        Self::Email(value)
    }
}

impl From<ShowMessageAction> for Action {
    fn from(value: ShowMessageAction) -> Self {
        Self::ShowMessage(value)
    }
}

impl From<UnknownAction> for Action {
    fn from(value: UnknownAction) -> Self {
        Self::Unknown(value)
    }
}

/// Expands up to 32 Task Scheduler substitutions, starting at `$(Arg0)`.
pub fn expand_action_templates(input: &str, arguments: &[String]) -> String {
    let replacements: BTreeMap<_, _> = arguments
        .iter()
        .take(MAX_RUN_ARGUMENTS)
        .enumerate()
        .map(|(index, value)| (format!("$(Arg{index})"), value))
        .collect();
    replacements
        .into_iter()
        .fold(input.into(), |expanded, (needle, replacement)| {
            expanded.replace(&needle, replacement)
        })
}

/// Quotes one argument according to Windows' conventional argv parser rules.
#[must_use]
pub fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character.is_ascii_whitespace() || character == '"')
    {
        return argument.into();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            quoted.push('"');
        } else {
            quoted.extend(std::iter::repeat_n('\\', backslashes));
            quoted.push(character);
        }
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::{expand_action_templates, quote_windows_argument};

    #[test]
    fn quotes_windows_arguments() {
        assert_eq!(quote_windows_argument("plain"), "plain");
        assert_eq!(quote_windows_argument("two words"), "\"two words\"");
        assert_eq!(quote_windows_argument(""), "\"\"");
        assert_eq!(quote_windows_argument(r#"a\"b"#), "\"a\\\\\\\"b\"");
    }

    #[test]
    fn expands_only_the_native_parameter_limit() {
        let arguments: Vec<_> = (0..33).map(|index| format!("value-{index}")).collect();
        assert_eq!(
            expand_action_templates("$(Arg31) $(Arg32)", &arguments),
            "value-31 $(Arg32)"
        );
    }
}

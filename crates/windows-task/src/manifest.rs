//! Versioned declarative TOML, JSON, and YAML documents.

use std::{collections::BTreeSet, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Error, ErrorKind, FolderPath, Result, TaskPath, ValidationReport,
    model::{SecurityDescriptor, TaskDefinition},
    validation::{Diagnostic, DiagnosticCode, DiagnosticLevel},
};

/// Current declarative format version.
pub const FORMAT_VERSION: u32 = 1;
/// Maximum manifest size accepted by the convenience parser.
pub const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;

/// Supported declarative serialization formats.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentFormat {
    /// TOML.
    Toml,
    /// JSON.
    Json,
    /// YAML via the maintained `serde-saphyr` parser.
    Yaml,
}

impl DocumentFormat {
    /// Detects a format from a conventional file extension.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        match path
            .as_ref()
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("toml") => Ok(Self::Toml),
            Some("json") => Ok(Self::Json),
            Some("yaml" | "yml") => Ok(Self::Yaml),
            _ => Err(Error::new(
                ErrorKind::Serialization,
                "cannot detect manifest format; use .toml, .json, .yaml, or .yml",
            )),
        }
    }
}

/// Non-secret references to Windows Credential Manager entries.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CredentialReferences {
    /// Credential Manager target for remote connection credentials.
    pub connection: Option<String>,
    /// Credential Manager target for a password-backed task registration.
    pub registration: Option<String>,
}

/// Desired folder metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFolder {
    /// Absolute folder path inside the managed namespace.
    pub path: FolderPath,
    /// Optional SDDL to apply.
    pub security_descriptor: Option<SecurityDescriptor>,
}

/// Desired task definition and non-secret credential references.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTask {
    /// Absolute task path inside the managed namespace.
    pub path: TaskPath,
    /// Complete typed task definition.
    pub definition: TaskDefinition,
    /// Credential Manager references, never plaintext credentials.
    #[serde(default)]
    pub credentials: CredentialReferences,
}

/// Complete desired state for one ownership namespace and optional target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskManifest {
    /// Must equal [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Stable owner UUID encoded into managed task registration Source markers.
    pub owner: Uuid,
    /// Human-readable owner or application name.
    pub owner_name: String,
    /// Root below which reconciliation is allowed.
    pub namespace: FolderPath,
    /// Optional remote computer; `None` means local.
    pub target: Option<String>,
    /// Desired folders.
    #[serde(default)]
    pub folders: Vec<ManagedFolder>,
    /// Desired tasks.
    #[serde(default)]
    pub tasks: Vec<ManagedTask>,
}

impl TaskManifest {
    /// Creates an empty version-one manifest.
    #[must_use]
    pub fn new(owner: Uuid, owner_name: impl Into<String>, namespace: FolderPath) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            owner,
            owner_name: owner_name.into(),
            namespace,
            target: None,
            folders: Vec::new(),
            tasks: Vec::new(),
        }
    }

    /// Parses a bounded manifest.
    pub fn from_slice(bytes: &[u8], format: DocumentFormat) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(serialization_error("manifest exceeds the 8 MiB limit"));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|error| serialization_error(format!("manifest is not UTF-8: {error}")))?;
        let manifest = match format {
            DocumentFormat::Toml => toml::from_str(text).map_err(|error: toml::de::Error| {
                serialization_error("invalid TOML manifest syntax or field type").with_context(
                    "byte_offset",
                    error
                        .span()
                        .map_or_else(|| "unknown".into(), |span| span.start.to_string()),
                )
            })?,
            DocumentFormat::Json => {
                serde_json::from_str(text).map_err(|error: serde_json::Error| {
                    serialization_error("invalid JSON manifest syntax or field type")
                        .with_context("line", error.line().to_string())
                        .with_context("column", error.column().to_string())
                })?
            }
            DocumentFormat::Yaml => {
                serde_saphyr::from_str(text).map_err(|error: serde_saphyr::Error| {
                    let mut result =
                        serialization_error("invalid YAML manifest syntax or field type");
                    if let Some(location) = error.location() {
                        result = result
                            .with_context("line", location.line().to_string())
                            .with_context("column", location.column().to_string());
                    }
                    result
                })?
            }
        };
        Ok(manifest)
    }

    /// Serializes a manifest deterministically enough for source control.
    pub fn to_string(&self, format: DocumentFormat) -> Result<String> {
        match format {
            DocumentFormat::Toml => toml::to_string_pretty(self)
                .map_err(|error| serialization_error(format!("TOML: {error}"))),
            DocumentFormat::Json => serde_json::to_string_pretty(self)
                .map_err(|error| serialization_error(format!("JSON: {error}"))),
            DocumentFormat::Yaml => {
                // The MSRV-compatible saphyr serializer emits ambiguous plain
                // scalars (for example `owner_name: value:`). Use the same data
                // model as JSON, with block containers and JSON-quoted scalars,
                // a YAML 1.2 subset that never guesses whether text is a token.
                let value = serde_json::to_value(self)
                    .map_err(|_| serialization_error("cannot encode YAML manifest values"))?;
                let mut output = String::new();
                write_yaml_value(&value, 0, &mut output);
                Ok(output)
            }
        }
    }

    /// Validates format, namespace, ownership, uniqueness, and every task.
    #[must_use]
    pub fn validate(&self) -> ValidationReport {
        let mut report = ValidationReport::default();
        if self.format_version != FORMAT_VERSION {
            report.diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: DiagnosticCode::UnsupportedCapability,
                path: "format_version".into(),
                message: format!(
                    "manifest version {} is unsupported; expected {FORMAT_VERSION}",
                    self.format_version
                ),
                remediation: Some(format!(
                    "Use format_version = {FORMAT_VERSION} and migrate unsupported fields."
                )),
            });
        }
        if self.namespace.is_root() {
            report.diagnostics.push(Diagnostic {
                level: DiagnosticLevel::Error,
                code: DiagnosticCode::OwnershipConflict,
                path: "namespace".into(),
                message: "the scheduler root cannot be a managed namespace".into(),
                remediation: Some("choose a dedicated child folder".into()),
            });
        }
        let mut folder_paths = BTreeSet::new();
        for (index, folder) in self.folders.iter().enumerate() {
            if !within_namespace(folder.path.as_str(), self.namespace.as_str()) {
                report
                    .diagnostics
                    .push(outside_namespace(format!("folders[{index}].path")));
            }
            if !folder_paths.insert(folder.path.clone()) {
                report
                    .diagnostics
                    .push(duplicate_path(format!("folders[{index}].path")));
            }
        }
        let mut task_paths = BTreeSet::new();
        for (index, task) in self.tasks.iter().enumerate() {
            if !within_namespace(task.path.as_str(), self.namespace.as_str()) {
                report
                    .diagnostics
                    .push(outside_namespace(format!("tasks[{index}].path")));
            }
            if !task_paths.insert(task.path.clone()) {
                report
                    .diagnostics
                    .push(duplicate_path(format!("tasks[{index}].path")));
            }
            report
                .diagnostics
                .extend(task.definition.validate().diagnostics.into_iter().map(
                    |mut diagnostic| {
                        diagnostic.path = format!("tasks[{index}].definition.{}", diagnostic.path);
                        diagnostic
                    },
                ));
        }
        report
    }

    /// Returns the ownership URI expected for one managed task.
    #[must_use]
    pub fn ownership_uri(&self, path: &TaskPath) -> String {
        format!(
            "urn:windows-task:owner:{}:task:{}",
            self.owner,
            percent_encode(path.as_str())
        )
    }
}

fn within_namespace(path: &str, namespace: &str) -> bool {
    path == namespace
        || path
            .strip_prefix(namespace)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn outside_namespace(path: String) -> Diagnostic {
    Diagnostic {
        level: DiagnosticLevel::Error,
        code: DiagnosticCode::OwnershipConflict,
        path,
        message: "path is outside the managed namespace".into(),
        remediation: Some("Move the path under this manifest's namespace.".into()),
    }
}

fn duplicate_path(path: String) -> Diagnostic {
    Diagnostic {
        level: DiagnosticLevel::Error,
        code: DiagnosticCode::DuplicateId,
        path,
        message: "path appears more than once".into(),
        remediation: Some("Remove the duplicate entry or use a distinct path.".into()),
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    encoded
}

fn write_yaml_value(value: &serde_json::Value, indent: usize, output: &mut String) {
    match value {
        serde_json::Value::Object(fields) if !fields.is_empty() => {
            for (key, value) in fields {
                let key = serde_json::Value::String(key.clone()).to_string();
                write_yaml_entry(&format!("{key}:"), value, indent, output);
            }
        }
        serde_json::Value::Array(items) if !items.is_empty() => {
            for value in items {
                write_yaml_entry("-", value, indent, output);
            }
        }
        _ => {
            output.push_str(&value.to_string());
            output.push('\n');
        }
    }
}

fn write_yaml_entry(prefix: &str, value: &serde_json::Value, indent: usize, output: &mut String) {
    output.push_str(&" ".repeat(indent));
    output.push_str(prefix);
    let container = match value {
        serde_json::Value::Object(fields) => !fields.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        _ => false,
    };
    output.push(if container { '\n' } else { ' ' });
    write_yaml_value(value, indent + 2, output);
}

fn serialization_error(message: impl Into<String>) -> Error {
    let message = message.into();
    Error::new(ErrorKind::Serialization, message.clone()).with_validation(ValidationReport {
        diagnostics: vec![Diagnostic {
            level: DiagnosticLevel::Error,
            code: DiagnosticCode::Other("manifest_syntax".into()),
            path: "$".into(),
            message,
            remediation: Some(
                "Check the format, reported location, field types and the 8 MiB input limit."
                    .into(),
            ),
        }],
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uuid::Uuid;

    use super::{DocumentFormat, TaskManifest};
    use crate::FolderPath;

    #[test]
    fn yaml_preserves_every_control_scalar_and_unicode_line_separator() {
        for code in (0..=0x9f).chain([0x2028, 0x2029, 0xfeff]) {
            let character = char::from_u32(code).expect("valid scalar fixture");
            let value = format!("before{character}after");
            let manifest =
                TaskManifest::new(Uuid::nil(), value, "\\Tests".parse().expect("namespace"));
            let output = manifest
                .to_string(DocumentFormat::Yaml)
                .expect("YAML output");
            let actual = TaskManifest::from_slice(output.as_bytes(), DocumentFormat::Yaml)
                .expect("YAML input");
            assert_eq!(actual, manifest, "Unicode scalar U+{code:04X}");
        }
    }

    #[test]
    fn yaml_quotes_ambiguous_scalars_and_preserves_nested_manifest_values() {
        use crate::{
            manifest::ManagedTask,
            model::{Action, ExecAction, TaskDefinition},
        };
        for value in [
            "fuz:",
            "true",
            "null",
            "007",
            "trailing ",
            " leading",
            "line\nnext\r\n",
            "\u{1b}\0\t",
            "日本語:#[]{}",
            "a\u{feff}b",
        ] {
            let mut manifest =
                TaskManifest::new(Uuid::nil(), value, "\\Tests".parse().expect("namespace"));
            let mut definition =
                TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe").args([value])));
            definition.registration.description = Some(value.into());
            manifest.tasks.push(ManagedTask {
                path: manifest.namespace.task("Task").expect("path"),
                definition,
                credentials: Default::default(),
            });
            let output = manifest
                .to_string(DocumentFormat::Yaml)
                .expect("serialize YAML");
            assert_eq!(
                TaskManifest::from_slice(output.as_bytes(), DocumentFormat::Yaml)
                    .expect("read emitted YAML"),
                manifest
            );
        }
    }

    #[test]
    fn malformed_documents_never_echo_input_in_serialized_errors() {
        for (format, input) in [
            (DocumentFormat::Toml, "owner = SENTINEL_SECRET"),
            (DocumentFormat::Json, "\"SENTINEL_SECRET\""),
            (DocumentFormat::Yaml, "owner: [SENTINEL_SECRET"),
        ] {
            let error =
                TaskManifest::from_slice(input.as_bytes(), format).expect_err("invalid manifest");
            let serialized = serde_json::to_string(&error).expect("structured error");
            assert!(!serialized.contains("SENTINEL"));
            assert!(serialized.contains("manifest_syntax"));
            assert!(serialized.contains("byte_offset") || serialized.contains("line"));
        }
    }

    #[test]
    fn empty_manifest_round_trips_all_formats() {
        let manifest = TaskManifest::new(
            Uuid::nil(),
            "tests",
            FolderPath::from_str("\\windows-task-tests").expect("namespace"),
        );
        for format in [
            DocumentFormat::Toml,
            DocumentFormat::Json,
            DocumentFormat::Yaml,
        ] {
            let text = manifest.to_string(format).expect("serialize");
            let decoded = TaskManifest::from_slice(text.as_bytes(), format).expect("deserialize");
            assert_eq!(decoded, manifest);
        }
    }
}

#[cfg(test)]
mod example_documents {
    use super::{DocumentFormat, TaskManifest};
    use crate::model::{Action, Principal, TaskSettings};

    const FULL: &str = include_str!("../examples/desired-state.toml");
    const MINIMAL: &str = include_str!("../examples/desired-state-minimal.toml");

    fn parse(text: &str) -> TaskManifest {
        TaskManifest::from_slice(text.as_bytes(), DocumentFormat::Toml)
            .expect("a checked-in example must parse")
    }

    // The fully explicit document must stay byte-identical to what the library
    // writes, so a changed default cannot leave a stale value in the document
    // readers copy from.
    #[test]
    fn the_full_example_is_the_canonical_output_of_the_current_model() {
        let manifest = parse(FULL);
        assert_eq!(
            manifest
                .to_string(DocumentFormat::Toml)
                .expect("canonical TOML"),
            FULL,
            "regenerate with: cargo run --example manifest --features serde"
        );
        let task = manifest.tasks.first().expect("one managed task");
        assert_eq!(task.definition.settings, TaskSettings::default());
        assert_eq!(task.definition.principal, Principal::default());
        assert!(manifest.validate().is_valid());
    }

    // Every field the minimal document omits must resolve to the value the
    // explicit document spells out. This is the contract that lets a reader
    // delete a line without changing what the task does.
    #[test]
    fn the_minimal_example_resolves_to_the_documented_defaults() {
        let minimal = parse(MINIMAL);
        let full = parse(FULL);
        let minimal_task = minimal.tasks.first().expect("one managed task");
        let full_task = full.tasks.first().expect("one managed task");

        assert_eq!(
            minimal_task.definition.settings,
            full_task.definition.settings
        );
        assert_eq!(
            minimal_task.definition.principal,
            full_task.definition.principal
        );
        assert_eq!(
            minimal_task.definition.triggers,
            full_task.definition.triggers
        );
        assert_eq!(minimal_task.credentials, full_task.credentials);
        let Some(Action::Exec(action)) = minimal_task.definition.actions.as_slice().first() else {
            panic!("the minimal example declares one exec action");
        };
        assert!(!action.hide_window);
        assert!(minimal.validate().is_valid());
    }
}

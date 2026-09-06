use std::fmt;

/// Broad, stable classification for errors returned by this crate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The operation is unavailable on the current platform.
    UnsupportedPlatform,
    /// A task or folder path was invalid.
    InvalidPath,
    /// A task definition did not satisfy Task Scheduler constraints.
    InvalidDefinition,
    /// Task XML could not be read or written.
    Xml,
    /// A declarative document could not be read or written.
    Serialization,
    /// The Task Scheduler service or its RPC endpoint is unavailable.
    SchedulerUnavailable,
    /// The caller does not have the required access.
    AccessDenied,
    /// Authentication or credentials failed.
    Authentication,
    /// The requested object was not found.
    NotFound,
    /// The object already exists.
    AlreadyExists,
    /// The target cannot represent the requested capability.
    Capability,
    /// A COM operation failed.
    Com,
    /// A Win32 operation failed.
    Win32,
    /// Work was cancelled before it started.
    Cancelled,
    /// The requested wait elapsed.
    Timeout,
    /// Task Scheduler event history is unavailable.
    HistoryUnavailable,
    /// An event bookmark is no longer valid; records may have been lost.
    HistoryGap,
    /// A bounded query cannot establish a complete result.
    QueryLimit,
    /// Current state conflicts with the requested ownership or operation.
    Conflict,
    /// The operation cannot be safely rolled back with the supplied inputs.
    Irreversible,
    /// A background worker stopped unexpectedly.
    WorkerStopped,
    /// An error not covered by a more specific stable kind.
    Other,
}

/// Error with stable classification and native diagnostic context.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Error {
    kind: ErrorKind,
    message: String,
    operation: Option<String>,
    target: Option<String>,
    native_code: Option<i32>,
    #[cfg_attr(feature = "serde", serde(default))]
    diagnostics: Vec<crate::Diagnostic>,
    #[cfg_attr(feature = "serde", serde(default))]
    context: std::collections::BTreeMap<String, String>,
}

impl Error {
    /// Creates an error with a stable kind and human-readable message.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            operation: None,
            target: None,
            native_code: None,
            diagnostics: Vec::new(),
            context: std::collections::BTreeMap::new(),
        }
    }

    /// Adds the operation that failed.
    #[must_use]
    pub fn with_operation(mut self, operation: impl Into<String>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Adds the task, folder, computer, or channel that failed.
    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Retains actionable validation findings instead of flattening them.
    #[must_use]
    pub fn with_validation(mut self, report: crate::ValidationReport) -> Self {
        self.diagnostics = report.diagnostics;
        self
    }

    /// Adds structured observation context. This is not automatically traced.
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Validation findings associated with this error.
    pub fn diagnostics(&self) -> &[crate::Diagnostic] {
        &self.diagnostics
    }

    /// Last observed state or other structured error context.
    pub fn context(&self) -> &std::collections::BTreeMap<String, String> {
        &self.context
    }

    /// Adds the original HRESULT or Win32 error value.
    #[must_use]
    pub const fn with_native_code(mut self, native_code: i32) -> Self {
        self.native_code = Some(native_code);
        self
    }

    /// Returns the stable classification.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the human-readable message without contextual prefixes.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the native HRESULT or Win32 value, when present.
    #[must_use]
    pub const fn native_code(&self) -> Option<i32> {
        self.native_code
    }

    /// Returns the operation context, when present.
    #[must_use]
    pub fn operation(&self) -> Option<&str> {
        self.operation.as_deref()
    }

    /// Returns the target context, when present.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(operation) = &self.operation {
            write!(formatter, "{operation}: ")?;
        }
        write!(formatter, "{}", self.message)?;
        if let Some(target) = &self.target {
            write!(formatter, " (target: {target})")?;
        }
        if let Some(code) = self.native_code {
            write!(
                formatter,
                " (native code: 0x{:08X})",
                u32::from_ne_bytes(code.to_ne_bytes())
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// Result alias used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Converts a portable model or path failure into the crate's error type so a
/// caller returning [`Result`] can use `?` on every construction step.
macro_rules! from_model_error {
    ($source:ty, $kind:expr) => {
        impl From<$source> for Error {
            fn from(error: $source) -> Self {
                Self::new($kind, error.to_string())
            }
        }
    };
}

from_model_error!(crate::ParsePathError, ErrorKind::InvalidPath);
from_model_error!(
    crate::model::ParseTaskDateTimeError,
    ErrorKind::InvalidDefinition
);
from_model_error!(
    crate::model::ParseTaskDurationError,
    ErrorKind::InvalidDefinition
);
from_model_error!(
    crate::model::CollectionLimitError,
    ErrorKind::InvalidDefinition
);
from_model_error!(
    crate::model::InvalidSecurityDescriptor,
    ErrorKind::InvalidDefinition
);
from_model_error!(crate::model::InvalidPrivilege, ErrorKind::InvalidDefinition);

#[cfg(feature = "recipes")]
from_model_error!(crate::schedule::ScheduleError, ErrorKind::InvalidDefinition);

#[cfg(test)]
mod conversion_tests {
    use super::{Error, ErrorKind};

    #[test]
    fn model_failures_convert_without_losing_their_classification() {
        use crate::model::{
            Action, Actions, InvalidSecurityDescriptor, RequiredPrivilege, SecurityDescriptor,
            TaskDateTime, TaskDuration,
        };

        let path: Error = "relative"
            .parse::<crate::TaskPath>()
            .expect_err("relative path")
            .into();
        assert_eq!(path.kind(), ErrorKind::InvalidPath);
        assert!(!path.message().is_empty());

        let boundary: Error = TaskDateTime::parse("not-a-timestamp")
            .expect_err("invalid boundary")
            .into();
        assert_eq!(boundary.kind(), ErrorKind::InvalidDefinition);

        let duration: Error = TaskDuration::parse("P1Y")
            .expect_err("calendar unit")
            .into();
        assert_eq!(duration.kind(), ErrorKind::InvalidDefinition);

        let overflow: Error = Actions::new(vec![
            Action::Exec(crate::model::ExecAction::new(
                "fixture.exe"
            ));
            crate::MAX_ACTIONS + 1
        ])
        .expect_err("action limit")
        .into();
        assert_eq!(overflow.kind(), ErrorKind::InvalidDefinition);

        let sddl: Error = SecurityDescriptor::from_sddl("")
            .expect_err("empty SDDL")
            .into();
        assert_eq!(sddl.kind(), ErrorKind::InvalidDefinition);
        assert_eq!(
            Error::from(InvalidSecurityDescriptor).kind(),
            ErrorKind::InvalidDefinition
        );

        let privilege: Error = RequiredPrivilege::new("NotAPrivilege")
            .expect_err("invalid privilege name")
            .into();
        assert_eq!(privilege.kind(), ErrorKind::InvalidDefinition);
    }

    #[cfg(feature = "recipes")]
    #[test]
    fn schedule_failures_convert_to_invalid_definition() {
        let error: Error = crate::schedule::every(
            crate::model::TaskDateTime::parse("2026-09-05T06:00:00").expect("anchor"),
            crate::model::TaskDuration::from_secs(0),
        )
        .expect_err("zero repetition")
        .into();
        assert_eq!(error.kind(), ErrorKind::InvalidDefinition);
    }
}

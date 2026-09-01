use std::{fmt, result::Result, str::FromStr};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Error returned while parsing a scheduler path.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ParsePathError {
    message: String,
}

impl ParsePathError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Canonical absolute Task Scheduler folder path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct FolderPath(String);

impl FolderPath {
    /// Returns the scheduler root folder (`\`).
    #[must_use]
    pub fn root() -> Self {
        Self("\\".into())
    }

    /// Joins one validated child folder.
    pub fn join(&self, child: &str) -> Result<Self, ParsePathError> {
        validate_component(child)?;
        if self.is_root() {
            Self::from_str(&format!("\\{child}"))
        } else {
            Self::from_str(&format!("{}\\{child}", self.0))
        }
    }

    /// Joins one validated task name.
    pub fn task(&self, name: &str) -> Result<TaskPath, ParsePathError> {
        validate_component(name)?;
        if self.is_root() {
            TaskPath::from_str(&format!("\\{name}"))
        } else {
            TaskPath::from_str(&format!("{}\\{name}", self.0))
        }
    }

    /// Whether this path is the scheduler root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == "\\"
    }

    /// Returns the canonical path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the parent folder, or `None` for root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let index = self.0.rfind('\\').unwrap_or(0);
        Some(if index == 0 {
            Self::root()
        } else {
            Self(self.0[..index].into())
        })
    }

    /// Returns the final folder name, or `None` for root.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        (!self.is_root()).then(|| self.0.rsplit('\\').next().unwrap_or_default())
    }
}

impl FromStr for FolderPath {
    type Err = ParsePathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        canonicalize(value, true).map(Self)
    }
}

impl TryFrom<String> for FolderPath {
    type Error = ParsePathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

impl From<FolderPath> for String {
    fn from(value: FolderPath) -> Self {
        value.0
    }
}

impl fmt::Display for FolderPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Canonical absolute Task Scheduler task path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(try_from = "String", into = "String")
)]
pub struct TaskPath(String);

impl TaskPath {
    /// Returns the canonical task path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the containing folder.
    #[must_use]
    pub fn folder(&self) -> FolderPath {
        let index = self.0.rfind('\\').unwrap_or(0);
        if index == 0 {
            FolderPath::root()
        } else {
            FolderPath(self.0[..index].into())
        }
    }

    /// Returns the final task name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.0.rsplit('\\').next().unwrap_or_default()
    }
}

impl FromStr for TaskPath {
    type Err = ParsePathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        canonicalize(value, false).map(Self)
    }
}

impl TryFrom<String> for TaskPath {
    type Error = ParsePathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_str(&value)
    }
}

impl From<TaskPath> for String {
    fn from(value: TaskPath) -> Self {
        value.0
    }
}

impl fmt::Display for TaskPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn canonicalize(value: &str, folder: bool) -> Result<String, ParsePathError> {
    if value.contains('\0') {
        return Err(ParsePathError::new("scheduler paths cannot contain NUL"));
    }
    let normalized = value.replace('/', "\\");
    if !normalized.starts_with('\\') {
        return Err(ParsePathError::new(
            "scheduler paths must be absolute and start with a backslash",
        ));
    }
    if normalized == "\\" {
        return folder
            .then_some(normalized)
            .ok_or_else(|| ParsePathError::new("a task path must include a task name"));
    }
    if normalized.ends_with('\\') {
        return Err(ParsePathError::new(
            "scheduler paths cannot end with a backslash",
        ));
    }
    for component in normalized[1..].split('\\') {
        validate_component(component)?;
    }
    Ok(normalized)
}

fn validate_component(component: &str) -> Result<(), ParsePathError> {
    if component.is_empty() {
        return Err(ParsePathError::new(
            "scheduler paths cannot contain empty components",
        ));
    }
    if matches!(component, "." | "..") {
        return Err(ParsePathError::new(
            "scheduler paths cannot contain . or ..",
        ));
    }
    if component.trim() != component {
        return Err(ParsePathError::new(
            "scheduler path components cannot begin or end with whitespace",
        ));
    }
    if component.contains(['\\', '/', '\0']) {
        return Err(ParsePathError::new(
            "a scheduler path component contains a separator or NUL",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FolderPath, TaskPath};
    use std::str::FromStr;

    #[test]
    fn canonicalizes_forward_slashes() {
        let path = TaskPath::from_str("/Acme/Backup").expect("valid path");
        assert_eq!(path.as_str(), "\\Acme\\Backup");
        assert_eq!(path.folder().as_str(), "\\Acme");
        assert_eq!(path.name(), "Backup");
    }

    #[test]
    fn root_is_only_a_folder() {
        FolderPath::from_str("\\").expect("root folder is valid");
        TaskPath::from_str("\\").expect_err("root is not a task");
    }

    #[test]
    fn rejects_native_leading_or_trailing_whitespace() {
        TaskPath::from_str("\\Acme\\ Task").expect_err("leading whitespace must be rejected");
        TaskPath::from_str("\\Acme\\Task ").expect_err("trailing whitespace must be rejected");
    }
}

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Built-in service identities understood by Task Scheduler.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ServiceAccount {
    /// Local System (`SYSTEM`).
    LocalSystem,
    /// Local Service.
    LocalService,
    /// Network Service.
    NetworkService,
}

/// Identity associated with a task principal.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(tag = "type", content = "value", rename_all = "snake_case")
)]
pub enum PrincipalIdentity {
    /// No identity is embedded in the definition.
    None,
    /// A user account by SAM name, UPN, SID, or well-known alias.
    User(String),
    /// A group by name or SID.
    Group(String),
    /// A built-in service account.
    ServiceAccount(ServiceAccount),
}

/// Task Scheduler logon strategy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum LogonType {
    /// The logon method is not specified.
    None,
    /// Stores a password with Task Scheduler.
    Password,
    /// Uses an existing interactive token.
    InteractiveToken,
    /// Uses a Service-for-User token without network or encrypted-file access.
    S4u,
    /// Activates for any member of the configured group.
    Group,
    /// Runs as Local System, Local Service, or Network Service.
    ServiceAccount,
    /// Uses an interactive token when available, otherwise the supplied password.
    InteractiveTokenOrPassword,
}

/// Requested elevation level.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum RunLevel {
    /// Runs with the least-privileged token available.
    #[default]
    LeastPrivilege,
    /// Runs with the highest available token.
    HighestAvailable,
}

/// Process token SID behavior.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum ProcessTokenSidType {
    /// Uses Task Scheduler's default behavior.
    #[default]
    Default,
    /// Does not add a task SID to the token.
    None,
    /// Adds an unrestricted task SID.
    Unrestricted,
}

/// A Windows privilege requested for the task token.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
pub struct RequiredPrivilege(String);

impl RequiredPrivilege {
    /// Creates a privilege name such as `SeBackupPrivilege`.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidPrivilege> {
        let value = value.into();
        if value.starts_with("Se") && value.ends_with("Privilege") && value.len() > 11 {
            Ok(Self(value))
        } else {
            Err(InvalidPrivilege(value))
        }
    }

    /// Returns the native privilege name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An invalid Windows privilege name.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid Windows privilege name {0:?}")]
pub struct InvalidPrivilege(String);

/// Security principal under which a task runs.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct Principal {
    /// XML identifier referenced by the task definition.
    pub id: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// User, group, or service identity.
    pub identity: PrincipalIdentity,
    /// Authentication/token strategy.
    pub logon_type: LogonType,
    /// Requested UAC run level.
    pub run_level: RunLevel,
    /// Task SID behavior.
    pub process_token_sid_type: ProcessTokenSidType,
    /// Extra privileges requested in the token.
    pub required_privileges: Vec<RequiredPrivilege>,
}

impl Default for Principal {
    fn default() -> Self {
        Self {
            id: "Author".into(),
            display_name: None,
            identity: PrincipalIdentity::None,
            logon_type: LogonType::None,
            run_level: RunLevel::LeastPrivilege,
            process_token_sid_type: ProcessTokenSidType::Default,
            required_privileges: Vec::new(),
        }
    }
}

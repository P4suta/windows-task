#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]

//! `windows-task` keeps Task Scheduler's COM details behind task-oriented,
//! ownership-safe Rust APIs. The domain model, XML, manifests, and validation
//! are portable; connecting to a scheduler requires Windows.

mod credentials;
mod error;
#[cfg(any(feature = "client", feature = "handler"))]
mod observe;
mod path;
mod validation;

pub mod model;
pub mod xml;

#[cfg(feature = "recipes")]
#[cfg_attr(docsrs, doc(cfg(feature = "recipes")))]
pub mod schedule;

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod manifest;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

#[cfg(feature = "history")]
#[cfg_attr(docsrs, doc(cfg(feature = "history")))]
pub mod history;

#[cfg(feature = "reconcile")]
#[cfg_attr(docsrs, doc(cfg(feature = "reconcile")))]
pub mod reconcile;

#[cfg(feature = "handler")]
#[cfg_attr(docsrs, doc(cfg(feature = "handler")))]
pub mod handler;

pub use credentials::{Credential, Credentials, Password};
pub use error::{Error, ErrorKind, Result};
pub use model::SecurityDescriptor;
pub use path::{FolderPath, ParsePathError, TaskPath};
pub use validation::{
    Diagnostic, DiagnosticCode, DiagnosticLevel, DiagnosticReport, ValidationReport,
};

#[cfg(feature = "handler")]
#[cfg_attr(docsrs, doc(cfg(feature = "handler")))]
pub use windows_task_macros::handler;

/// The maximum number of actions accepted by Task Scheduler 2.0.
pub const MAX_ACTIONS: usize = 32;

/// The maximum number of triggers accepted by Task Scheduler 2.0.
pub const MAX_TRIGGERS: usize = 48;

/// The maximum number of values accepted by `IRegisteredTask::Run`/`RunEx`.
pub const MAX_RUN_ARGUMENTS: usize = 32;

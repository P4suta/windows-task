//! Official `windows-task` command-line interface.

use std::{
    collections::BTreeSet,
    error::Error as StdError,
    io::Write as _,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, SystemTime},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use uuid::Uuid;
use windows_task::{
    Credential, Error, ErrorKind, FolderPath, Password, Result, SecurityDescriptor, TaskPath,
    client::{
        ListOptions, RegistrationMode, RegistrationOptions, RunOptions, Scheduler,
        SecurityInformation, WaitOptions,
    },
    history::{HistoryQuery, RunOutcome},
    manifest::{DocumentFormat, TaskManifest},
    model::LogonType,
    reconcile::{
        ApplyOptions, CredentialPurpose, CredentialResolver, PlanOptions, apply_with_credentials,
        inspect, plan_state,
    },
    xml::RawTaskXml,
};

type CliResult<T> = std::result::Result<T, Box<dyn StdError>>;

#[derive(Debug, Parser)]
#[command(name = "windows-task", version, about)]
struct Cli {
    #[command(flatten)]
    connection: ConnectionArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Default, Args)]
struct ConnectionArgs {
    /// Remote computer name. Overrides a manifest target.
    #[arg(long, global = true)]
    computer: Option<String>,
    /// Generic Windows Credential Manager target for the remote connection.
    #[arg(long, global = true, value_name = "CREDENTIAL_TARGET")]
    connection_credential: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a manifest offline and optionally against the target service.
    Validate {
        /// Manifest path.
        manifest: PathBuf,
        /// Overrides extension-based format detection.
        #[arg(long)]
        format: Option<Format>,
        /// Also ask the connected Task Scheduler service to validate each task.
        #[arg(long)]
        native: bool,
    },
    /// Diagnose local or remote Task Scheduler connectivity and rights.
    Doctor,
    /// Print capabilities reported and probed from the target.
    Capabilities,
    /// Inspect or mutate registered tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Inspect or mutate scheduler folders.
    Folder {
        #[command(subcommand)]
        command: FolderCommand,
    },
    /// Query, watch, or explicitly configure Operational history.
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
    /// Produce a live ownership-safe desired-state plan without mutation.
    Plan {
        /// Manifest path.
        manifest: PathBuf,
        /// Overrides extension-based format detection.
        #[arg(long)]
        format: Option<Format>,
        /// Plan removal of owned tasks absent from desired state.
        #[arg(long)]
        prune: bool,
        /// Plan explicit adoption of colliding unowned tasks.
        #[arg(long)]
        adopt: bool,
    },
    /// Apply a manifest with rollback snapshots and reverse compensation.
    Apply {
        /// Manifest path.
        manifest: PathBuf,
        /// Overrides extension-based format detection.
        #[arg(long)]
        format: Option<Format>,
        /// Remove owned tasks absent from desired state.
        #[arg(long)]
        prune: bool,
        /// Explicitly adopt colliding unowned tasks.
        #[arg(long)]
        adopt: bool,
        /// Continue if exact rollback credentials or ACLs cannot be captured.
        #[arg(long)]
        allow_irreversible: bool,
        /// Stop running instances before update/delete.
        #[arg(long)]
        stop_running: bool,
        /// Credential Manager target overriding desired registration references.
        #[arg(long, value_name = "CREDENTIAL_TARGET")]
        registration_credential: Option<String>,
        /// Credential Manager target used specifically to restore old tasks.
        #[arg(long, value_name = "CREDENTIAL_TARGET")]
        rollback_credential: Option<String>,
        /// Required acknowledgement for mutation.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Read one task, including its typed definition.
    Get {
        path: TaskPath,
        /// Emit exact registered XML instead of JSON.
        #[arg(long)]
        raw: bool,
    },
    /// List tasks.
    List {
        #[arg(default_value = "\\")]
        folder: FolderPath,
        #[arg(long)]
        recursive: bool,
        #[arg(long)]
        hidden: bool,
    },
    /// Register a bounded raw Task XML document.
    Register {
        path: TaskPath,
        xml: PathBuf,
        #[arg(long, value_enum, default_value_t = RegisterMode::CreateOrUpdate)]
        mode: RegisterMode,
        /// Credential Manager target for a password-backed principal.
        #[arg(long, value_name = "CREDENTIAL_TARGET")]
        registration_credential: Option<String>,
        /// Permit registration triggers to fire.
        #[arg(long)]
        allow_registration_triggers: bool,
        /// Required acknowledgement for scheduler mutation.
        #[arg(long)]
        yes: bool,
    },
    /// Delete one task without implicitly stopping its running instances.
    Delete {
        path: TaskPath,
        #[arg(long)]
        yes: bool,
    },
    /// Enable one task.
    Enable { path: TaskPath },
    /// Disable one task.
    Disable { path: TaskPath },
    /// Start a task and optionally wait for its correlated result.
    Run {
        path: TaskPath,
        #[arg(long = "parameter", value_name = "VALUE")]
        parameters: Vec<String>,
        #[arg(long)]
        ignore_constraints: bool,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 300)]
        timeout_seconds: u64,
    },
    /// Stop all instances of a task or one instance GUID.
    Stop {
        #[arg(
            long,
            conflicts_with = "instance",
            required_unless_present = "instance"
        )]
        path: Option<TaskPath>,
        #[arg(long, conflicts_with = "path", required_unless_present = "path")]
        instance: Option<Uuid>,
    },
    /// List running instances.
    Running {
        #[arg(long)]
        hidden: bool,
    },
}

#[derive(Debug, Subcommand)]
enum FolderCommand {
    /// List child folders.
    List {
        #[arg(default_value = "\\")]
        path: FolderPath,
        #[arg(long)]
        recursive: bool,
    },
    /// Create one folder; parents must already exist.
    Create {
        path: FolderPath,
        #[arg(long)]
        sddl: Option<String>,
    },
    /// Delete one empty folder.
    Delete {
        path: FolderPath,
        #[arg(long)]
        yes: bool,
    },
    /// Read owner, group, and DACL SDDL.
    Security { path: FolderPath },
    /// Set a folder DACL.
    SetSecurity {
        path: FolderPath,
        sddl: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum HistoryCommand {
    /// Query bounded Operational history (newest first by default).
    Query {
        #[arg(long)]
        task: Option<TaskPath>,
        #[arg(long)]
        instance: Option<Uuid>,
        #[arg(long)]
        since_seconds: Option<u64>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        forward: bool,
    },
    /// Print newly observed events as newline-delimited JSON.
    Watch {
        #[arg(long)]
        task: Option<TaskPath>,
        #[arg(long)]
        instance: Option<Uuid>,
        #[arg(long, default_value_t = 500)]
        poll_milliseconds: u64,
    },
    /// Explicitly enable the Operational channel (administrator rights likely).
    Enable {
        #[arg(long)]
        yes: bool,
    },
    /// Explicitly disable the Operational channel (administrator rights likely).
    Disable {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Toml,
    Json,
    Yaml,
}

impl From<Format> for DocumentFormat {
    fn from(value: Format) -> Self {
        match value {
            Format::Toml => Self::Toml,
            Format::Json => Self::Json,
            Format::Yaml => Self::Yaml,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RegisterMode {
    Create,
    Update,
    CreateOrUpdate,
}

impl From<RegisterMode> for RegistrationMode {
    fn from(value: RegisterMode) -> Self {
        match value {
            RegisterMode::Create => Self::Create,
            RegisterMode::Update => Self::Update,
            RegisterMode::CreateOrUpdate => Self::CreateOrUpdate,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("windows-task: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> CliResult<ExitCode> {
    let Cli {
        connection,
        command,
    } = cli;
    match command {
        Command::Validate {
            manifest,
            format,
            native,
        } => validate_command(&connection, &manifest, format, native),
        Command::Doctor => {
            let scheduler = connect(&connection, None, None)?;
            let report = scheduler.blocking().doctor()?;
            print_json(&report)?;
            Ok(if report.is_healthy() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Command::Capabilities => {
            let scheduler = connect(&connection, None, None)?;
            print_json(&scheduler.blocking().capabilities()?)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Task { command } => task_command(&connection, command),
        Command::Folder { command } => folder_command(&connection, command),
        Command::History { command } => history_command(&connection, command),
        Command::Plan {
            manifest,
            format,
            prune,
            adopt,
        } => {
            let manifest = load_manifest(&manifest, format)?;
            let credential = manifest_connection_reference(&manifest)?;
            let scheduler = connect(
                &connection,
                manifest.target.as_deref(),
                credential.as_deref(),
            )?;
            let state = inspect(&scheduler.blocking(), &manifest)?;
            let plan = plan_state(&manifest, &state, PlanOptions { prune, adopt })?;
            print_json(&plan)?;
            Ok(if plan.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Command::Apply {
            manifest,
            format,
            prune,
            adopt,
            allow_irreversible,
            stop_running,
            registration_credential,
            rollback_credential,
            yes,
        } => {
            require_yes(yes, "apply mutates Task Scheduler")?;
            let manifest = load_manifest(&manifest, format)?;
            let connection_reference = manifest_connection_reference(&manifest)?;
            let scheduler = connect(
                &connection,
                manifest.target.as_deref(),
                connection_reference.as_deref(),
            )?;
            let mut resolver = CredentialManagerResolver {
                desired_override: registration_credential.as_deref(),
                rollback_override: rollback_credential.as_deref(),
            };
            let report = apply_with_credentials(
                &scheduler.blocking(),
                &manifest,
                ApplyOptions {
                    prune,
                    adopt,
                    allow_irreversible,
                    stop_running,
                    ..ApplyOptions::default()
                },
                &mut resolver,
            )?;
            print_json(&report)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn validate_command(
    connection: &ConnectionArgs,
    path: &Path,
    format: Option<Format>,
    native: bool,
) -> CliResult<ExitCode> {
    let manifest = load_manifest(path, format)?;
    let report = manifest.validate();
    if !native || !report.is_valid() {
        print_json(&report)?;
        return Ok(if report.is_valid() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        });
    }
    let credential = manifest_connection_reference(&manifest)?;
    let scheduler = connect(
        connection,
        manifest.target.as_deref(),
        credential.as_deref(),
    )?;
    let blocking = scheduler.blocking();
    let mut reports = Vec::with_capacity(manifest.tasks.len());
    let mut valid = true;
    for task in manifest.tasks {
        let report = blocking.validate(task.definition)?;
        valid &= report.is_valid();
        reports.push(json!({ "path": task.path, "report": report }));
    }
    print_json(&json!({ "portable": report, "native": reports }))?;
    Ok(if valid {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

fn task_command(connection: &ConnectionArgs, command: TaskCommand) -> CliResult<ExitCode> {
    let scheduler = connect(connection, None, None)?;
    let blocking = scheduler.blocking();
    match command {
        TaskCommand::Get { path, raw } => {
            let task = blocking.get_task(&path)?;
            if raw {
                println!("{}", task.snapshot.raw().decoded()?);
            } else {
                print_json(&registered_task_json(&task, true)?)?;
            }
        }
        TaskCommand::List {
            folder,
            recursive,
            hidden,
        } => {
            let values: Vec<_> = blocking
                .list_tasks(
                    &folder,
                    ListOptions {
                        recursive,
                        include_hidden: hidden,
                    },
                )?
                .iter()
                .map(|task| registered_task_json(task, false))
                .collect::<Result<_>>()?;
            print_json(&values)?;
        }
        TaskCommand::Register {
            path,
            xml,
            mode,
            registration_credential,
            allow_registration_triggers,
            yes,
        } => {
            require_yes(yes, "task registration creates or replaces scheduler state")?;
            let raw = RawTaskXml::new(read_file(&xml)?)?;
            let logon_type = raw.definition()?.principal.logon_type;
            let password = if password_backed(logon_type) {
                let target = registration_credential.ok_or_else(|| {
                    cli_error(
                        ErrorKind::Authentication,
                        "password-backed XML requires --registration-credential",
                    )
                })?;
                Some(Password::from_credential_manager(&target)?)
            } else {
                None
            };
            let task = blocking.register_raw(
                &path,
                raw,
                logon_type,
                RegistrationOptions {
                    mode: mode.into(),
                    ignore_registration_triggers: !allow_registration_triggers,
                    password,
                    ..RegistrationOptions::default()
                },
            )?;
            print_json(&registered_task_json(&task, false)?)?;
        }
        TaskCommand::Delete { path, yes } => {
            require_yes(yes, "task delete is destructive")?;
            blocking.delete_task(&path)?;
            print_json(&json!({ "deleted": path }))?;
        }
        TaskCommand::Enable { path } => {
            blocking.set_enabled(&path, true)?;
            print_json(&json!({ "path": path, "enabled": true }))?;
        }
        TaskCommand::Disable { path } => {
            blocking.set_enabled(&path, false)?;
            print_json(&json!({ "path": path, "enabled": false }))?;
        }
        TaskCommand::Run {
            path,
            parameters,
            ignore_constraints,
            wait,
            timeout_seconds,
        } => {
            let handle = blocking.run(
                &path,
                RunOptions {
                    parameters,
                    ignore_constraints,
                    ..RunOptions::default()
                },
            )?;
            if wait {
                let outcome: RunOutcome = blocking.wait_for_run(
                    &handle,
                    WaitOptions {
                        timeout: Duration::from_secs(timeout_seconds),
                        ..WaitOptions::default()
                    },
                )?;
                print_json(&json!({ "run": handle, "outcome": outcome }))?;
            } else {
                print_json(&handle)?;
            }
        }
        TaskCommand::Stop { path, instance } => match (path, instance) {
            (Some(path), None) => {
                blocking.stop_all(&path)?;
                print_json(&json!({ "stopped": path }))?;
            }
            (None, Some(instance)) => {
                blocking.stop_instance(instance)?;
                print_json(&json!({ "stopped_instance": instance }))?;
            }
            (None, None) => {
                return Err(cli_error(
                    ErrorKind::InvalidDefinition,
                    "task stop requires --path or --instance",
                )
                .into());
            }
            (Some(_), Some(_)) => unreachable!("clap rejects conflicting stop targets"),
        },
        TaskCommand::Running { hidden } => print_json(&blocking.running_tasks(hidden)?)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn folder_command(connection: &ConnectionArgs, command: FolderCommand) -> CliResult<ExitCode> {
    let scheduler = connect(connection, None, None)?;
    let blocking = scheduler.blocking();
    let default_security =
        SecurityInformation::OWNER | SecurityInformation::GROUP | SecurityInformation::DACL;
    match command {
        FolderCommand::List { path, recursive } => {
            print_json(&blocking.list_folders(&path, recursive)?)?;
        }
        FolderCommand::Create { path, sddl } => {
            let descriptor = sddl.map(SecurityDescriptor::from_sddl).transpose()?;
            print_json(&blocking.create_folder(&path, descriptor)?)?;
        }
        FolderCommand::Delete { path, yes } => {
            require_yes(yes, "folder delete is destructive")?;
            blocking.delete_folder(&path)?;
            print_json(&json!({ "deleted": path }))?;
        }
        FolderCommand::Security { path } => {
            let descriptor = blocking.folder_security(&path, default_security)?;
            print_json(&json!({ "path": path, "sddl": descriptor.as_sddl() }))?;
        }
        FolderCommand::SetSecurity { path, sddl, yes } => {
            require_yes(yes, "setting a folder DACL changes access rights")?;
            let descriptor = SecurityDescriptor::from_sddl(sddl)?;
            blocking.set_folder_security(&path, descriptor, SecurityInformation::DACL)?;
            print_json(&json!({ "path": path, "security_updated": true }))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn history_command(connection: &ConnectionArgs, command: HistoryCommand) -> CliResult<ExitCode> {
    let scheduler = connect(connection, None, None)?;
    let blocking = scheduler.blocking();
    match command {
        HistoryCommand::Query {
            task,
            instance,
            since_seconds,
            limit,
            forward,
        } => {
            let since = since_seconds
                .and_then(|seconds| SystemTime::now().checked_sub(Duration::from_secs(seconds)));
            print_json(&blocking.history(HistoryQuery {
                task,
                instance_id: instance,
                since,
                limit: Some(limit),
                forward,
            })?)?;
        }
        HistoryCommand::Watch {
            task,
            instance,
            poll_milliseconds,
        } => {
            let watcher = blocking.watch_history(
                HistoryQuery {
                    task,
                    instance_id: instance,
                    limit: Some(100_000),
                    ..HistoryQuery::default()
                },
                Duration::from_millis(poll_milliseconds),
            )?;
            let stdout = std::io::stdout();
            let mut output = stdout.lock();
            for event in watcher {
                serde_json::to_writer(&mut output, &event?)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        HistoryCommand::Enable { yes } => {
            require_yes(
                yes,
                "enabling history changes Event Log channel configuration",
            )?;
            blocking.set_history_enabled(true)?;
            print_json(&json!({ "history_enabled": true }))?;
        }
        HistoryCommand::Disable { yes } => {
            require_yes(
                yes,
                "disabling history changes Event Log channel configuration",
            )?;
            blocking.set_history_enabled(false)?;
            print_json(&json!({ "history_enabled": false }))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn connect(
    options: &ConnectionArgs,
    manifest_target: Option<&str>,
    manifest_credential: Option<&str>,
) -> Result<Scheduler> {
    let target = options.computer.as_deref().or(manifest_target);
    let credential_target = options
        .connection_credential
        .as_deref()
        .or(manifest_credential);
    let mut builder = Scheduler::builder();
    if let Some(target) = target {
        builder = builder.remote(target);
        if let Some(reference) = credential_target {
            builder = builder.credential(Credential::from_credential_manager(reference)?);
        }
    } else {
        if credential_target.is_some() {
            return Err(Error::new(
                ErrorKind::InvalidDefinition,
                "a connection credential requires --computer or a manifest target",
            ));
        }
        builder = builder.local();
    }
    builder.connect_blocking()
}

fn load_manifest(path: &Path, format: Option<Format>) -> Result<TaskManifest> {
    let format =
        format.map_or_else(|| DocumentFormat::from_path(path), |value| Ok(value.into()))?;
    TaskManifest::from_slice(&read_file(path)?, format)
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|error| {
        Error::new(
            ErrorKind::Serialization,
            format!("cannot read {}: {error}", path.display()),
        )
    })
}

fn manifest_connection_reference(manifest: &TaskManifest) -> Result<Option<String>> {
    let references: BTreeSet<_> = manifest
        .tasks
        .iter()
        .filter_map(|task| task.credentials.connection.clone())
        .collect();
    if references.len() > 1 {
        return Err(Error::new(
            ErrorKind::InvalidDefinition,
            "manifest tasks use conflicting connection credential references",
        ));
    }
    Ok(references.into_iter().next())
}

fn registered_task_json(
    task: &windows_task::client::RegisteredTask,
    include_xml: bool,
) -> Result<Value> {
    let definition = task.snapshot.definition().ok();
    let xml = include_xml
        .then(|| task.snapshot.raw().decoded())
        .transpose()?;
    Ok(json!({
        "path": task.path,
        "state": task.state,
        "enabled": task.enabled,
        "last_result": task.last_result,
        "missed_runs": task.missed_runs,
        "last_run": task.last_run,
        "next_run": task.next_run,
        "definition": definition,
        "xml": xml,
    }))
}

fn print_json(value: &impl serde::Serialize) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn require_yes(value: bool, operation: &str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Conflict,
            format!("{operation}; repeat with --yes after reviewing the target"),
        ))
    }
}

fn cli_error(kind: ErrorKind, message: impl Into<String>) -> Error {
    Error::new(kind, message)
}

const fn password_backed(logon_type: LogonType) -> bool {
    matches!(
        logon_type,
        LogonType::Password | LogonType::InteractiveTokenOrPassword
    )
}

struct CredentialManagerResolver<'a> {
    desired_override: Option<&'a str>,
    rollback_override: Option<&'a str>,
}

impl CredentialResolver for CredentialManagerResolver<'_> {
    fn registration_password(
        &mut self,
        path: &TaskPath,
        reference: Option<&str>,
        purpose: CredentialPurpose,
    ) -> Result<Password> {
        let target = match purpose {
            CredentialPurpose::DesiredRegistration => self.desired_override.or(reference),
            CredentialPurpose::Rollback => self.rollback_override.or(reference),
        }
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Irreversible,
                "no Credential Manager target is available for this registration",
            )
            .with_target(path.to_string())
        })?;
        Password::from_credential_manager(target)
    }
}

//! Project-local development and COM handler automation.
//!
//! Invoke from the workspace root via `cargo xtask <subcommand>`.

#![deny(missing_docs)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode},
};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use uuid::Uuid;

mod verification;

/// `cargo xtask` CLI surface.
#[derive(Debug, Parser)]
#[command(name = "xtask", version, about = "windows-task development automation")]
struct Cli {
    /// Subcommand.
    #[command(subcommand)]
    command: Command,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Scan source safety rules without requiring a Unix shell.
    StrictCode,
    /// Save platform-specific coverage without conflating compile-only targets.
    Coverage,
    /// Run formatting, Clippy, tests, and Windows cross-target checks.
    Ci {
        /// Select native mutation tests only on a disposable Windows host.
        #[arg(long, value_enum, default_value_t = verification::Suite::Portable)]
        suite: verification::Suite,
    },
    /// Run recorded test suites without disguising skipped native tests as passes.
    Test {
        /// Suite to execute. Native mutation tests require a disposable Windows host.
        #[arg(long, value_enum, default_value_t = verification::Suite::Portable)]
        suite: verification::Suite,
    },
    /// Build isolated consumers from the actual publishable crate archives.
    Package,
    /// Check the library and CLI for supported Windows architectures.
    CheckWindows {
        /// Architecture selection.
        #[arg(long, value_enum, default_value_t = Architecture::All)]
        architecture: Architecture,
    },
    /// Run host checks and tests with the promised Rust 1.85 MSRV.
    Msrv,
    /// Generate or apply COM handler registration.
    Handler {
        #[command(subcommand)]
        command: HandlerCommand,
    },
}

/// Supported handler architectures.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Architecture {
    /// Check both x64 and ARM64.
    All,
    /// x86-64 MSVC.
    X64,
    /// ARM64 MSVC.
    Arm64,
}

/// COM handler registry automation.
#[derive(Debug, Subcommand)]
enum HandlerCommand {
    /// Write an importable `.reg` file without changing the registry.
    RegFile {
        /// Handler CLSID.
        #[arg(long)]
        clsid: Uuid,
        /// Absolute handler DLL path.
        #[arg(long)]
        dll: PathBuf,
        /// Registration scope.
        #[arg(long, value_enum, default_value_t = RegistryScope::User)]
        scope: RegistryScope,
        /// Output `.reg` path.
        #[arg(long)]
        output: PathBuf,
        /// Generate removal rather than registration.
        #[arg(long)]
        unregister: bool,
    },
    /// Register an in-process handler with `reg.exe` on Windows.
    Register {
        #[arg(long)]
        clsid: Uuid,
        #[arg(long)]
        dll: PathBuf,
        #[arg(long, value_enum, default_value_t = RegistryScope::User)]
        scope: RegistryScope,
        /// Required acknowledgement for registry mutation.
        #[arg(long)]
        yes: bool,
    },
    /// Remove an in-process handler registration with `reg.exe` on Windows.
    Unregister {
        #[arg(long)]
        clsid: Uuid,
        #[arg(long, value_enum, default_value_t = RegistryScope::User)]
        scope: RegistryScope,
        /// Required acknowledgement for registry mutation.
        #[arg(long)]
        yes: bool,
    },
}

/// Registry hive used for COM activation.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum RegistryScope {
    /// Per-user registration under `HKCU\Software\Classes`.
    User,
    /// Machine-wide registration under `HKLM\Software\Classes`.
    Machine,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::StrictCode => verification::strict_code(),
        Command::Coverage => verification::coverage(),
        Command::Ci { suite } => verification::ci(suite),
        Command::Test { suite } => verification::test(suite),
        Command::Package => verification::package(),
        Command::CheckWindows { architecture } => check_windows(architecture),
        Command::Msrv => {
            cargo(&["check", "--workspace", "--all-targets", "--all-features"])?;
            cargo(&["test", "--workspace", "--all-features"])
        }
        Command::Handler { command } => handler_command(command),
    }
}

fn check_windows(architecture: Architecture) -> Result<()> {
    let targets: &[&str] = match architecture {
        Architecture::All => &["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"],
        Architecture::X64 => &["x86_64-pc-windows-msvc"],
        Architecture::Arm64 => &["aarch64-pc-windows-msvc"],
    };
    for target in targets {
        cargo(&[
            "check",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--target",
            target,
        ])?;
    }
    Ok(())
}

fn handler_command(command: HandlerCommand) -> Result<()> {
    match command {
        HandlerCommand::RegFile {
            clsid,
            dll,
            scope,
            output,
            unregister,
        } => write_reg_file(&output, clsid, &dll, scope, unregister),
        HandlerCommand::Register {
            clsid,
            dll,
            scope,
            yes,
        } => {
            require_yes(yes)?;
            if !dll.is_absolute() {
                bail!("handler DLL path must be absolute");
            }
            let key = registry_server_key(scope, clsid);
            process(
                "reg.exe",
                [
                    OsString::from("ADD"),
                    key.clone(),
                    OsString::from("/ve"),
                    OsString::from("/d"),
                    dll.as_os_str().to_owned(),
                    OsString::from("/f"),
                ],
            )?;
            process(
                "reg.exe",
                [
                    OsString::from("ADD"),
                    key,
                    OsString::from("/v"),
                    OsString::from("ThreadingModel"),
                    OsString::from("/d"),
                    OsString::from("Both"),
                    OsString::from("/f"),
                ],
            )
        }
        HandlerCommand::Unregister { clsid, scope, yes } => {
            require_yes(yes)?;
            process(
                "reg.exe",
                [
                    OsString::from("DELETE"),
                    registry_class_key(scope, clsid),
                    OsString::from("/f"),
                ],
            )
        }
    }
}

fn write_reg_file(
    output: &Path,
    clsid: Uuid,
    dll: &Path,
    scope: RegistryScope,
    unregister: bool,
) -> Result<()> {
    if !unregister && !dll.is_absolute() {
        bail!("handler DLL path must be absolute");
    }
    let hive = match scope {
        RegistryScope::User => "HKEY_CURRENT_USER\\Software\\Classes",
        RegistryScope::Machine => "HKEY_LOCAL_MACHINE\\Software\\Classes",
    };
    let key = format!("{hive}\\CLSID\\{{{clsid}}}\\InprocServer32");
    let body = if unregister {
        format!("Windows Registry Editor Version 5.00\r\n\r\n[-{key}]\r\n")
    } else {
        let path = dll.display().to_string();
        if path.contains(['\r', '\n', '\0']) {
            bail!("handler DLL path contains a registry line delimiter");
        }
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "Windows Registry Editor Version 5.00\r\n\r\n[{key}]\r\n@=\"{escaped}\"\r\n\"ThreadingModel\"=\"Both\"\r\n"
        )
    };
    let mut bytes = vec![0xFF, 0xFE];
    for unit in body.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(output, bytes).with_context(|| format!("cannot write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn registry_class_key(scope: RegistryScope, clsid: Uuid) -> OsString {
    let root = match scope {
        RegistryScope::User => "HKCU\\Software\\Classes",
        RegistryScope::Machine => "HKLM\\Software\\Classes",
    };
    OsString::from(format!("{root}\\CLSID\\{{{clsid}}}"))
}

fn registry_server_key(scope: RegistryScope, clsid: Uuid) -> OsString {
    let mut key = registry_class_key(scope, clsid);
    key.push("\\InprocServer32");
    key
}

fn require_yes(yes: bool) -> Result<()> {
    if yes {
        Ok(())
    } else {
        bail!("registry mutation requires --yes")
    }
}

fn cargo(arguments: &[&str]) -> Result<()> {
    let mut command = ProcessCommand::new("cargo");
    command.arg("+1.85.0").args(arguments);
    run_process(&mut command)
}

fn process<I>(program: &str, arguments: I) -> Result<()>
where
    I: IntoIterator<Item = OsString>,
{
    let mut command = ProcessCommand::new(program);
    command.args(arguments);
    run_process(&mut command)
}

fn run_process(command: &mut ProcessCommand) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("cannot start {command:?}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{command:?} exited with {status}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_files_are_utf16_and_never_imported_by_generation() {
        let directory = std::env::temp_dir().join(format!("handler-reg-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).expect("fixture directory");
        let clsid = Uuid::from_u128(123);
        let dll = directory.join("unicode-実行.dll");
        let output = directory.join("fixture.reg");
        for (scope, hive) in [
            (RegistryScope::User, "HKEY_CURRENT_USER"),
            (RegistryScope::Machine, "HKEY_LOCAL_MACHINE"),
        ] {
            for unregister in [false, true] {
                run(Cli {
                    command: Command::Handler {
                        command: HandlerCommand::RegFile {
                            clsid,
                            dll: dll.clone(),
                            scope,
                            output: output.clone(),
                            unregister,
                        },
                    },
                })
                .expect("generation requires no native registry access");
                let bytes = std::fs::read(&output).expect("generated file");
                assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
                let units: Vec<_> = bytes[2..]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let text = String::from_utf16(&units).expect("UTF-16 registry file");
                assert!(text.starts_with("Windows Registry Editor Version 5.00\r\n"));
                assert!(text.contains(hive));
                assert_eq!(text.contains("[-"), unregister);
                assert_eq!(text.contains("unicode-実行.dll"), !unregister);
                assert_eq!(text.contains("\"ThreadingModel\"=\"Both\""), !unregister);
            }
        }
        std::fs::remove_file(output).expect("cleanup own file");
        std::fs::remove_dir(directory).expect("cleanup own directory");
    }

    #[test]
    fn registry_mutation_requires_acknowledgement_and_absolute_input() {
        let clsid = Uuid::nil();
        for yes in [false, true] {
            handler_command(HandlerCommand::Register {
                clsid,
                dll: PathBuf::from("relative.dll"),
                scope: RegistryScope::User,
                yes,
            })
            .expect_err("unacknowledged or relative registry mutation rejected");
        }
        handler_command(HandlerCommand::Unregister {
            clsid,
            scope: RegistryScope::Machine,
            yes: false,
        })
        .expect_err("removal requires acknowledgement");
        assert!(
            registry_server_key(RegistryScope::User, clsid)
                .to_string_lossy()
                .starts_with("HKCU\\")
        );
        assert!(
            registry_class_key(RegistryScope::Machine, clsid)
                .to_string_lossy()
                .starts_with("HKLM\\")
        );
        run_process(&mut ProcessCommand::new(
            "windows-task-deliberately-missing-program",
        ))
        .expect_err("process startup error is propagated");
        run_process(ProcessCommand::new("rustc").arg("--not-a-rustc-option"))
            .expect_err("unsuccessful child is propagated");
        run_process(ProcessCommand::new("rustc").arg("--version")).expect("successful process");
    }
}

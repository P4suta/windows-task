//! Recorded, shared verification entry points for developers and CI.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use clap::ValueEnum;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum Suite {
    Portable,
    Windows,
    All,
}

struct Run {
    directory: PathBuf,
    results: Vec<Value>,
}

impl Run {
    fn new() -> Result<Self> {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/verification")
            .join(Uuid::new_v4().to_string());
        fs::create_dir_all(&directory)?;
        let capture = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default()
        };
        let environment = json!({
            "schema_version": 1,
            "started_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
            "revision": capture(&["rev-parse", "HEAD"]).trim(),
            "working_tree": capture(&["status", "--porcelain"]),
            "toolchain": "1.85.0", "features": "all-features unless specified per step"
        });
        fs::write(
            directory.join("environment.json"),
            serde_json::to_vec_pretty(&environment)?,
        )?;
        eprintln!("Verification artifacts: {}", directory.display());
        Ok(Self {
            directory,
            results: Vec::new(),
        })
    }

    fn step(
        &mut self,
        program: &str,
        arguments: &[&str],
        environment: &[(&str, &str)],
    ) -> Result<()> {
        let index = self.results.len();
        let stdout = self.directory.join(format!("{index:02}.stdout.log"));
        let stderr = self.directory.join(format!("{index:02}.stderr.log"));
        let mut command = Command::new(program);
        command.args(arguments).env("RUST_BACKTRACE", "1");
        for (key, value) in environment {
            command.env(key, value);
        }
        command.stdout(Stdio::from(fs::File::create(&stdout)?));
        command.stderr(Stdio::from(fs::File::create(&stderr)?));
        eprintln!("Running {program} {}", arguments.join(" "));
        let started = Instant::now();
        let status = command.status();
        let (outcome, exit_code, error) = match status {
            Ok(status) => (
                if status.success() { "passed" } else { "failed" },
                status.code(),
                None,
            ),
            Err(error) => ("not_run", None, Some(error.to_string())),
        };
        self.results
            .push(json!({"program": program, "arguments": arguments,
            "environment": environment, "outcome": outcome, "exit_code": exit_code,
            "elapsed_ms": started.elapsed().as_millis(), "error": error,
            "stdout": stdout.file_name().map(|name| name.to_string_lossy()),
            "stderr": stderr.file_name().map(|name| name.to_string_lossy())}));
        fs::write(
            self.directory.join("results.json"),
            serde_json::to_vec_pretty(&self.results)?,
        )?;
        // Argument arrays are authoritative and preserve quoting on every host.
        fs::write(
            self.directory.join("REPRODUCE.md"),
            "Run from the workspace root with Rust 1.85.0. For each results.json entry, execute program with the exact arguments array and environment pairs. stdout/stderr files preserve the first attempt. Never replace failures with retry results.\n",
        )?;
        if outcome != "passed" {
            eprintln!("{outcome}: see {}", self.directory.display());
        }
        Ok(())
    }

    fn cargo(&mut self, arguments: &[&str]) -> Result<()> {
        let mut args = vec!["+1.85.0"];
        args.extend_from_slice(arguments);
        self.step("cargo", &args, &[])
    }

    fn suite(&mut self, suite: Suite) -> Result<()> {
        if matches!(suite, Suite::Portable | Suite::All) {
            self.cargo(&[
                "test",
                "--locked",
                "--workspace",
                "--all-features",
                "--lib",
                "--bins",
            ])?;
            self.cargo(&["test", "--locked", "--workspace", "--all-features", "--doc"])?;
            self.cargo(&[
                "test",
                "--locked",
                "-p",
                "windows-task-cli",
                "--test",
                "contracts",
            ])?;
            self.cargo(&[
                "test",
                "--locked",
                "-p",
                "windows-task",
                "--all-features",
                "--test",
                "macro_contracts",
            ])?;
        }
        if matches!(suite, Suite::Windows | Suite::All) {
            if !cfg!(windows) {
                self.results
                    .push(json!({"suite": "windows", "outcome": "not_run",
                    "error": "requires a disposable Windows host"}));
                fs::write(
                    self.directory.join("results.json"),
                    serde_json::to_vec_pretty(&self.results)?,
                )?;
                return Ok(());
            }
            self.cargo(&[
                "build",
                "--release",
                "--manifest-path",
                "fixtures/native/Cargo.toml",
            ])?;
            let dll = std::env::current_dir()?
                .join("fixtures/native/target/release/windows_task_native_fixture.dll");
            self.step(
                "cargo",
                &[
                    "+1.85.0",
                    "test",
                    "--locked",
                    "-p",
                    "windows-task",
                    "--all-features",
                    "--lib",
                    "release_dll_lifecycle_and_panic_containment",
                    "--",
                    "--ignored",
                    "--test-threads=1",
                ],
                &[("WINDOWS_TASK_HANDLER_DLL", &dll.to_string_lossy())],
            )?;
            self.step(
                "cargo",
                &[
                    "+1.85.0",
                    "test",
                    "--locked",
                    "-p",
                    "windows-task",
                    "--all-features",
                    "--test",
                    "windows_smoke",
                ],
                &[],
            )?;
            self.step(
                "cargo",
                &[
                    "+1.85.0",
                    "test",
                    "--locked",
                    "-p",
                    "windows-task",
                    "--all-features",
                    "--test",
                    "windows_smoke",
                    "--",
                    "--ignored",
                    "--test-threads=1",
                    "--show-output",
                ],
                &[
                    ("WINDOWS_TASK_MUTATION_TESTS", "1"),
                    (
                        "WINDOWS_TASK_CLEAR_EVENT_LOG",
                        &std::env::var("WINDOWS_TASK_CLEAR_EVENT_LOG").unwrap_or_default(),
                    ),
                    (
                        "WINDOWS_TASK_EXECUTION_FIXTURE",
                        &std::env::current_dir()?
                            .join(
                                "fixtures/native/target/release/windows-task-execution-fixture.exe",
                            )
                            .to_string_lossy(),
                    ),
                ],
            )?;
        }
        Ok(())
    }

    fn finish(&self) -> Result<()> {
        if self.results.iter().any(|step| step["outcome"] != "passed") {
            bail!(
                "verification failed or was not run; artifacts: {}",
                self.directory.display()
            );
        }
        Ok(())
    }
}

pub(super) fn test(suite: Suite) -> Result<()> {
    let mut run = Run::new()?;
    run.suite(suite)?;
    run.finish()
}

pub(super) fn ci(suite: Suite) -> Result<()> {
    let mut run = Run::new()?;
    run.cargo(&["fmt", "--all", "--", "--check"])?;
    run.cargo(&[
        "clippy",
        "--locked",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--",
        "-D",
        "warnings",
    ])?;
    run.suite(suite)?;
    for features in [
        "",
        "client",
        "async",
        "history",
        "diagnostics",
        "serde",
        "reconcile",
        "recipes",
        "cron",
        "handler",
        "tracing",
    ] {
        let mut args = vec![
            "test",
            "--locked",
            "-p",
            "windows-task",
            "--lib",
            "--no-default-features",
        ];
        if !features.is_empty() {
            args.extend_from_slice(&["--features", features]);
        }
        run.cargo(&args)?;
    }
    run.step(
        "cargo",
        &[
            "+1.85.0",
            "doc",
            "--locked",
            "--workspace",
            "--all-features",
            "--no-deps",
        ],
        &[("RUSTDOCFLAGS", "-D warnings")],
    )?;
    for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
        run.cargo(&[
            "check",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--target",
            target,
        ])?;
    }
    for (program, args) in [
        ("cargo", vec!["+1.85.0", "xtask", "strict-code"]),
        ("typos", vec![]),
        ("actionlint", vec![]),
        ("yamllint", vec!["."]),
        ("cargo-deny", vec!["check"]),
    ] {
        run.step(program, &args, &[])
            .with_context(|| format!("record {program}"))?;
    }
    if cfg!(windows) {
        run.step(
            "cmd",
            &[
                "/d",
                "/c",
                "markdownlint-cli2",
                "**/*.md",
                "#target",
                "#fixtures/native/target",
                "#fuzz/target",
            ],
            &[],
        )?;
    } else {
        run.step(
            "markdownlint-cli2",
            &[
                "**/*.md",
                "#target",
                "#fixtures/native/target",
                "#fuzz/target",
            ],
            &[],
        )?;
    }
    run.finish()
}

pub(super) fn package() -> Result<()> {
    let mut run = Run::new()?;
    let version = env!("CARGO_PKG_VERSION");
    let staging = run.directory.join("source");
    fs::create_dir(&staging)?;
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "LICENSE-APACHE",
        "LICENSE-MIT",
        "NOTICE",
    ] {
        fs::copy(name, staging.join(name))?;
    }
    copy_sources(std::path::Path::new("crates"), &staging.join("crates"))?;
    let staging_manifest = staging.join("Cargo.toml");
    let staging_manifest = staging_manifest.to_string_lossy();
    let unpack = run.directory.join("packages");
    fs::create_dir(&unpack)?;
    let package_root = std::env::current_dir()?
        .join(&unpack)
        .to_string_lossy()
        .replace('\\', "/");
    let mut package_patches: Vec<String> = Vec::new();
    for name in ["windows-task-macros", "windows-task", "windows-task-cli"] {
        let mut arguments = vec![
            "package",
            "--allow-dirty",
            "--no-verify",
            "--manifest-path",
            &staging_manifest,
            "-p",
            name,
        ];
        for patch in &package_patches {
            arguments.extend_from_slice(&["--config", patch]);
        }
        run.cargo(&arguments)?;
        run.finish()?;
        let archive = staging.join(format!("target/package/{name}-{version}.crate"));
        // Preserve the actual package outside compiler caches for CI artifacts.
        fs::copy(&archive, unpack.join(format!("{name}-{version}.crate")))?;
        run.step(
            "tar",
            &[
                "-xf",
                &archive.to_string_lossy(),
                "-C",
                &unpack.to_string_lossy(),
            ],
            &[],
        )?;
        run.finish()?;
        package_patches.push(format!(
            "patch.crates-io.{name}.path='{package_root}/{name}-{version}'"
        ));
    }
    let consumer = run.directory.join("consumer");
    fs::create_dir_all(consumer.join("src"))?;
    let patches = format!(
        "[patch.crates-io]\nwindows-task = {{ path = '{package_root}/windows-task-{version}' }}\nwindows-task-macros = {{ path = '{package_root}/windows-task-macros-{version}' }}\n"
    );
    fs::write(
        consumer.join("Cargo.toml"),
        format!(
            "[package]\nname = 'verification-consumer'\nversion = '0.0.0'\nedition = '2024'\n[lib]\nname = 'verification_handler'\ncrate-type = ['cdylib']\n[workspace]\n[dependencies]\nwindows-task = {{ version = '{version}', features = ['handler'] }}\n{patches}"
        ),
    )?;
    fs::write(
        consumer.join("src/main.rs"),
        r#"use windows_task::model::{TaskDefinition, Action, ExecAction};
fn main() -> windows_task::Result<()> {
    let definition = TaskDefinition::new(Action::Exec(ExecAction::new("fixture.exe").args(["&<quoted>"])));
    let xml = windows_task::xml::to_string(&definition)?;
    assert_eq!(windows_task::xml::from_bytes(xml.as_bytes())?, definition);
    #[cfg(windows)] {
        let scheduler = windows_task::client::Scheduler::builder().local().connect_blocking()?;
        scheduler.blocking().capabilities()?;
        scheduler.shutdown(std::time::Duration::from_secs(5))?;
    }
    Ok(())
}
"#,
    )?;
    fs::copy("fixtures/native/src/lib.rs", consumer.join("src/lib.rs"))?;
    run.cargo(&[
        "run",
        "--release",
        "--manifest-path",
        &consumer.join("Cargo.toml").to_string_lossy(),
    ])?;
    run.cargo(&[
        "build",
        "--release",
        "--lib",
        "--manifest-path",
        &consumer.join("Cargo.toml").to_string_lossy(),
    ])?;
    if cfg!(windows) {
        let dll = std::env::current_dir()?
            .join(&consumer)
            .join("target/release/verification_handler.dll");
        run.step(
            "cargo",
            &[
                "+1.85.0",
                "test",
                "--locked",
                "-p",
                "windows-task",
                "--all-features",
                "--lib",
                "release_dll_lifecycle_and_panic_containment",
                "--",
                "--ignored",
                "--test-threads=1",
            ],
            &[("WINDOWS_TASK_HANDLER_DLL", &dll.to_string_lossy())],
        )?;
    }
    let cli_manifest = unpack.join(format!("windows-task-cli-{version}/Cargo.toml"));
    let mut cli = fs::read_to_string(&cli_manifest)?;
    cli.push_str("\n[workspace]\n");
    cli.push_str(&patches);
    fs::write(&cli_manifest, cli)?;
    run.cargo(&[
        "run",
        "--release",
        "--manifest-path",
        &cli_manifest.to_string_lossy(),
        "--",
        "--help",
    ])?;
    run.finish()
}

#[expect(
    clippy::filetype_is_file,
    reason = "staging must reject symlinks and special files rather than treating every non-directory as source"
)]
fn copy_sources(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == "target" {
            continue;
        }
        let destination = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_sources(&entry.path(), &destination)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), destination)?;
        } else {
            bail!(
                "source staging refuses non-regular entry: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn strict_code() -> Result<()> {
    let rules: [(&str, &str, &[&str]); 4] = [
        (
            concat!(r"\b(TO", "DO|FIX", r"ME)\b"),
            "untracked work marker",
            &[],
        ),
        (
            r"#\[allow\([a-z_:]+\)\]",
            "lint suppression without a reason",
            &[],
        ),
        (
            r"\bunsafe[[:space:]]*\{",
            "unsafe outside an audited native boundary",
            &[
                "--glob",
                "!crates/windows-task/src/client/sys.rs",
                "--glob",
                "!crates/windows-task/src/credentials.rs",
                "--glob",
                "!crates/windows-task/src/handler.rs",
            ],
        ),
        (r"#!\[feature\(", "nightly feature in MSRV source", &[]),
    ];
    let mut failed = false;
    for (pattern, description, extra) in rules {
        let output = Command::new("rg")
            .args(["-n", pattern, "--glob", "*.rs", "--glob", "!target/**"])
            .args(extra)
            .arg(".")
            .output()
            .context("strict source checks require ripgrep")?;
        if output.status.code().is_none_or(|code| code > 1) {
            bail!(
                "ripgrep failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if description == "untracked work marker"
                && line.split_once("(#").is_some_and(|(_, suffix)| {
                    suffix.split_once(')').is_some_and(|(number, _)| {
                        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                    })
                })
            {
                continue;
            }
            eprintln!("{description}: {line}");
            failed = true;
        }
    }
    if failed {
        bail!("strict source checks failed");
    }
    Ok(())
}

pub(super) fn coverage() -> Result<()> {
    let mut run = Run::new()?;
    let output = run
        .directory
        .join(format!("coverage-{}.json", std::env::consts::OS));
    let output = output.to_string_lossy();
    let mut arguments = vec![
        "exec",
        "github:taiki-e/cargo-llvm-cov",
        "--",
        "cargo",
        "+1.85.0",
        "llvm-cov",
        "--workspace",
        "--all-features",
        "--json",
        "--output-path",
        &output,
    ];
    if !cfg!(windows) {
        arguments.extend_from_slice(&["--fail-under-regions", "70"]);
    }
    run.step(
        if cfg!(windows) { "mise.exe" } else { "mise" },
        &arguments,
        &[],
    )?;
    run.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_success_failure_and_unstarted_processes() {
        let mut run = Run::new().expect("verification directory");
        run.step(
            "rustc",
            &["--version"],
            &[("VERIFICATION_FIXTURE", "recorded")],
        )
        .expect("record successful command");
        run.finish().expect("successful process is confirmed");
        run.step("rustc", &["--not-a-rustc-option"], &[])
            .expect("record rejected command");
        run.step("windows-task-deliberately-missing-program", &[], &[])
            .expect("record unstarted command");
        assert!(run.finish().is_err());
        let entries: Vec<Value> = serde_json::from_slice(
            &fs::read(run.directory.join("results.json")).expect("results file"),
        )
        .expect("structured evidence");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["outcome"], "passed");
        assert_eq!(entries[1]["outcome"], "failed");
        assert_eq!(entries[2]["outcome"], "not_run");
        assert!(entries[2]["exit_code"].is_null());
        assert_eq!(entries[0]["environment"][0][1], "recorded");
        assert!(
            fs::read_to_string(run.directory.join("00.stdout.log"))
                .expect("stdout")
                .contains("rustc")
        );
        assert!(
            fs::read_to_string(run.directory.join("01.stderr.log"))
                .expect("stderr")
                .contains("not-a-rustc-option")
        );
        assert!(run.directory.join("REPRODUCE.md").exists());
    }

    #[test]
    fn source_staging_keeps_nested_inputs_and_omits_compiler_caches() {
        let run = Run::new().expect("verification directory");
        let source = run.directory.join("fixture");
        fs::create_dir_all(source.join("nested/target")).expect("fixture directories");
        fs::write(source.join("nested/input.rs"), "source").expect("source fixture");
        fs::write(source.join("nested/target/cache"), "cache").expect("cache fixture");
        let destination = run.directory.join("staged");
        copy_sources(&source, &destination).expect("stage sources");
        assert_eq!(
            fs::read(destination.join("nested/input.rs")).expect("copied input"),
            b"source"
        );
        assert!(!destination.join("nested/target").exists());
        assert!(copy_sources(&source, &destination).is_err());
    }

    #[test]
    #[cfg(not(windows))]
    fn unavailable_native_suite_is_not_a_successful_empty_run() {
        let mut run = Run::new().expect("verification directory");
        run.suite(Suite::Windows).expect("record unavailable suite");
        assert!(run.finish().is_err());
        assert_eq!(run.results[0]["outcome"], "not_run");
    }
}

use std::{collections::BTreeMap, fs, path::PathBuf, process::Command, sync::OnceLock};

use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
struct Workflow {
    jobs: BTreeMap<String, Job>,
}

#[derive(Deserialize)]
struct Job {
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    run: Option<String>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_binary() -> &'static PathBuf {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let output = root()
            .join("target/verification/release-workflow")
            .join(Uuid::new_v4().to_string())
            .join("bin");
        fs::create_dir_all(&output).expect("controlled fixture binary directory");
        let binary = output.join(if cfg!(windows) { "gh.exe" } else { "gh" });
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg("--crate-name=release_gh_fixture")
            .arg(root().join("fixtures/release-gh.rs"))
            .arg("-o")
            .arg(&binary)
            .status()
            .expect("compile the Rust API fixture");
        assert!(status.success(), "the controlled fixture must compile");
        binary
    })
}

fn bash() -> PathBuf {
    let mut candidates = Vec::new();
    if cfg!(windows) {
        if let Some(directory) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(directory).join("Git/bin/bash.exe"));
        }
        for directory in std::env::split_paths(&std::env::var_os("PATH").expect("tool PATH")) {
            if directory.join("git.exe").is_file() {
                if let Some(parent) = directory.parent() {
                    candidates.push(parent.join("bin/bash.exe"));
                }
            }
        }
    }
    candidates.push(PathBuf::from("bash"));
    candidates
        .into_iter()
        .find(|program| {
            Command::new(program)
                .arg("--version")
                .output()
                .is_ok_and(|output| {
                    output.status.success() && output.stdout.starts_with(b"GNU bash, version")
                })
        })
        .expect("the workflow requires GNU Bash; install Git for Windows on Windows")
}

fn execute(state: &str) -> (bool, String, String) {
    let workflow: Workflow =
        serde_saphyr::from_str(include_str!("../../../.github/workflows/release.yml"))
            .expect("parse the production workflow");
    let body = workflow.jobs["release"]
        .steps
        .iter()
        .filter_map(|step| step.run.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let workspace = root()
        .join("target/verification/release-workflow")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(workspace.join("artifacts")).expect("controlled artifact directory");
    fs::write(workspace.join("artifacts/fixture.zip"), "verified archive")
        .expect("controlled archive");
    fs::write(
        workspace.join("artifacts/fixture.zip.sha256"),
        "controlled checksum",
    )
    .expect("controlled checksum file");
    fs::write(workspace.join("state"), state).expect("controlled API state");
    fs::write(workspace.join("effects"), "").expect("controlled API effect journal");
    fs::write(workspace.join("requests"), "").expect("controlled API request journal");
    let mut paths = vec![
        fixture_binary()
            .parent()
            .expect("fixture parent")
            .to_path_buf(),
    ];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").expect("tool PATH"),
    ));
    let mut shell = Command::new(bash());
    shell.env_clear();
    for name in ["SystemRoot", "COMSPEC", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            shell.env(name, value);
        }
    }
    let result = shell
        .current_dir(&workspace)
        .env(
            "PATH",
            std::env::join_paths(paths).expect("controlled fixture PATH"),
        )
        .env("RELEASE_TEST_STATE", &workspace)
        .env("GITHUB_REPOSITORY", "P4suta/release-fixture")
        .env("RELEASE_TAG", "v0.1.0")
        .args(["--noprofile", "--norc", "-c", &body])
        .output()
        .expect("execute the production algorithm against the Rust API fixture");
    fs::write(workspace.join("stdout"), result.stdout).expect("fixture stdout evidence");
    fs::write(workspace.join("stderr"), result.stderr).expect("fixture stderr evidence");
    let requests =
        fs::read_to_string(workspace.join("requests")).expect("fixture request evidence");
    assert!(
        requests.starts_with("view\n"),
        "the production workflow must reach the Rust API fixture; evidence: {}",
        workspace.display()
    );
    (
        result.status.success(),
        fs::read_to_string(workspace.join("state")).expect("final fixture state"),
        fs::read_to_string(workspace.join("effects")).expect("final fixture effect journal"),
    )
}

#[test]
fn missing_target_stays_draft_until_all_assets_are_attached() {
    let (passed, state, effects) = execute("missing");
    assert!(passed, "a new verified draft must complete");
    assert_eq!(state, "published", "the complete draft is finalized");
    assert_eq!(
        effects, "create-draft\npublish\n",
        "assets precede publication"
    );
}

#[test]
fn existing_draft_is_completed_before_publication() {
    let (passed, state, effects) = execute("draft");
    assert!(passed, "an unpublished target must complete");
    assert_eq!(state, "published", "the complete draft is finalized");
    assert_eq!(effects, "upload\npublish\n", "upload precedes publication");
}

#[test]
fn published_targets_are_rejected_without_asset_mutation() {
    let (passed, state, effects) = execute("published");
    assert!(!passed, "a published target cannot be reused");
    assert_eq!(state, "published", "the published target is preserved");
    assert_eq!(effects, "", "the client must not modify published assets");
}

#[test]
fn unavailable_metadata_never_becomes_a_publication() {
    let (passed, state, effects) = execute("unavailable");
    assert!(!passed, "an unavailable target cannot be guessed");
    assert_eq!(state, "unavailable", "unknown remote state is preserved");
    assert_eq!(effects, "", "a failed API cannot finalize a release");
}

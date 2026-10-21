use std::{collections::BTreeMap, path::PathBuf, process::Command};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Workflow {
    jobs: BTreeMap<String, Job>,
}

#[derive(Deserialize)]
struct Job {
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    run: String,
}

fn workflow() -> Workflow {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .expect("read the actual CI workflow");
    serde_saphyr::from_str(&source).expect("parse the actual CI workflow")
}

fn bash() -> PathBuf {
    if cfg!(windows) {
        let git_bash = PathBuf::from(
            std::env::var_os("ProgramFiles").expect("Windows ProgramFiles directory"),
        )
        .join("Git/bin/bash.exe");
        assert!(git_bash.is_file(), "Git for Windows supplies the CI shell");
        git_bash
    } else {
        PathBuf::from("bash")
    }
}

fn run_required(needs: &Value) -> i32 {
    let workflow = workflow();
    let required = &workflow.jobs["required"];
    let step = &required.steps[0];
    assert_eq!(
        required.needs,
        [
            "portable",
            "windows",
            "windows-arm64",
            "proofs",
            "features",
            "package"
        ],
        "the required check must depend on every contract"
    );
    let output = Command::new(bash())
        .args(["-c", &step.run])
        .env("RESULTS", needs.to_string())
        .env("CONTRACT", &step.env["CONTRACT"])
        .output()
        .expect("execute the actual required-check script");
    output.status.code().expect("the gate returns an exit code")
}

fn selected_native() -> Value {
    json!({
        "portable": {"result": "success", "outputs": {
            "native": "true", "proofs": "false", "features": "false", "package": "false"
        }},
        "windows": {"result": "success"},
        "windows-arm64": {"result": "success"},
        "proofs": {"result": "skipped"},
        "features": {"result": "skipped"},
        "package": {"result": "skipped"}
    })
}

#[test]
fn successful_selected_work_passes_the_actual_shell() {
    assert_eq!(run_required(&selected_native()), 0);
}

#[test]
fn skipping_selected_windows_work_fails_the_actual_shell() {
    let mut needs = selected_native();
    needs["windows"]["result"] = json!("skipped");
    assert_eq!(run_required(&needs), 1);
}

#[test]
fn required_contract_covers_every_supported_selection_and_result() {
    let workflow = workflow();
    let contract = &workflow.jobs["required"].steps[0].env["CONTRACT"];
    let program = format!(
        r#"
        def production: {contract};
        def cases:
          range(0; 16) as $bits |
          [range(0; 4) | if (($bits / pow(2; .) | floor) % 2) == 1
                        then "true" else "false" end] as $flags |
          range(0; 4096) as $encoded |
          [range(0; 6) | ["success", "failure", "cancelled", "skipped"]
            [(($encoded / pow(4; .) | floor) % 4)]] as $results |
          {{portable: {{result: $results[0], outputs:
              {{native: $flags[0], proofs: $flags[1], features: $flags[2], package: $flags[3]}}}},
            windows: {{result: $results[1]}}, "windows-arm64": {{result: $results[2]}},
            proofs: {{result: $results[3]}}, features: {{result: $results[4]}},
            package: {{result: $results[5]}}}} as $input |
          (["success"] + ([$flags[0], $flags[0], $flags[1], $flags[2], $flags[3]] |
            map(if . == "true" then "success" else "skipped" end))) as $expected |
          if ($input | production) == ($results == $expected)
          then 1 else error("incorrect selected-result decision") end;
        reduce cases as $case (0; . + $case) == 65536
        "#
    );
    let output = Command::new("jq")
        .args(["-en", &program])
        .output()
        .expect("run the actual jq contract over its complete finite domain");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
}

#[test]
fn missing_jobs_and_malformed_flags_fail_closed() {
    for job in [
        "portable",
        "windows",
        "windows-arm64",
        "proofs",
        "features",
        "package",
    ] {
        let mut needs = selected_native();
        needs.as_object_mut().expect("object input").remove(job);
        assert_ne!(run_required(&needs), 0, "missing {job}");
    }
    for flag in ["native", "proofs", "features", "package"] {
        for value in [json!(null), json!(true), json!(""), json!("TRUE")] {
            let mut needs = selected_native();
            needs["portable"]["outputs"][flag] = value;
            assert_ne!(run_required(&needs), 0, "malformed {flag}");
        }
    }
    let mut needs = selected_native();
    needs["unknown"] = json!({"result": "success"});
    assert_ne!(run_required(&needs), 0, "unknown job");
}

#[test]
fn release_selection_forces_all_checks_after_validating_flags() {
    let workflow = workflow();
    let step = workflow.jobs["portable"]
        .steps
        .iter()
        .find(|step| step.env.contains_key("FULL"))
        .expect("production selection");
    assert_eq!(step.env["FULL"], "${{ inputs.revision != '' }}");
    for bits in 0_u32..32 {
        let directory =
            std::env::temp_dir().join(format!("windows-task-ci-{}-{bits}", std::process::id()));
        std::fs::create_dir(&directory).expect("fresh selection output directory");
        let output_path = directory.join("outputs");
        let flags = [0, 1, 2, 3, 4].map(|bit| {
            if bits & (1 << bit) == 0 {
                "false"
            } else {
                "true"
            }
        });
        let output = Command::new(bash())
            .args(["-c", &step.run])
            .env("FULL", flags[0])
            .env("NATIVE", flags[1])
            .env("PROOFS", flags[2])
            .env("FEATURES", flags[3])
            .env("PACKAGE", flags[4])
            .env("CONTRACT", &step.env["CONTRACT"])
            .env("GITHUB_OUTPUT", &output_path)
            .output()
            .expect("execute production selection");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = std::fs::read_to_string(&output_path).expect("selection outputs");
        for (index, name) in ["native", "proofs", "features", "package"]
            .iter()
            .enumerate()
        {
            let expected = flags[0] == "true" || flags[index + 1] == "true";
            assert!(
                actual
                    .lines()
                    .any(|line| line == format!("{name}={expected}")),
                "{actual}"
            );
        }
        std::fs::remove_file(output_path).expect("remove selection output");
        std::fs::remove_dir(directory).expect("remove empty selection directory");
    }
}

#[test]
fn full_selection_cannot_hide_invalid_path_filter_outputs() {
    let workflow = workflow();
    let step = workflow.jobs["portable"]
        .steps
        .iter()
        .find(|step| step.env.contains_key("FULL"))
        .expect("production selection");
    let directory = std::env::temp_dir().join(format!("windows-task-ci-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).expect("fresh selection directory");
    let output_path = directory.join("outputs");
    for full in ["true", "false"] {
        for invalid in ["", "TRUE", "null", "1"] {
            for name in ["FULL", "NATIVE", "PROOFS", "FEATURES", "PACKAGE"] {
                let output = Command::new(bash())
                    .args(["-c", &step.run])
                    .env("FULL", full)
                    .env("NATIVE", "false")
                    .env("PROOFS", "false")
                    .env("FEATURES", "false")
                    .env("PACKAGE", "false")
                    .env(name, invalid)
                    .env("CONTRACT", &step.env["CONTRACT"])
                    .env("GITHUB_OUTPUT", &output_path)
                    .output()
                    .expect("execute production selection");
                assert!(!output.status.success(), "invalid {name} must be rejected");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("invalid selection flags"),
                    "the selector must reject the value, rather than fail to start jq"
                );
                assert_eq!(
                    std::fs::read_to_string(&output_path).expect("selection output"),
                    ""
                );
            }
        }
    }
    std::fs::remove_file(output_path).expect("remove selection output");
    std::fs::remove_dir(directory).expect("remove empty selection directory");
}

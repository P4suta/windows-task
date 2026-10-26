use std::{collections::BTreeMap, path::PathBuf, process::Command};

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Workflow {
    #[serde(rename = "on")]
    events: BTreeMap<String, Value>,
    jobs: BTreeMap<String, Job>,
}

#[derive(Deserialize)]
struct Job {
    #[serde(rename = "if")]
    condition: Option<String>,
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
fn ordinary_work_accepts_only_skipped_heavy_jobs() {
    let mut needs = selected_native();
    for flag in ["native", "proofs", "features", "package"] {
        needs["portable"]["outputs"][flag] = json!("false");
    }
    needs["windows"]["result"] = json!("skipped");
    needs["windows-arm64"]["result"] = json!("skipped");
    assert_eq!(run_required(&needs), 0);
    needs["proofs"]["result"] = json!("success");
    assert_ne!(
        run_required(&needs),
        0,
        "ordinary proof execution is unexpected"
    );
}

#[test]
fn unfinished_or_unknown_results_and_missing_flags_fail_closed() {
    for job in [
        "portable",
        "windows",
        "windows-arm64",
        "proofs",
        "features",
        "package",
    ] {
        for result in [
            json!(null),
            json!(""),
            json!("pending"),
            json!("queued"),
            json!("in_progress"),
            json!("unknown"),
        ] {
            let mut needs = selected_native();
            needs[job]["result"] = result;
            assert_ne!(run_required(&needs), 0, "unfinished or unknown {job}");
        }
    }
    for flag in ["native", "proofs", "features", "package"] {
        let mut needs = selected_native();
        needs["portable"]["outputs"]
            .as_object_mut()
            .expect("outputs")
            .remove(flag);
        assert_ne!(run_required(&needs), 0, "missing {flag}");
    }
}

#[test]
fn ordinary_events_and_heavy_job_conditions_use_the_selector() {
    let workflow = workflow();
    assert_eq!(
        workflow
            .events
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["merge_group", "pull_request", "workflow_call"]
    );
    let revision = &workflow.events["workflow_call"]["inputs"]["revision"];
    assert_eq!(revision["default"], json!(""));
    assert_eq!(revision["required"], json!(false));
    assert_eq!(revision["type"], json!("string"));
    for (job, flag) in [
        ("windows", "native"),
        ("windows-arm64", "native"),
        ("proofs", "proofs"),
        ("features", "features"),
        ("package", "package"),
    ] {
        assert_eq!(workflow.jobs[job].needs, ["portable"]);
        assert_eq!(
            workflow.jobs[job].condition.as_deref(),
            Some(format!("needs.portable.outputs.{flag} == 'true'").as_str())
        );
    }
}

#[test]
fn ordinary_revision_does_not_select_heavy_jobs_for_changed_paths() {
    let workflow = workflow();
    let step = workflow.jobs["portable"]
        .steps
        .iter()
        .find(|step| step.env.contains_key("FULL"))
        .expect("production selection");
    let directory = std::env::temp_dir().join(format!("windows-task-ci-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).expect("fresh selection directory");
    let output_path = directory.join("outputs");
    let output = Command::new(bash())
        .args(["-c", &step.run])
        .env("FULL", "false")
        .env("NATIVE", "true")
        .env("PROOFS", "true")
        .env("FEATURES", "true")
        .env("PACKAGE", "true")
        .env("CONTRACT", &step.env["CONTRACT"])
        .env("GITHUB_OUTPUT", &output_path)
        .output()
        .expect("execute the production selector");
    assert!(output.status.success(), "{output:?}");
    let actual = std::fs::read_to_string(&output_path).expect("selection outputs");
    std::fs::remove_file(output_path).expect("remove selection output");
    std::fs::remove_dir(directory).expect("remove empty selection directory");
    for name in ["native", "proofs", "features", "package"] {
        assert!(
            actual.lines().any(|line| line == format!("{name}=false")),
            "ordinary changes selected heavy work: {actual}"
        );
    }
}

#[test]
fn selection_covers_both_admitted_full_values() {
    let workflow = workflow();
    let step = workflow.jobs["portable"]
        .steps
        .iter()
        .find(|step| step.env.contains_key("FULL"))
        .expect("production selection");
    assert_eq!(step.env["FULL"], "${{ inputs.revision != '' }}");
    assert_eq!(
        step.env.keys().map(String::as_str).collect::<Vec<_>>(),
        ["CONTRACT", "FULL"]
    );
    for full in ["false", "true"] {
        let directory =
            std::env::temp_dir().join(format!("windows-task-ci-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).expect("fresh selection output directory");
        let output_path = directory.join("outputs");
        let output = Command::new(bash())
            .args(["-c", &step.run])
            .env("FULL", full)
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
        assert_eq!(
            actual.lines().count(),
            4,
            "the selector emits every job flag once"
        );
        for name in ["native", "proofs", "features", "package"] {
            assert!(
                actual.lines().any(|line| line == format!("{name}={full}")),
                "{actual}"
            );
        }
        std::fs::remove_file(output_path).expect("remove selection output");
        std::fs::remove_dir(directory).expect("remove empty selection directory");
    }
}

#[test]
fn invalid_or_missing_full_fails_before_selection() {
    let workflow = workflow();
    let step = workflow.jobs["portable"]
        .steps
        .iter()
        .find(|step| step.env.contains_key("FULL"))
        .expect("production selection");
    let directory = std::env::temp_dir().join(format!("windows-task-ci-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).expect("fresh selection directory");
    let output_path = directory.join("outputs");
    for invalid in [
        None,
        Some(""),
        Some("TRUE"),
        Some("null"),
        Some("1"),
        Some("false\n"),
    ] {
        let mut command = Command::new(bash());
        command.args(["-c", &step.run]).env_remove("FULL");
        if let Some(value) = invalid {
            command.env("FULL", value);
        }
        let output = command
            .env("CONTRACT", &step.env["CONTRACT"])
            .env("GITHUB_OUTPUT", &output_path)
            .output()
            .expect("execute production selection");
        assert!(
            !output.status.success(),
            "invalid FULL {invalid:?} must be rejected"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid full selection"),
            "the selector must reject the value, rather than fail to start jq"
        );
        assert_eq!(
            std::fs::read_to_string(&output_path).expect("selection output"),
            ""
        );
    }
    std::fs::remove_file(output_path).expect("remove selection output");
    std::fs::remove_dir(directory).expect("remove empty selection directory");
}

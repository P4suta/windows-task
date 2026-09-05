use std::{fs, path::PathBuf, process::Command};

fn directory() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/verification")
        .join(format!("cli-contract-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).expect("artifact directory");
    path
}

#[test]
fn malformed_apply_returns_json_and_exit_one_before_connecting() {
    let directory = directory();
    let manifest = directory.join("invalid.json");
    fs::write(&manifest, b"{broken").expect("invalid input");
    let output = Command::new(env!("CARGO_BIN_EXE_windows-task"))
        .args(["apply", "--yes"])
        .arg(&manifest)
        .output()
        .expect("CLI starts");
    fs::write(directory.join("stdout.json"), &output.stdout).expect("stdout evidence");
    fs::write(directory.join("stderr.log"), &output.stderr).expect("stderr evidence");
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["phase"], "preparation");
    assert!(report["report"].is_null());
    assert!(
        !report["cause"]["diagnostics"]
            .as_array()
            .expect("field diagnostics")
            .is_empty()
    );
}

#[test]
fn doctor_bundles_preconnection_failure_and_preserves_checks_on_collection_failure() {
    let directory = directory();
    let bundle = directory.join("bundle");
    for attempt in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_windows-task"))
            .args([
                "--connection-credential",
                "SENTINEL_CREDENTIAL",
                "doctor",
                "--bundle",
            ])
            .arg(&bundle)
            .output()
            .expect("CLI starts");
        fs::write(
            directory.join(format!("{attempt}.stdout.json")),
            &output.stdout,
        )
        .expect("stdout evidence");
        assert_eq!(output.status.code(), Some(2));
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("JSON report");
        let diagnostics = report["diagnostics"].as_array().expect("diagnostic checks");
        assert!(
            diagnostics
                .iter()
                .any(|entry| entry["path"] == "connection.credentials")
        );
        assert_eq!(
            diagnostics
                .iter()
                .any(|entry| entry["path"] == "bundle.collection"),
            attempt == 1
        );
    }
    let contents =
        fs::read_to_string(bundle.join("diagnostics.json")).expect("first incident preserved");
    assert!(!contents.contains("SENTINEL"));
}

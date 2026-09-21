#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    directory: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("kairo-json-output-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("fixture directory should be created");
        Self {
            config: directory.join("config.toml"),
            directory,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command
            .current_dir(&self.directory)
            .env("KAIRO_CONFIG", &self.config)
            .env_remove("KAIRO_ARTIFACT_ACCESS_KEY_ID")
            .env_remove("KAIRO_ARTIFACT_SECRET_ACCESS_KEY")
            .env_remove("KAIRO_MINIO_ACCESS_KEY_ID")
            .env_remove("KAIRO_MINIO_SECRET_ACCESS_KEY")
            .env_remove("KAIRO_R2_ACCESS_KEY_ID")
            .env_remove("KAIRO_R2_SECRET_ACCESS_KEY");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn parse_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("output should be valid JSON")
}

#[test]
fn runs_json_is_an_empty_array_with_no_local_runs() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["--json", "runs"])
        .output()
        .expect("kairo should start");
    assert!(output.status.success());
    let value = parse_json(&output.stdout);
    assert_eq!(value, serde_json::json!([]));
    assert!(!output.stdout.contains(&0x1b));
}

#[test]
fn runs_json_lists_a_completed_run() {
    let fixture = Fixture::new();
    let workflow = fixture.directory.join("workflow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: json-runs-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\nedges: []\n",
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../demos/basic/multiply-by-nine.wat")
                .display(),
        ),
    )
    .expect("workflow should be written");
    assert!(
        fixture
            .command()
            .arg("run")
            .arg(&workflow)
            .arg("--run")
            .arg("json-run")
            .output()
            .expect("kairo should start")
            .status
            .success()
    );

    let output = fixture
        .command()
        .args(["--json", "runs"])
        .output()
        .expect("kairo should start");
    assert!(output.status.success());
    let value = parse_json(&output.stdout);
    let runs = value.as_array().expect("output should be a JSON array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["name"], "json-run");
    assert_eq!(runs[0]["workflow"], "json-runs-test");
    assert_eq!(runs[0]["kind"], "cell");
}

#[test]
fn a_usage_error_reports_as_a_single_json_object_with_exit_code_two() {
    let fixture = Fixture::new();
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    let output = fixture
        .command()
        .arg("--json")
        .arg("run")
        .arg(&component)
        .arg("--materialize")
        .output()
        .expect("kairo should start");
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let value = parse_json(&output.stdout);
    assert!(value["error"].as_str().unwrap().contains("--materialize"));
    assert!(
        output.stderr.is_empty(),
        "errors go to stdout in --json mode"
    );
}

#[test]
fn quiet_does_not_change_the_primary_result_or_exit_code() {
    // `status()` already no-ops whenever stderr isn't a terminal (true for any piped test
    // harness), so a captured-stdio comparison can't observe the "loud" case's status lines --
    // this asserts `--quiet` doesn't change behavior, not that it silences a terminal.
    let fixture = Fixture::new();
    let workflow = fixture.directory.join("workflow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: quiet-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\nedges: []\n",
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../demos/basic/multiply-by-nine.wat")
                .display(),
        ),
    )
    .expect("workflow should be written");

    let loud = fixture
        .command()
        .arg("run")
        .arg(&workflow)
        .output()
        .expect("kairo should start");
    assert!(loud.status.success());

    let quiet = fixture
        .command()
        .args(["--quiet", "run"])
        .arg(&workflow)
        .output()
        .expect("kairo should start");
    assert!(quiet.status.success());
    assert!(
        quiet.stderr.is_empty(),
        "quiet run must not print status lines even if stderr were a terminal"
    );
    assert_eq!(
        quiet.stdout, loud.stdout,
        "the primary result is unaffected"
    );
}

#[test]
fn components_json_lists_a_registered_component() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .command()
            .arg("init")
            .arg("--no-storage")
            .output()
            .expect("kairo should start")
            .status
            .success()
    );
    let component =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/basic/multiply-by-nine.wat");
    assert!(
        fixture
            .command()
            .arg("add")
            .arg(&component)
            .arg("--name")
            .arg("multiply")
            .output()
            .expect("kairo should start")
            .status
            .success()
    );

    let output = fixture
        .command()
        .args(["--json", "components"])
        .output()
        .expect("kairo should start");
    assert!(output.status.success());
    let value = parse_json(&output.stdout);
    let components = value.as_array().expect("output should be a JSON array");
    assert!(
        components
            .iter()
            .any(|component| component["name"] == "multiply"),
        "{components:?}"
    );
}

#[test]
fn doctor_json_is_a_single_document_on_failure() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["--json", "doctor"])
        .output()
        .expect("kairo should start");
    assert!(!output.status.success());
    // must parse as exactly one JSON value; two concatenated objects would fail this.
    let value = parse_json(&output.stdout);
    assert_eq!(value["healthy"], false);
}

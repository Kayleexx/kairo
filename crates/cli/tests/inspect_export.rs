#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

struct Fixture {
    directory: PathBuf,
    config: PathBuf,
    workflow: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("kairo-inspect-export-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("fixture directory should be created");
        let sample = directory.join("sample.txt");
        fs::write(&sample, "Contact alice@example.com or 9876543210.\n")
            .expect("sample input should be written");
        let workflow = directory.join("workflow.yaml");
        fs::write(
            &workflow,
            format!(
                "workflow: redact-export-test\naccepts: [txt]\nmode: stream\ninput: {}\noutput:\n  filename: redacted.txt\n  content_type: text/plain\nsteps:\n  - name: normalize-and-redact\n    component: {}\nedges: []\n",
                sample.display(),
                repository_path("components/reference/doc-redact/component.wasm").display(),
            ),
        )
        .expect("workflow should be written");
        Self {
            directory: directory.clone(),
            config: directory.join("config.toml"),
            workflow,
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

#[test]
fn re_exports_a_never_locally_exported_artifact_byte_identical() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .command()
            .args(["init", "--local"])
            .output()
            .expect("kairo should start")
            .status
            .success()
    );

    let baseline_path = fixture.directory.join("baseline.txt");
    let baseline_run = fixture
        .command()
        .arg("run")
        .arg(&fixture.workflow)
        .arg("--run")
        .arg("baseline")
        .arg("-o")
        .arg(&baseline_path)
        .output()
        .expect("kairo should start");
    assert!(baseline_run.status.success(), "{baseline_run:?}");
    let baseline_bytes = fs::read(&baseline_path).expect("baseline output should exist");

    let no_export_run = fixture
        .command()
        .arg("run")
        .arg(&fixture.workflow)
        .arg("--run")
        .arg("no-export")
        .arg("--no-export")
        .output()
        .expect("kairo should start");
    assert!(no_export_run.status.success(), "{no_export_run:?}");
    let never_exported = fixture.directory.join("redacted.txt");
    assert!(
        !never_exported.exists(),
        "no-export run must not leave a local file"
    );

    let re_export_path = fixture.directory.join("re-exported.txt");
    let inspect = fixture
        .command()
        .args(["inspect", "no-export", "--export"])
        .arg(&re_export_path)
        .output()
        .expect("kairo should start");
    assert!(inspect.status.success(), "{inspect:?}");
    let stdout = String::from_utf8(inspect.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("exported ·"), "got: {stdout}");

    let re_exported_bytes = fs::read(&re_export_path).expect("re-exported output should exist");
    assert_eq!(
        baseline_bytes, re_exported_bytes,
        "re-exported artifact must be byte-identical to the original output"
    );
}

#[test]
fn rejects_export_for_a_workflow_without_a_declared_output() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .command()
            .args(["init", "--local"])
            .output()
            .expect("kairo should start")
            .status
            .success()
    );
    let scalar_workflow = fixture.directory.join("scalar.yaml");
    fs::write(
        &scalar_workflow,
        format!(
            "workflow: scalar-export-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\nedges: []\n",
            repository_path("demos/basic/multiply-by-nine.wat").display(),
        ),
    )
    .expect("workflow should be written");
    assert!(
        fixture
            .command()
            .arg("run")
            .arg(&scalar_workflow)
            .arg("--run")
            .arg("scalar-run")
            .output()
            .expect("kairo should start")
            .status
            .success()
    );

    let output = fixture
        .command()
        .args(["inspect", "scalar-run", "--export"])
        .arg(fixture.directory.join("nope.bin"))
        .output()
        .expect("kairo should start");
    assert!(!output.status.success());
}

#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("kairo-inspection-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("fixture directory should be created");
        let config = directory.join("config.toml");
        Self { directory, config }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command
            .current_dir(&self.directory)
            .env("KAIRO_CONFIG", &self.config);
        command
    }

    fn initialize(&self) {
        let output = self
            .command()
            .args(["init", "--local"])
            .output()
            .expect("kairo should start");
        assert!(output.status.success());
    }

    fn run(&self, workflow: &str) {
        let output = self
            .command()
            .arg("run")
            .arg(repository_path(workflow))
            .arg("--state")
            .output()
            .expect("kairo should start");
        assert!(output.status.success());
    }

    fn run_named(&self, workflow: &str, cell: &str) {
        let output = self
            .command()
            .arg("run")
            .arg(repository_path(workflow))
            .args(["--run", cell])
            .output()
            .expect("kairo should start");
        assert!(output.status.success());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn repository_path(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[test]
fn renders_a_validated_workflow_graph() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .arg("workflows")
        .arg(repository_path("demos/checkout/workflow.yaml"))
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("checkout-settlement · scalar · 4 components"));
    assert!(stdout.contains("ephemeral → add-tax"));
    assert!(stdout.contains("required checkpoint → add-shipping"));
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn lists_and_inspects_a_cell_without_a_database_path() {
    let fixture = Fixture::new();
    fixture.initialize();
    fixture.run("demos/checkout/workflow.yaml");

    let listed = fixture
        .command()
        .arg("runs")
        .output()
        .expect("kairo should start");
    assert!(listed.status.success());
    let stdout = String::from_utf8(listed.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("checkout-settlement · completed · output 3207"));
    assert!(stdout.contains("next · kairo inspect"));

    let inspected = fixture
        .command()
        .arg("inspect")
        .output()
        .expect("kairo should start");
    assert!(inspected.status.success());
    assert!(inspected.stderr.is_empty());
    let stdout = String::from_utf8(inspected.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("state · completed"));
    assert!(stdout.contains("apply-discount · 2500 → 2250"));
    assert!(stdout.contains("checkpoint · sha256:"));
    assert!(stdout.contains("duration ·"));
    assert!(!stdout.contains("duration unavailable"));
}

#[test]
fn creates_a_new_durable_run_without_user_supplied_state() {
    let fixture = Fixture::new();
    fixture.initialize();

    for _ in 0..2 {
        let output = fixture
            .command()
            .args(["run"])
            .arg(repository_path("demos/checkout/workflow.yaml"))
            .output()
            .expect("kairo should start");
        assert!(output.status.success());
    }

    let listed = fixture
        .command()
        .arg("cells")
        .output()
        .expect("kairo should start");
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("runs · 2"));
}

#[test]
fn inspects_the_most_recent_run_when_multiple_exist() {
    let fixture = Fixture::new();
    fixture.initialize();
    fixture.run("demos/basic/workflow.yaml");
    fixture.run("demos/checkout/workflow.yaml");

    let latest = fixture
        .command()
        .arg("inspect")
        .output()
        .expect("kairo should start");
    assert!(latest.status.success());
    assert!(latest.stdout.starts_with(b"checkout-settlement\n"));

    let selected = fixture
        .command()
        .args(["inspect", "checkout-settlement"])
        .output()
        .expect("kairo should start");
    assert!(selected.status.success());
    assert!(selected.stdout.starts_with(b"checkout-settlement\n"));
}

#[test]
fn explains_when_no_cells_exist() {
    let fixture = Fixture::new();

    let listed = fixture
        .command()
        .arg("cells")
        .output()
        .expect("kairo should start");
    assert!(listed.status.success());
    assert_eq!(
        listed.stdout,
        b"no runs found \xc2\xb7 run a workflow first\n"
    );

    let inspected = fixture
        .command()
        .arg("inspect")
        .output()
        .expect("kairo should start");
    assert!(!inspected.status.success());
    assert!(inspected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&inspected.stderr).contains("no runs found"));
}

#[test]
fn names_cells_and_filters_them_by_workflow() {
    let fixture = Fixture::new();
    fixture.initialize();
    fixture.run_named("demos/checkout/workflow.yaml", "order-1042");
    fixture.run_named("demos/checkout/workflow.yaml", "order-1043");
    fixture.run_named("demos/basic/workflow.yaml", "temperature-1");

    let listed = fixture
        .command()
        .args(["runs", "checkout-settlement"])
        .output()
        .expect("kairo should start");
    assert!(listed.status.success());
    let stdout = String::from_utf8(listed.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("order-1042 · checkout-settlement"));
    assert!(stdout.contains("order-1043 · checkout-settlement"));
    assert!(!stdout.contains("temperature-1"));

    let ambiguous = fixture
        .command()
        .args(["inspect", "checkout-settlement"])
        .output()
        .expect("kairo should start");
    assert!(!ambiguous.status.success());
    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(stderr.contains("order-1042, order-1043"));

    let workflows = fixture
        .command()
        .arg("workflows")
        .output()
        .expect("kairo should start");
    assert!(workflows.status.success());
    let stdout = String::from_utf8(workflows.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("checkout-settlement · 2 runs · 2 completed"));
    assert!(stdout.contains("celsius-to-fahrenheit · 1 runs · 1 completed"));
}

#[test]
fn verifies_a_cells_local_checkpoint() {
    let fixture = Fixture::new();
    fixture.initialize();
    fixture.run_named("demos/checkout/workflow.yaml", "verified-order");

    let output = fixture
        .command()
        .args(["inspect", "verified-order", "--verify"])
        .output()
        .expect("kairo should start");
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("verification · 1 checkpoint(s) available in local storage")
    );
}

#[test]
fn rejects_an_invalid_run_name() {
    let fixture = Fixture::new();
    fixture.initialize();
    let output = fixture
        .command()
        .arg("run")
        .arg(repository_path("demos/basic/workflow.yaml"))
        .args(["--cell", "../../outside"])
        .output()
        .expect("kairo should start");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid run name"));
    assert!(!fixture.directory.join("outside.db").exists());
}

#[test]
fn reports_a_bad_journal_without_hiding_healthy_cells() {
    let fixture = Fixture::new();
    fixture.initialize();
    fixture.run_named("demos/basic/workflow.yaml", "healthy");
    fs::write(fixture.directory.join(".kairo/broken.db"), b"not sqlite")
        .expect("invalid journal should be written");

    let output = fixture
        .command()
        .arg("cells")
        .output()
        .expect("kairo should start");
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("healthy"));
    assert!(stdout.contains("broken · invalid"));
}

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::{Path, PathBuf},
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
            std::env::temp_dir().join(format!("kairo-setup-{}-{sequence}", process::id()));
        fs::create_dir(&directory).expect("fixture directory should be created");
        let config = directory.join("config.toml");
        Self { directory, config }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command
            .current_dir(&self.directory)
            .env("KAIRO_CONFIG", &self.config)
            .env_remove("AWS_ACCESS_KEY_ID")
            .env_remove("AWS_SECRET_ACCESS_KEY");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn configures_an_external_store_without_credentials() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args([
            "init",
            "--endpoint",
            "https://storage.example.test",
            "--bucket",
            "workflows",
        ])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("storage configured"));
    assert_eq!(
        fs::read_to_string(&fixture.config).expect("config should be written"),
        "[storage]\nendpoint = \"https://storage.example.test\"\nbucket = \"workflows\"\nlocal = false\n"
    );
}

#[test]
fn configures_a_local_filesystem_store_without_credentials() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["init", "--local"])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("local storage ready"));
    assert!(
        fs::read_to_string(&fixture.config)
            .expect("config should be written")
            .contains("local = true")
    );
    assert!(!fixture.directory.join(".env").exists());
}

#[test]
fn explains_the_next_step_for_r2_configuration() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args([
            "init",
            "--endpoint",
            "https://account.r2.cloudflarestorage.com",
        ])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("R2 configured"));
    assert!(stdout.contains("add R2 credentials to .env"));
}

#[test]
fn storage_check_requires_credentials_without_contacting_the_endpoint() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[storage]\nendpoint = \"https://storage.example.test\"\nbucket = \"workflows\"\n",
    )
    .expect("config should be written");

    let output = fixture
        .command()
        .args(["storage", "check"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("storage credentials need"));
}

#[test]
fn rejects_a_bucket_without_an_external_endpoint() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["init", "--bucket", "workflows"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("storage endpoint is required"));
    assert!(!Path::new(&fixture.config).exists());
}

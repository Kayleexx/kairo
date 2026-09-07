#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("external artifact storage active"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("storage.example.test"));
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
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("local artifact storage active · .kairo/artifacts")
    );
    assert!(
        fs::read_to_string(&fixture.config)
            .expect("config should be written")
            .contains("endpoint = \".kairo/artifacts\"\nbucket = \"\"\nlocal = true")
    );
    assert!(!fixture.directory.join(".env").exists());
}

#[test]
fn storage_check_proves_the_local_store() {
    let fixture = Fixture::new();
    let initialized = fixture
        .command()
        .args(["init", "--local"])
        .output()
        .expect("kairo should start");
    assert!(initialized.status.success());

    let output = fixture
        .command()
        .args(["storage", "check"])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("valid local artifact storage · write/read verified · sha256:"));
    assert!(fixture.directory.join(".kairo/artifacts").is_dir());
}

#[test]
fn omits_the_r2_next_step_when_credentials_are_available() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .env("KAIRO_R2_ACCESS_KEY_ID", "mock-access")
        .env("KAIRO_R2_SECRET_ACCESS_KEY", "mock-secret")
        .args([
            "init",
            "--endpoint",
            "https://account.r2.cloudflarestorage.com",
        ])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("R2 artifact storage active"));
    assert!(!stdout.contains("add R2 credentials to .env"));
    assert!(!stdout.contains("https://account.r2.cloudflarestorage.com"));
}

#[test]
fn initialized_storage_overrides_environment_storage() {
    let fixture = Fixture::new();
    let initialized = fixture
        .command()
        .args(["init", "--local"])
        .output()
        .expect("kairo should start");
    assert!(initialized.status.success());

    let output = fixture
        .command()
        .env("KAIRO_ARTIFACT_ENDPOINT", "https://storage.example.test")
        .env("KAIRO_ARTIFACT_BUCKET", "workflows")
        .args(["storage", "check"])
        .output()
        .expect("kairo should start");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("valid local artifact storage · write/read verified · sha256:")
    );
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("KAIRO_ARTIFACT_ACCESS_KEY_ID"));
}

#[test]
fn storage_check_explains_missing_r2_credentials() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.config,
        "[storage]\nendpoint = \"https://account.r2.cloudflarestorage.com\"\nbucket = \"workflows\"\n",
    )
    .expect("config should be written");

    let output = fixture
        .command()
        .args(["storage", "check"])
        .output()
        .expect("kairo should start");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("KAIRO_R2_ACCESS_KEY_ID"));
}

#[cfg(unix)]
#[test]
fn configures_r2_from_an_account_id_in_dotenv() {
    let fixture = Fixture::new();
    fs::write(
        fixture.directory.join(".env"),
        "KAIRO_R2_ACCOUNT_ID=account\nKAIRO_ARTIFACT_BUCKET=workflows\n",
    )
    .expect("dotenv should be written");
    let command_line = format!("{} init", env!("CARGO_BIN_EXE_kairo"));
    let mut command = Command::new("script");
    command
        .current_dir(&fixture.directory)
        .env("KAIRO_CONFIG", &fixture.config)
        .env_remove("KAIRO_ARTIFACT_ENDPOINT")
        .env_remove("KAIRO_ARTIFACT_BUCKET")
        .env_remove("KAIRO_R2_ACCOUNT_ID")
        .env_remove("KAIRO_R2_ACCESS_KEY_ID")
        .env_remove("KAIRO_R2_SECRET_ACCESS_KEY")
        .args(["-q", "-c", &command_line, "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn().expect("script should start");
    child
        .stdin
        .take()
        .expect("script input should be available")
        .write_all(b"r2\n")
        .expect("selection should be written");
    let output = child.wait_with_output().expect("script should exit");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("output should be UTF-8");
    assert!(stdout.contains("R2 artifact storage active"));
    assert!(!stdout.contains("https://account.r2.cloudflarestorage.com"));
    assert_eq!(
        fs::read_to_string(&fixture.config).expect("config should be written"),
        "[storage]\nendpoint = \"https://account.r2.cloudflarestorage.com\"\nbucket = \"workflows\"\nlocal = false\n"
    );
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

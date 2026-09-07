#![allow(clippy::expect_used, clippy::unwrap_used)]

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        io::{BufRead, BufReader},
        path::{Path, PathBuf},
        process::{self, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
    };

    use rusqlite::Connection;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        directory: PathBuf,
        workflow: PathBuf,
        state: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let directory =
                std::env::temp_dir().join(format!("kairo-restart-{}-{sequence}", process::id()));
            fs::create_dir(&directory).expect("fixture directory should be created");
            let workflow = directory.join("workflow.yaml");
            let state = directory.join("cell.db");
            fs::write(
                &workflow,
                format!(
                    "workflow: restart-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\n  - name: slow\n    component: {}\nedges:\n  - from: multiply\n    to: slow\n",
                    repository_path("demos/basic/multiply-by-nine.wat").display(),
                    repository_path("crates/cli/tests/fixtures/slow-console.wat").display()
                ),
            )
            .expect("workflow should be written");
            Self {
                directory,
                workflow,
                state,
            }
        }

        fn durable() -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "kairo-durable-restart-{}-{sequence}",
                process::id()
            ));
            fs::create_dir(&directory).expect("fixture directory should be created");
            let workflow = directory.join("workflow.yaml");
            let state = directory.join("cell.db");
            fs::write(
                &workflow,
                format!(
                    "workflow: durable-restart-test\ninput: 20\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\n  - name: slow\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n  - from: divide\n    to: slow\n    durability: required\n",
                    repository_path("demos/basic/multiply-by-nine.wat").display(),
                    repository_path("demos/basic/divide-by-five.wat").display(),
                    repository_path("crates/cli/tests/fixtures/slow-console.wat").display(),
                ),
            )
            .expect("workflow should be written");
            Self {
                directory,
                workflow,
                state,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn reconstructs_completed_work_after_sigkill() {
        let fixture = Fixture::new();
        let mut child = Command::new(env!("CARGO_BIN_EXE_kairo"))
            .args(["--allow-console", "--verbose", "run"])
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kairo should start");
        let stderr = child.stderr.take().expect("stderr should be piped");
        let mut saw_slow_start = false;
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("diagnostic should be readable");
            if line.contains("cell component started") && line.contains("step=\"slow\"") {
                saw_slow_start = true;
                break;
            }
        }
        assert!(
            saw_slow_start,
            "slow component should start before termination"
        );
        child.kill().expect("kairo should be killed");
        assert!(!child.wait().expect("kairo should exit").success());

        let resumed = Command::new(env!("CARGO_BIN_EXE_kairo"))
            .arg("--allow-console")
            .arg("run")
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .stderr(Stdio::null())
            .output()
            .expect("kairo should restart");

        assert!(resumed.status.success());
        assert_eq!(resumed.stdout, b"180\n");
        assert_eq!(event_count(&fixture.state, "component_started", 0), 1);
        assert_eq!(event_count(&fixture.state, "component_completed", 0), 1);
        assert_eq!(event_count(&fixture.state, "component_started", 1), 2);
        assert_eq!(event_count(&fixture.state, "component_completed", 1), 1);
    }

    #[test]
    fn restores_a_minio_checkpoint_after_sigkill() {
        if !minio_configured() {
            return;
        }
        let fixture = Fixture::durable();
        let mut child = Command::new(env!("CARGO_BIN_EXE_kairo"))
            .args(["--allow-console", "--verbose", "run"])
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kairo should start");
        wait_for_slow_start(child.stderr.take().expect("stderr should be piped"));
        child.kill().expect("kairo should be killed");
        assert!(!child.wait().expect("kairo should exit").success());

        let resumed = Command::new(env!("CARGO_BIN_EXE_kairo"))
            .arg("--allow-console")
            .arg("run")
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .stderr(Stdio::null())
            .output()
            .expect("kairo should restart");

        assert!(resumed.status.success());
        assert_eq!(resumed.stdout, b"36\n");
        assert_eq!(event_count(&fixture.state, "component_started", 0), 1);
        assert_eq!(event_count(&fixture.state, "component_started", 1), 1);
        assert_eq!(event_count(&fixture.state, "component_started", 2), 2);
        assert_eq!(event_count(&fixture.state, "checkpoint_created", 1), 1);
    }

    fn wait_for_slow_start(stderr: impl std::io::Read) {
        let mut saw_slow_start = false;
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("diagnostic should be readable");
            if line.contains("cell component started") && line.contains("step=\"slow\"") {
                saw_slow_start = true;
                break;
            }
        }
        assert!(
            saw_slow_start,
            "slow component should start before termination"
        );
    }

    fn minio_configured() -> bool {
        std::env::var_os("KAIRO_MINIO_ENDPOINT").is_some()
            && std::env::var_os("KAIRO_ARTIFACT_BUCKET").is_some()
    }

    fn event_count(path: &Path, kind: &str, index: i64) -> i64 {
        Connection::open(path)
            .expect("journal should open")
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind = ?1 AND step_index = ?2",
                (kind, index),
                |row| row.get(0),
            )
            .expect("event count should load")
    }

    fn repository_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }
}

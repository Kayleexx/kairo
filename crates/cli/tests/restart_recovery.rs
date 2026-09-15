#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

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
            .current_dir(&fixture.directory)
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
            .current_dir(&fixture.directory)
            .arg("--allow-console")
            .arg("run")
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .output()
            .expect("kairo should restart");

        assert!(
            resumed.status.success(),
            "resume failed: {}",
            String::from_utf8_lossy(&resumed.stderr)
        );
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
            .current_dir(&fixture.directory)
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
            .current_dir(&fixture.directory)
            .arg("--allow-console")
            .arg("run")
            .arg(&fixture.workflow)
            .arg("--state")
            .arg(&fixture.state)
            .output()
            .expect("kairo should restart");

        assert!(
            resumed.status.success(),
            "resume failed: {}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert_eq!(resumed.stdout, b"36\n");
        assert_eq!(event_count(&fixture.state, "component_started", 0), 1);
        assert_eq!(event_count(&fixture.state, "component_started", 1), 1);
        assert_eq!(event_count(&fixture.state, "checkpoint_created", 1), 1);
        // the required boundary after "divide" starts a second ExecutionGroup, with its own
        // journal at a real, discoverable sibling path -- "slow" (step 2) lives there, not in
        // the first group's `cell.db`.
        let second_group = kairo_worker::group_state_path(&fixture.state, 2);
        assert_eq!(event_count(&second_group, "component_started", 2), 2);
    }

    /// blocks until the real `cell component started ... step="slow"` diagnostic line appears
    /// (no fixed timeout or sleep -- driven entirely by the child's own real output), or reports
    /// every line seen if the process exited first, so a failure here shows *why* it exited
    /// instead of just that the expected line never came.
    fn wait_for_slow_start(stderr: impl std::io::Read) {
        let mut seen = Vec::new();
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("diagnostic should be readable");
            if line.contains("cell component started") && line.contains("step=\"slow\"") {
                return;
            }
            seen.push(line);
        }
        panic!(
            "slow component never started before the process exited; full diagnostic output:\n{}",
            seen.join("\n")
        );
    }

    fn minio_configured() -> bool {
        std::env::var_os("KAIRO_ARTIFACT_ENDPOINT").is_some()
            && std::env::var_os("KAIRO_ARTIFACT_BUCKET").is_some()
            && std::env::var_os("KAIRO_MINIO_ACCESS_KEY_ID").is_some()
            && std::env::var_os("KAIRO_MINIO_SECRET_ACCESS_KEY").is_some()
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

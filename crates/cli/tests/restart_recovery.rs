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

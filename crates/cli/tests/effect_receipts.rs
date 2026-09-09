#![allow(clippy::expect_used)]

#[cfg(unix)]
mod unix {
    use rusqlite::Connection;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    struct Fixture {
        directory: PathBuf,
        provider: Child,
        control: Child,
    }

    impl Fixture {
        fn start() -> Self {
            let directory =
                std::env::temp_dir().join(format!("kairo-effects-{}", std::process::id()));
            let _ = fs::remove_dir_all(&directory);
            fs::create_dir(&directory).expect("fixture directory should be created");
            let provider = command(&directory)
                .args(["effects", "serve", "--response-delay-ms", "3500"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("provider should start");
            wait_for(
                &directory.join(".kairo/effects.addr"),
                Duration::from_secs(5),
            );
            let control = command(&directory)
                .args(["start", "--workers", "2"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("control should start");
            wait_until(Duration::from_secs(5), || {
                command(&directory)
                    .arg("workers")
                    .output()
                    .is_ok_and(|output| {
                        output.status.success()
                            && String::from_utf8_lossy(&output.stdout).contains("workers · 2")
                    })
            });
            Self {
                directory,
                provider,
                control,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for worker in ["worker-1", "worker-2"] {
                let _ = command(&self.directory)
                    .args(["chaos", "kill", worker])
                    .output();
            }
            let _ = self.control.kill();
            let _ = self.control.wait();
            let _ = self.provider.kill();
            let _ = self.provider.wait();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn worker_crash_does_not_duplicate_an_effect() {
        let fixture = Fixture::start();
        let workflow =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/effects/workflow.yaml");
        let mut run = command(&fixture.directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "receipt-crash"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run should start");
        let database = fixture.directory.join(".kairo/effects.db");
        wait_until(Duration::from_secs(5), || effect_count(&database) == 1);
        let killed = command(&fixture.directory)
            .args(["chaos", "kill", "worker-1"])
            .output()
            .expect("chaos command should run");
        assert!(killed.status.success());
        wait_until(Duration::from_secs(12), || {
            run.try_wait().expect("run state should load").is_some()
        });
        assert!(run.wait().expect("run should exit").success());
        assert_eq!(effect_count(&database), 1);
        let state = fixture.directory.join(".kairo/receipt-crash.db");
        let status: String = Connection::open(state)
            .expect("receipt journal should open")
            .query_row("SELECT status FROM effect_receipts", [], |row| row.get(0))
            .expect("receipt should exist");
        assert_eq!(status, "committed");
    }

    fn command(directory: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command.current_dir(directory);
        command
    }
    fn wait_for(path: &Path, timeout: Duration) {
        wait_until(timeout, || path.exists());
    }
    fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !ready() {
            assert!(Instant::now() < deadline, "operation timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn effect_count(path: &Path) -> i64 {
        Connection::open(path)
            .ok()
            .and_then(|db| {
                db.query_row("SELECT COUNT(*) FROM effects", [], |row| row.get(0))
                    .ok()
            })
            .unwrap_or(0)
    }
}

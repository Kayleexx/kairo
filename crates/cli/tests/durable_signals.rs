#![allow(clippy::expect_used)]

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
        thread,
        time::{Duration, Instant},
    };

    struct Fixture {
        directory: PathBuf,
        control: Child,
    }

    impl Fixture {
        fn start() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "kairo-signals-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&directory);
            fs::create_dir(&directory).expect("fixture directory should be created");
            let control = command(&directory)
                .args(["start", "--workers", "2", "--foreground"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("control should start");
            wait_until(Duration::from_secs(5), || {
                command(&directory)
                    .arg("workers")
                    .output()
                    .is_ok_and(|output| {
                        String::from_utf8_lossy(&output.stdout).contains("workers · 2")
                    })
            });
            Self { directory, control }
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
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn signal_survives_worker_loss_and_resumes_once() {
        let fixture = Fixture::start();
        let workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../demos/reference/approval/workflow.yaml");
        let mut run = command(&fixture.directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "approval-e2e", "--watch"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run should start");
        let killed = command(&fixture.directory)
            .args(["chaos", "kill", "worker-1"])
            .output()
            .expect("worker kill should run");
        assert!(killed.status.success());
        wait_until(Duration::from_secs(5), || {
            command(&fixture.directory)
                .args(["signal", "approval-e2e"])
                .output()
                .is_ok_and(|output| output.status.success())
        });
        wait_until(Duration::from_secs(5), || {
            run.try_wait().expect("run state should load").is_some()
        });
        let output = run.wait_with_output().expect("run should finish");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"3154\n");
    }

    #[test]
    fn timer_resumes_after_worker_loss() {
        let fixture = Fixture::start();
        let killed = command(&fixture.directory)
            .args(["chaos", "kill", "worker-1"])
            .output()
            .expect("worker kill should run");
        assert!(killed.status.success());
        let workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../demos/reference/delayed-processing/workflow.yaml");
        let output = command(&fixture.directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "timer-e2e", "--watch"])
            .output()
            .expect("timer workflow should run");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"3498\n");
    }

    #[test]
    fn ordered_signal_wait_returns_control_to_the_terminal() {
        let fixture = Fixture::start();
        let workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../demos/reference/approval/workflow.yaml");
        let output = command(&fixture.directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "approval-detached"])
            .output()
            .expect("approval workflow should enter its wait");
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            b"next \xc2\xb7 kairo signal approval-detached\n"
        );

        let inspection = command(&fixture.directory)
            .args(["inspect", "approval-detached"])
            .output()
            .expect("waiting run should be inspectable");
        assert!(
            String::from_utf8_lossy(&inspection.stdout)
                .contains("state · waiting for approval.granted")
        );
        assert!(
            command(&fixture.directory)
                .args(["signal", "approval-detached"])
                .output()
                .is_ok_and(|output| output.status.success())
        );
        wait_until(Duration::from_secs(5), || {
            command(&fixture.directory)
                .args(["inspect", "approval-detached"])
                .output()
                .is_ok_and(|output| {
                    String::from_utf8_lossy(&output.stdout).contains("state · completed")
                })
        });
    }

    #[test]
    fn resolves_a_single_waiting_run_by_workflow_name() {
        let fixture = Fixture::start();
        let workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../demos/reference/approval/workflow.yaml");
        let started = command(&fixture.directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "approval-by-name"])
            .output()
            .expect("approval workflow should enter its wait");
        assert!(started.status.success());

        let signal = command(&fixture.directory)
            .args(["signal", "approval"])
            .output()
            .expect("workflow shorthand should resolve");
        assert!(signal.status.success(), "signal failed: {signal:?}");
        wait_until(Duration::from_secs(5), || {
            command(&fixture.directory)
                .args(["inspect", "approval-by-name"])
                .output()
                .is_ok_and(|output| {
                    String::from_utf8_lossy(&output.stdout).contains("state · completed")
                })
        });
    }

    #[test]
    fn refuses_to_guess_between_waiting_runs() {
        let fixture = Fixture::start();
        let workflow = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../demos/reference/approval/workflow.yaml");
        for run in ["approval-one", "approval-two"] {
            let output = command(&fixture.directory)
                .arg("run")
                .arg(&workflow)
                .args(["--run", run])
                .output()
                .expect("approval workflow should enter its wait");
            assert!(output.status.success());
        }

        let signal = command(&fixture.directory)
            .args(["signal", "approval"])
            .output()
            .expect("ambiguous shorthand should fail");
        let error = String::from_utf8_lossy(&signal.stderr);
        assert!(!signal.status.success());
        assert!(error.contains("multiple `approval` runs are waiting"));
        assert!(error.contains("approval-one"));
        assert!(error.contains("approval-two"));
    }

    fn command(directory: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command.current_dir(directory);
        command
    }
    fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !ready() {
            assert!(Instant::now() < deadline, "operation timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

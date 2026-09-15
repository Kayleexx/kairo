#![allow(clippy::expect_used)]

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        path::Path,
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn starts_in_background_and_stops_cleanly() {
        let directory = std::env::temp_dir().join(format!("kairo-service-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("fixture directory should be created");

        let started = command(&directory)
            .args(["start", "--workers", "2"])
            .output()
            .expect("start should run");
        assert!(started.status.success());
        let started_stdout = String::from_utf8_lossy(&started.stdout);
        assert!(
            started_stdout.contains("kairo · ready") && started_stdout.contains("scale · 2"),
            "start/up must not succeed silently: {started_stdout}"
        );
        wait_until(Duration::from_secs(5), || {
            command(&directory)
                .arg("workers")
                .output()
                .is_ok_and(|output| {
                    output.status.success()
                        && String::from_utf8_lossy(&output.stdout).contains("workers · 2")
                })
        });

        let stopped = command(&directory)
            .arg("stop")
            .output()
            .expect("stop should run");
        assert!(stopped.status.success());
        let _ = fs::remove_dir_all(directory);
    }

    /// `--scale` is an alias for `--workers` on `up`. `status` is a distinct, user-oriented
    /// command from `workers` -- its default output must say nothing about worker ids, only
    /// revealing them under `--verbose`.
    #[test]
    fn up_accepts_scale_and_status_stays_user_oriented() {
        let directory =
            std::env::temp_dir().join(format!("kairo-service-scale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("fixture directory should be created");

        let started = command(&directory)
            .args(["up", "--scale", "2"])
            .output()
            .expect("up should run");
        assert!(started.status.success(), "{started:?}");
        wait_until(Duration::from_secs(5), || {
            command(&directory)
                .arg("status")
                .output()
                .is_ok_and(|output| {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    output.status.success()
                        && stdout.contains("kairo · ready")
                        && stdout.contains("scale · 2")
                })
        });
        let default_status = command(&directory)
            .arg("status")
            .output()
            .expect("status should run");
        let default_stdout = String::from_utf8_lossy(&default_status.stdout);
        assert!(
            !default_stdout.contains("worker-1") && !default_stdout.contains("worker-2"),
            "default status must not name workers: {default_stdout}"
        );

        let verbose_status = command(&directory)
            .args(["--verbose", "status"])
            .output()
            .expect("verbose status should run");
        let verbose_stdout = String::from_utf8_lossy(&verbose_status.stdout);
        assert!(
            verbose_stdout.contains("worker-1") && verbose_stdout.contains("worker-2"),
            "--verbose status should reveal the same detail `kairo workers` shows: {verbose_stdout}"
        );

        let stopped = command(&directory)
            .arg("down")
            .output()
            .expect("down should run");
        assert!(stopped.status.success());
        let _ = fs::remove_dir_all(directory);
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

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

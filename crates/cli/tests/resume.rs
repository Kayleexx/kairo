#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{self, Command, Stdio},
    };

    fn repository_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn kairo() -> Command {
        Command::new(env!("CARGO_BIN_EXE_kairo"))
    }

    /// `kairo resume <run>` should be a real, discoverable shorthand for what a user already
    /// does manually today: rerun `kairo run <workflow> --run <same-name>` against a durable
    /// cell interrupted mid-flight.
    #[test]
    fn resumes_an_interrupted_durable_run_by_name() {
        let directory = std::env::temp_dir().join(format!("kairo-resume-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("fixture directory should be created");

        let create = kairo()
            .current_dir(&directory)
            .args([
                "--allow-console",
                "workflow",
                "create",
                "--name",
                "resume-demo",
                "--component",
            ])
            .arg(repository_path("demos/basic/multiply-by-nine.wat"))
            .args(["--component"])
            .arg(repository_path(
                "crates/cli/tests/fixtures/slow-console.wat",
            ))
            .args(["--durability", "required", "--input", "20"])
            .output()
            .expect("workflow should be created");
        assert!(create.status.success(), "{create:?}");

        let mut child = kairo()
            .current_dir(&directory)
            .args(["--allow-console", "--verbose", "run", "resume-demo.yaml"])
            .args(["--run", "resume-demo"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kairo should start");
        wait_for_slow_start(child.stderr.take().expect("stderr should be piped"));
        child.kill().expect("kairo should be killed");
        assert!(!child.wait().expect("kairo should exit").success());

        let resumed = kairo()
            .current_dir(&directory)
            .args(["--allow-console", "resume", "resume-demo"])
            .output()
            .expect("kairo resume should run");
        assert!(
            resumed.status.success(),
            "resume failed: {}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert_eq!(resumed.stdout, b"180\n");

        let _ = fs::remove_dir_all(directory);
    }

    /// blocks until the slow component's own diagnostic line appears, or reports why the process
    /// exited early instead of hanging silently.
    fn wait_for_slow_start(stderr: impl std::io::Read) {
        let mut seen = Vec::new();
        for line in BufReader::new(stderr).lines() {
            let line = line.expect("diagnostic should be readable");
            if line.contains("cell component started") && line.contains("step=\"slow-console\"") {
                return;
            }
            seen.push(line);
        }
        panic!(
            "slow component never started before the process exited; full diagnostic output:\n{}",
            seen.join("\n")
        );
    }
}

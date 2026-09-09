#![allow(clippy::expect_used)]

#[cfg(unix)]
mod unix {
    use std::{fs, path::PathBuf, process::Command};

    #[test]
    fn runs_an_effect_workflow_without_manual_service_setup() {
        let directory =
            std::env::temp_dir().join(format!("kairo-auto-effect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).expect("fixture directory should be created");
        let workflow =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demos/effects/workflow.yaml");

        let output = command(&directory)
            .arg("run")
            .arg(workflow)
            .args(["--run", "automatic-effect"])
            .output()
            .expect("run should start");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"36\n");

        let inspection = command(&directory)
            .args(["inspect", "automatic-effect"])
            .output()
            .expect("inspect should start");
        assert!(inspection.status.success());
        assert!(String::from_utf8_lossy(&inspection.stdout).contains("record-order · committed"));

        let stopped = command(&directory)
            .arg("stop")
            .output()
            .expect("stop should start");
        assert!(stopped.status.success());
        let _ = fs::remove_dir_all(directory);
    }

    fn command(directory: &std::path::Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command.current_dir(directory);
        command
    }
}

#![allow(clippy::expect_used, clippy::unwrap_used)]

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn kairo() -> Command {
        Command::new(env!("CARGO_BIN_EXE_kairo"))
    }

    fn repository_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new(name: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("kairo-{name}-{}-{sequence}", std::process::id()));
            fs::create_dir(&path).expect("fixture directory should be created");
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            // best-effort: the bench command manages its own service lifecycle and stops it
            // itself, but a failed test run should never leave a local service behind.
            let _ = kairo().current_dir(&self.0).arg("down").output();
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn measures_real_recovery_after_a_real_worker_kill() {
        let directory = Directory::new("bench-chaos");
        let workflow = directory.0.join("slow.yaml");
        fs::write(
            &workflow,
            format!(
                "workflow: bench-chaos-slow\ninput: 1\nsteps:\n  - name: slow\n    component: {}\nedges: []\n",
                repository_path("crates/cli/tests/fixtures/slow-console.wat").display(),
            ),
        )
        .expect("workflow should be written");

        let output = kairo()
            .current_dir(&directory.0)
            .args(["bench", "run"])
            .arg(&workflow)
            .args(["--failure-scenario", "worker-kill", "--repetitions", "1"])
            .output()
            .expect("bench should run");
        assert!(output.status.success(), "{output:?}");

        let report_path = fs::read_dir(directory.0.join(".kairo/benchmarks"))
            .expect("benchmarks directory should exist")
            .next()
            .expect("a report should be written")
            .unwrap()
            .path();
        let report: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&report_path).unwrap())
                .expect("report should be valid JSON");

        assert_eq!(report["raw_samples"].as_array().unwrap().len(), 0);
        assert_eq!(report["failures"].as_array().unwrap().len(), 0);
        let recovery = report["recovery"].as_array().unwrap();
        assert_eq!(recovery.len(), 1);
        let cycle = &recovery[0];

        // the workflow does real, observably slow work specifically so the kill lands while it
        // is genuinely still running -- assert a real kill-and-recover happened, not just that
        // the run eventually finished (which could also happen if the kill raced and missed).
        assert_eq!(cycle["outcome"], "completed", "{cycle}");
        assert!(cycle["killed_worker"].as_str().is_some(), "{cycle}");
        assert!(cycle["recovery_wall_ms"].as_u64().unwrap() > 0, "{cycle}");
        assert_eq!(
            cycle["reassignment_reason"], "ReassignedAfterLeaseExpiry",
            "a reason of \"Initial\" would mean the kill raced and had no real effect: {cycle}"
        );
    }
}

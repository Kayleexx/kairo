#![allow(clippy::expect_used, clippy::unwrap_used)]

#[cfg(unix)]
mod unix {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
        thread,
        time::{Duration, Instant},
    };

    use kairo_control::{AssignmentReason, RunStatus};
    use kairo_core::{Config, Workflow};
    use kairo_runtime::{DurabilityProfile, Runtime, WorkflowProfile};

    fn minio_configured() -> bool {
        std::env::var_os("KAIRO_ARTIFACT_ENDPOINT").is_some()
            && std::env::var_os("KAIRO_ARTIFACT_BUCKET").is_some()
            && std::env::var_os("KAIRO_MINIO_ACCESS_KEY_ID").is_some()
            && std::env::var_os("KAIRO_MINIO_SECRET_ACCESS_KEY").is_some()
    }

    fn repository_path(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    fn minio_env(command: &mut Command) {
        for key in [
            "KAIRO_ARTIFACT_ENDPOINT",
            "KAIRO_ARTIFACT_BUCKET",
            "KAIRO_MINIO_ACCESS_KEY_ID",
            "KAIRO_MINIO_SECRET_ACCESS_KEY",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }

    fn kairo(directory: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kairo"));
        command.current_dir(directory);
        minio_env(&mut command);
        // isolate from any real global `~/.config/kairo/config.toml` on the host -- a persisted
        // storage choice there must never shadow this fixture's own env-var storage config.
        command.env("KAIRO_CONFIG", directory.join("global-config.toml"));
        command
    }

    struct Fixture {
        directory: PathBuf,
        workflow: PathBuf,
        service: Option<Child>,
    }

    impl Fixture {
        /// three steps chained by two `durability: required` boundaries, so a real run becomes
        /// three ExecutionGroups: [multiply], [divide], [add].
        fn multi_group(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "kairo-execution-groups-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&directory);
            fs::create_dir(&directory).expect("fixture directory should be created");
            let workflow = directory.join("workflow.yaml");
            fs::write(
                &workflow,
                format!(
                    "workflow: {name}\ninput: 100\nsteps:\n  - name: multiply\n    component: {}\n  - name: divide\n    component: {}\n  - name: add\n    component: {}\nedges:\n  - from: multiply\n    to: divide\n    durability: required\n  - from: divide\n    to: add\n    durability: required\n",
                    repository_path("demos/basic/multiply-by-nine.wat").display(),
                    repository_path("demos/basic/divide-by-five.wat").display(),
                    repository_path("demos/basic/add-thirty-two.wat").display(),
                ),
            )
            .expect("workflow should be written");
            seed_profile(&directory, &workflow, &["multiply", "divide"]);
            Self {
                directory,
                workflow,
                service: None,
            }
        }

        /// two steps: a fast required boundary, then a genuinely slow terminal step -- so a real
        /// worker kill lands mid-group-two, proving bounded, correct recovery.
        fn kill_target(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "kairo-execution-groups-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&directory);
            fs::create_dir(&directory).expect("fixture directory should be created");
            let workflow = directory.join("workflow.yaml");
            fs::write(
                &workflow,
                format!(
                    "workflow: {name}\ninput: 100\nsteps:\n  - name: multiply\n    component: {}\n  - name: slow\n    component: {}\nedges:\n  - from: multiply\n    to: slow\n    durability: required\n",
                    repository_path("demos/basic/multiply-by-nine.wat").display(),
                    repository_path("crates/cli/tests/fixtures/slow-console.wat").display(),
                ),
            )
            .expect("workflow should be written");
            seed_profile(&directory, &workflow, &["multiply"]);
            Self {
                directory,
                workflow,
                service: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = kairo(&self.directory).arg("down").output();
            if let Some(service) = &mut self.service {
                let _ = service.wait();
            }
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    /// writes a real `.kairo/profiles/<shape>.json` with a small, known `checkpoint_bytes` for
    /// each named boundary step, computed via the same `workflow_shape` the runtime itself uses
    /// -- so the worker's placement gate 2 sees a real, cheap-to-move edge instead of "unknown".
    fn seed_profile(directory: &Path, workflow: &Path, boundary_steps: &[&str]) {
        let loaded = Workflow::load(workflow, 1024 * 1024, 64).expect("workflow should load");
        let runtime = Runtime::new(Config {
            allow_console: true,
            ..Config::default()
        })
        .expect("runtime should initialize");
        let shape = runtime
            .workflow_shape(&loaded)
            .expect("shape should compute");
        let mut edges = HashMap::new();
        for step in boundary_steps {
            edges.insert(
                (*step).to_owned(),
                DurabilityProfile {
                    recompute_us: 1,
                    checkpoint_bytes: 19,
                    checkpoint_us: 1,
                    samples: 1,
                },
            );
        }
        let profile = WorkflowProfile {
            workflow: loaded.name().to_owned(),
            shape,
            edges,
        };
        let profiles_directory = directory.join(".kairo/profiles");
        fs::create_dir_all(&profiles_directory).expect("profiles directory should be created");
        fs::write(
            profiles_directory.join(format!("{}.json", profile.shape)),
            serde_json::to_vec(&profile).expect("profile should serialize"),
        )
        .expect("profile should be written");
    }

    impl Fixture {
        /// `kairo start` never forwards `--allow-console` to its spawned workers (by design, per
        /// `lifecycle::start_with_console`'s own doc comment) -- `serve` is the same foreground
        /// service loop `start --foreground` delegates to, and does honor it, matching how
        /// `bench run --failure-scenario worker-kill` keeps a fixture observably busy.
        fn start_workers(&mut self, count: usize) {
            let mut command = kairo(&self.directory);
            command
                .args(["--allow-console", "serve", "--workers", &count.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            self.service = Some(command.spawn().expect("control service should start"));
            let directory = self.directory.clone();
            wait_until(Duration::from_secs(5), || {
                kairo(&directory)
                    .arg("workers")
                    .output()
                    .is_ok_and(|output| {
                        String::from_utf8_lossy(&output.stdout)
                            .contains(&format!("workers · {count}"))
                    })
            });
        }
    }

    fn endpoint(directory: &Path) -> kairo_control::Endpoint {
        kairo_control::load_endpoint(&directory.join(".kairo")).expect("endpoint should load")
    }

    fn history(directory: &Path, id: &str) -> Vec<kairo_control::RunEvent> {
        kairo_control::snapshot(&endpoint(directory))
            .expect("snapshot should load")
            .runs
            .into_iter()
            .find(|run| run.id == id)
            .map(|run| run.history)
            .unwrap_or_default()
    }

    fn running_worker(directory: &Path, id: &str) -> Option<String> {
        kairo_control::snapshot(&endpoint(directory))
            .expect("snapshot should load")
            .runs
            .into_iter()
            .find(|run| run.id == id)
            .and_then(|run| match run.status {
                RunStatus::Running { worker, .. } => Some(worker),
                _ => None,
            })
    }

    fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + timeout;
        while !ready() {
            assert!(Instant::now() < deadline, "operation timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_group_boundary_with_a_light_known_edge_moves_to_a_different_worker() {
        if !minio_configured() {
            eprintln!(
                "skipping: MinIO is not configured (KAIRO_ARTIFACT_ENDPOINT/BUCKET/MINIO credentials)"
            );
            return;
        }
        let mut fixture = Fixture::multi_group("multi-group-e2e");
        fixture.start_workers(2);

        let output = kairo(&fixture.directory)
            .arg("run")
            .arg(&fixture.workflow)
            .args(["--run", "multi-group-e2e", "--watch"])
            .output()
            .expect("run should complete");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"212\n", "100 * 9 / 5 + 32");

        let history = history(&fixture.directory, "multi-group-e2e");
        let yields: Vec<_> = history
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    kairo_control::RunEvent::Queued {
                        reason: AssignmentReason::ReassignedAfterGroupYield { .. },
                        ..
                    }
                )
            })
            .collect();
        assert!(
            yields.len() >= 2,
            "expected at least two real ExecutionGroup yields (one per required boundary), got: {history:?}"
        );

        let assigned_workers: Vec<&str> = history
            .iter()
            .filter_map(|event| match event {
                kairo_control::RunEvent::Assigned { worker, .. } => Some(worker.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            assigned_workers.len() >= 3,
            "expected one real assignment per group (three groups), got: {assigned_workers:?}"
        );
        assert!(
            assigned_workers.windows(2).any(|pair| pair[0] != pair[1]),
            "expected at least one real cross-worker handoff between consecutive groups, got: {assigned_workers:?}"
        );

        let inspection = kairo(&fixture.directory)
            .args(["inspect", "multi-group-e2e"])
            .output()
            .expect("inspect should run");
        assert!(inspection.status.success(), "{inspection:?}");
        let rendered = String::from_utf8_lossy(&inspection.stdout);
        assert!(
            rendered.contains("placement"),
            "kairo inspect should surface real per-group worker placement: {rendered}"
        );
        assert!(
            assigned_workers
                .iter()
                .all(|worker| rendered.contains(worker)),
            "every real assigned worker should be named in the placement section: {rendered}"
        );
        assert!(
            rendered.contains("group 0") && rendered.contains("group 1"),
            "placement lines should be labeled by real ExecutionGroup index: {rendered}"
        );
        assert!(
            !rendered.contains("execution group moved"),
            "placement must show the real worker transition, not the old unconditional \
             'execution group moved' label: {rendered}"
        );
        assert!(
            rendered.contains(" \u{2192} ") && rendered.contains("after durable boundary"),
            "a real cross-worker handoff should read `from -> to * after durable boundary`: {rendered}"
        );
    }

    #[test]
    fn a_worker_kill_mid_group_recovers_from_the_reported_boundary() {
        if !minio_configured() {
            eprintln!(
                "skipping: MinIO is not configured (KAIRO_ARTIFACT_ENDPOINT/BUCKET/MINIO credentials)"
            );
            return;
        }
        let mut fixture = Fixture::kill_target("kill-mid-group-e2e");
        fixture.start_workers(2);

        let mut run = kairo(&fixture.directory)
            .arg("run")
            .arg(&fixture.workflow)
            .args(["--run", "kill-mid-group-e2e", "--watch"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("run should start");

        // wait for a real ExecutionGroup yield (group one, "multiply", finished and committed)
        // before the kill, so it lands genuinely inside group two ("slow"), not before group one
        // even started.
        wait_until(Duration::from_secs(10), || {
            history(&fixture.directory, "kill-mid-group-e2e")
                .iter()
                .any(|event| {
                    matches!(
                        event,
                        kairo_control::RunEvent::Queued {
                            reason: AssignmentReason::ReassignedAfterGroupYield { .. },
                            ..
                        }
                    )
                })
        });
        let worker = wait_for_running_worker(&fixture.directory, "kill-mid-group-e2e");
        kairo_control::kill_worker(&endpoint(&fixture.directory), worker)
            .expect("chaos kill should be accepted");

        wait_until(Duration::from_secs(10), || {
            run.try_wait().is_ok_and(|status| status.is_some())
        });
        let output = run.wait_with_output().expect("run should finish");
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"900\n", "100 * 9, recovered after the kill");

        let history = history(&fixture.directory, "kill-mid-group-e2e");
        assert!(
            history.iter().any(|event| matches!(
                event,
                kairo_control::RunEvent::Queued {
                    reason: AssignmentReason::ReassignedAfterLeaseExpiry,
                    ..
                }
            )),
            "a real worker kill must produce a real lease-expiry reassignment, not just an eventual finish: {history:?}"
        );
    }

    fn wait_for_running_worker(directory: &Path, id: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(worker) = running_worker(directory, id) {
                return worker;
            }
            assert!(
                Instant::now() < deadline,
                "no worker ever picked up group two"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

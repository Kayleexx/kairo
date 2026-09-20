#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use rusqlite::Connection;

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
        let path = std::env::temp_dir().join(format!("kairo-{name}-{}-{sequence}", process::id()));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn component_started_count(state: &std::path::Path, index: i64) -> i64 {
    event_count(state, "component_started", index).expect("event count should load")
}

fn event_count(state: &std::path::Path, kind: &str, index: i64) -> Option<i64> {
    Connection::open(state)
        .ok()?
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = ?1 AND step_index = ?2",
            (kind, index),
            |row| row.get(0),
        )
        .ok()
}

/// polls the real journal file on disk until `condition` sees the durable state it's waiting for,
/// rather than sleeping a guessed duration -- the wait is bounded by real observed state, not time.
fn wait_for(
    state: &std::path::Path,
    timeout: Duration,
    condition: impl Fn(&std::path::Path) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while !condition(state) {
        assert!(
            Instant::now() < deadline,
            "condition did not become true within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn resumes_across_a_real_process_restart_with_local_journal_and_blob_intact() {
    let directory = Directory::new("value-restart");
    let workflow = directory.0.join("echo-flow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: echo-flow\nmode: value\nsteps:\n  - name: first\n    component: {}\n  - name: second\n    component: {}\nedges:\n  - from: first\n    to: second\n    durability: ephemeral\n",
            repository_path("components/runtime/value-echo/component.wasm").display(),
            repository_path("components/runtime/value-echo/component.wasm").display(),
        ),
    )
    .expect("workflow should write");
    let input = directory.0.join("input.txt");
    fs::write(&input, "hi").expect("input file should write");
    let state = directory.0.join("state.db");

    let run = |directory: &Directory, workflow: &PathBuf, input: &PathBuf, state: &PathBuf| {
        kairo()
            .current_dir(&directory.0)
            .arg("run")
            .arg(workflow)
            .arg("--input-file")
            .arg(input)
            .arg("--state")
            .arg(state)
            .output()
            .expect("kairo run should run")
    };

    let first = run(&directory, &workflow, &input, &state);
    assert!(first.status.success(), "{first:?}");
    let first_output = String::from_utf8_lossy(&first.stdout).trim().to_owned();
    assert_eq!(first_output, "jk");
    assert_eq!(component_started_count(&state, 0), 1);

    // a brand new OS process, pointed at the same journal/blob state the first process wrote --
    // this is a genuine process restart, not same-process Runtime reuse.
    let second = run(&directory, &workflow, &input, &state);
    assert!(second.status.success(), "{second:?}");
    let second_output = String::from_utf8_lossy(&second.stdout).trim().to_owned();
    assert_eq!(second_output, first_output);

    // the durable-to-restart first step must never re-execute across the restart.
    assert_eq!(component_started_count(&state, 0), 1);
}

#[test]
fn recovers_from_the_last_durable_boundary_after_a_real_worker_kill() {
    let directory = Directory::new("value-worker-kill");
    let workflow = directory.0.join("checkpoint-flow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: checkpoint-flow\nmode: value\nresources:\n  fuel: 4000000000\n  memory_bytes: 67108864\nsteps:\n  - name: echo\n    component: {}\n  - name: slow\n    component: {}\nedges:\n  - from: echo\n    to: slow\n    durability: required\n",
            repository_path("components/runtime/value-echo/component.wasm").display(),
            repository_path("components/runtime/value-slow/component.wasm").display(),
        ),
    )
    .expect("workflow should write");
    let input = directory.0.join("input.txt");
    fs::write(&input, "hi").expect("input file should write");
    let state = directory.0.join("state.db");

    let mut first = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(&workflow)
        .arg("--input-file")
        .arg(&input)
        .arg("--state")
        .arg(&state)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("kairo run should spawn");

    // wait for the real durable checkpoint after "echo" -- proof the required edge actually
    // persisted before the kill, not a guess about timing.
    wait_for(&state, Duration::from_secs(10), |state| {
        event_count(state, "checkpoint_created", 0) == Some(1)
    });

    // a real SIGKILL (std::process::Child::kill sends SIGKILL on unix) mid-execution of the slow
    // second step -- simulates losing the worker process entirely, not a deterministic failure.
    first.kill().expect("process should be killable");
    let status = first.wait().expect("killed process should be waited on");
    assert!(!status.success());

    let second = kairo()
        .current_dir(&directory.0)
        .arg("run")
        .arg(&workflow)
        .arg("--input-file")
        .arg(&input)
        .arg("--state")
        .arg(&state)
        .output()
        .expect("kairo run should run");
    assert!(second.status.success(), "{second:?}");
    assert_eq!(String::from_utf8_lossy(&second.stdout).trim(), "jk");

    // the durably checkpointed "echo" step must never re-execute after recovering from the kill.
    // the second process has already exited successfully, but give its sqlite writes a bounded
    // moment to land before reading them back from a fresh connection, instead of racing them.
    wait_for(&state, Duration::from_secs(5), |state| {
        Connection::open(state)
            .ok()
            .and_then(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM events WHERE kind = 'component_started' AND step_index = 0",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .ok()
            })
            .is_some_and(|count| count >= 1)
    });
    assert_eq!(component_started_count(&state, 0), 1);
}

/// `--watch` used to unconditionally reject any non-scalar workflow; it must now work for
/// `mode: value` too, driven by the same observable journal `kairo inspect` already reads.
#[test]
fn watch_runs_a_value_mode_workflow_to_completion() {
    let directory = Directory::new("value-watch");
    let workflow = directory.0.join("watch-flow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: watch-flow\nmode: value\nresources:\n  fuel: 4000000000\n  memory_bytes: 67108864\nsteps:\n  - name: echo\n    component: {}\n  - name: slow\n    component: {}\nedges:\n  - from: echo\n    to: slow\n    durability: required\n",
            repository_path("components/runtime/value-echo/component.wasm").display(),
            repository_path("components/runtime/value-slow/component.wasm").display(),
        ),
    )
    .expect("workflow should write");

    let run = kairo()
        .current_dir(&directory.0)
        .args(["run", "watch-flow.yaml", "--value", "hi", "--watch"])
        .output()
        .expect("kairo run --watch should run");
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "jk");

    let inspect = kairo()
        .current_dir(&directory.0)
        .arg("inspect")
        .output()
        .expect("kairo inspect should run");
    assert!(inspect.status.success(), "{inspect:?}");
    let stdout = String::from_utf8_lossy(&inspect.stdout);
    assert!(stdout.contains("state · completed"), "{stdout}");
    assert!(stdout.contains("checkpoint ·"), "{stdout}");
}

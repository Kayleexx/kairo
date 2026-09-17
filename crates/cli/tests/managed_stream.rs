#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use kairo_control::{AssignmentReason, LiveEdgeState, RunEvent, RunOutcome, RunOutput, RunStatus};

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
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kairo-managed-stream-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = kairo().current_dir(&self.0).arg("down").output();
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
#[test]
fn producer_loss_restarts_from_the_prior_durable_boundary() {
    exercise_live_edge_terminal(LiveAction::ProducerLoss);
}

#[cfg(unix)]
#[test]
fn consumer_loss_restarts_from_the_prior_durable_boundary() {
    exercise_live_edge_terminal(LiveAction::ConsumerLoss);
}

#[cfg(unix)]
#[test]
fn active_cancellation_stops_both_live_participants() {
    exercise_live_edge_terminal(LiveAction::Cancel);
}

#[cfg(unix)]
#[test]
fn control_restart_orphans_live_session_and_recovers_from_boundary() {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, vec![7_u8; 16 * 1024 * 1024]).expect("input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    fs::write(directory.0.join("workflow.yaml"), format!("workflow: restart-live\nmode: stream\nresources:\n  fuel: 500000000\n  memory_bytes: 33554432\ninput: {}\nsteps:\n  - name: checkpoint\n    component: {}\n  - name: relay\n    component: {}\n  - name: consume\n    component: {}\nedges:\n  - from: checkpoint\n    to: relay\n    durability: required\n  - from: relay\n    to: consume\n", input.display(), transform.display(), transform.display(), consume.display())).expect("workflow");
    let mut service = start_service(&directory.0);
    wait_for_workers(&directory.0);
    let mut original = start_run(&directory.0, "restart-live");
    let (old_endpoint, session) =
        wait_for_live_session(&directory.0, "restart-live", &mut original);
    service.kill().expect("kill control");
    service.wait().expect("wait control");
    let mut restarted = start_service(&directory.0);
    wait_for_workers(&directory.0);
    let endpoint = kairo_control::load_endpoint(&directory.0.join(".kairo")).expect("new endpoint");
    let snapshot = kairo_control::snapshot(&endpoint).expect("orphan snapshot");
    assert!(snapshot.live_edges.iter().any(|edge| edge.id == session.id && matches!(edge.state, LiveEdgeState::Failed { .. })), "{snapshot:?}");
    assert_ne!(endpoint.address, old_endpoint.address);
    assert!(
        kairo_control::complete_live_edge(
            &endpoint,
            &session.producer_worker,
            &kairo_control::Assignment {
                run: kairo_control::RunRequest {
                    id: "restart-live".into(),
                    workflow: "workflow.yaml".into(),
                    state: directory.0.join(".kairo/restart-live.db"),
                    storage: None,
                    wait: None,
                    plan: None,
                    resume: None,
                    preferred_worker: None,
                    preferred_deadline_ms: None,
                    shape: None,
                    stream_input: None
                },
                epoch: session.parent_epoch,
                live_edge: Some(kairo_control::LiveEdgeAssignment {
                    session_id: session.id.clone(),
                    edge_id: session.edge_id.clone(),
                    parent_epoch: session.parent_epoch,
                    producer_endpoint: String::new(),
                    group: session.producer_group
                })
            },
            kairo_control::LiveEdgeParticipant::Producer
        )
        .is_err()
    );
    let _ = original.wait_with_output();
    let final_snapshot = wait_for_completion(&endpoint, "restart-live");
    let run = final_snapshot
        .runs
        .iter()
        .find(|run| run.id == "restart-live")
        .expect("recovered run");
    assert!(matches!(
        run.status,
        RunStatus::Completed {
            output: RunOutput::Stream(ref output),
            ..
        } if output.bytes == 16 * 1024 * 1024 && output.checksum == 117_440_512
    ));
    assert!(run.history.iter().any(|event| matches!(
        event,
        RunEvent::Queued {
            reason: AssignmentReason::ResumedAfterRestart,
            ..
        }
    )));
    assert_eq!(
        run.history
            .iter()
            .filter(|event| matches!(
                event,
                RunEvent::Outcome {
                    outcome: RunOutcome::Completed,
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(final_snapshot.live_edges.iter().any(|edge| {
        edge.run_id == "restart-live"
            && edge.parent_epoch > session.parent_epoch
            && edge.producer_group == session.producer_group
            && matches!(edge.state, LiveEdgeState::Completed)
    }));
    let _ = kairo().current_dir(&directory.0).arg("down").output();
    let _ = restarted.wait();
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum LiveAction {
    ProducerLoss,
    ConsumerLoss,
    Cancel,
}

#[cfg(unix)]
fn exercise_live_edge_terminal(action: LiveAction) {
    let directory = Directory::new();
    let input = directory.0.join("input.bin");
    fs::write(&input, vec![7_u8; 16 * 1024 * 1024]).expect("large input");
    let transform = repository_path("demos/stream/transform.wat");
    let consume = repository_path("demos/stream/consume.wat");
    let workflow = directory.0.join("workflow.yaml");
    fs::write(
        &workflow,
        format!(
            "workflow: durable-live-recovery\nmode: stream\nresources:\n  fuel: 500000000\n  memory_bytes: 33554432\ninput: {input}\nsteps:\n  - name: checkpoint\n    component: {transform}\n  - name: relay\n    component: {transform}\n  - name: consume\n    component: {consume}\nedges:\n  - from: checkpoint\n    to: relay\n    durability: required\n  - from: relay\n    to: consume\n",
            input = input.display(),
            transform = transform.display(),
            consume = consume.display(),
        ),
    )
    .expect("workflow should be written");
    let mut service = kairo()
        .current_dir(&directory.0)
        .args(["serve", "--workers", "2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("service should start");
    wait_for_workers(&directory.0);
    let mut run = kairo()
        .current_dir(&directory.0)
        .args([
            "run",
            "workflow.yaml",
            "--watch",
            "--run",
            "durable-live-recovery",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("managed stream should start");
    let (endpoint, session) =
        wait_for_live_session(&directory.0, "durable-live-recovery", &mut run);
    match action {
        LiveAction::ProducerLoss => kairo_control::kill_worker(&endpoint, session.producer_worker),
        LiveAction::ConsumerLoss => kairo_control::kill_worker(&endpoint, session.consumer_worker),
        LiveAction::Cancel => kairo_control::cancel(&endpoint, "durable-live-recovery".to_owned()),
    }
    .expect("live action should be accepted");
    let output = run.wait_with_output().expect("run should exit");
    let final_snapshot = if matches!(action, LiveAction::Cancel) {
        wait_for_cancellation(&endpoint, "durable-live-recovery")
    } else {
        kairo_control::snapshot(&endpoint).expect("final snapshot")
    };
    assert_eq!(
        output.status.success(),
        !matches!(action, LiveAction::Cancel),
        "{output:?}\n{final_snapshot:?}"
    );
    let snapshot = final_snapshot;
    let run = snapshot
        .runs
        .into_iter()
        .find(|run| run.id == "durable-live-recovery")
        .expect("run status");
    if matches!(action, LiveAction::Cancel) {
        assert!(matches!(run.status, RunStatus::Canceled), "{run:?}");
        assert!(
            snapshot
                .live_edges
                .iter()
                .any(|session| matches!(session.state, LiveEdgeState::Cancelled))
        );
    } else {
        assert!(matches!(run.status, RunStatus::Completed { .. }), "{run:?}");
    }
    assert!(
        matches!(action, LiveAction::Cancel)
            || snapshot
                .live_edges
                .iter()
                .any(|session| matches!(session.state, LiveEdgeState::Failed { .. })),
        "the interrupted live session must not be considered durable progress: {:?}",
        snapshot.live_edges
    );
    let _ = kairo().current_dir(&directory.0).arg("down").output();
    let _ = service.wait();
}

#[cfg(unix)]
fn wait_for_live_session(
    directory: &std::path::Path,
    id: &str,
    run: &mut std::process::Child,
) -> (kairo_control::Endpoint, kairo_control::LiveEdgeSession) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(endpoint) = kairo_control::load_endpoint(&directory.join(".kairo"))
            && let Ok(snapshot) = kairo_control::snapshot(&endpoint)
            && let Some(session) = snapshot.live_edges.into_iter().find(|session| {
                session.run_id == id && matches!(session.state, LiveEdgeState::Streaming)
            })
        {
            return (endpoint, session);
        }
        assert!(
            run.try_wait().expect("run status").is_none(),
            "run exited before it submitted the live session"
        );
        assert!(
            Instant::now() < deadline,
            "live session did not become active: {:?}",
            kairo_control::load_endpoint(&directory.join(".kairo"))
                .ok()
                .and_then(|endpoint| kairo_control::snapshot(&endpoint).ok())
        );
        std::thread::yield_now();
    }
}

#[cfg(unix)]
fn wait_for_workers(directory: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(endpoint) = kairo_control::load_endpoint(&directory.join(".kairo"))
            && kairo_control::snapshot(&endpoint).is_ok_and(|snapshot| {
                snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.healthy)
                    .count()
                    == 2
            })
        {
            return;
        }
        assert!(Instant::now() < deadline, "workers did not register");
        std::thread::yield_now();
    }
}

#[cfg(unix)]
fn start_service(directory: &std::path::Path) -> std::process::Child {
    kairo()
        .current_dir(directory)
        .args(["serve", "--workers", "2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("service")
}

#[cfg(unix)]
fn start_run(directory: &std::path::Path, id: &str) -> std::process::Child {
    kairo()
        .current_dir(directory)
        .args(["run", "workflow.yaml", "--watch", "--run", id])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run")
}

#[cfg(unix)]
fn wait_for_completion(endpoint: &kairo_control::Endpoint, id: &str) -> kairo_control::Snapshot {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = kairo_control::snapshot(endpoint).expect("snapshot");
        if matches!(
            snapshot
                .runs
                .iter()
                .find(|run| run.id == id)
                .map(|run| &run.status),
            Some(RunStatus::Completed { .. })
        ) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "recovery did not complete: {snapshot:?}"
        );
        std::thread::yield_now();
    }
}

#[cfg(unix)]
fn wait_for_cancellation(endpoint: &kairo_control::Endpoint, id: &str) -> kairo_control::Snapshot {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = kairo_control::snapshot(endpoint).expect("snapshot");
        if matches!(
            snapshot
                .runs
                .iter()
                .find(|run| run.id == id)
                .map(|run| &run.status),
            Some(RunStatus::Canceled)
        ) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "cancellation did not finish: {snapshot:?}"
        );
        std::thread::yield_now();
    }
}

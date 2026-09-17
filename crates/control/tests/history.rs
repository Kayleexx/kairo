#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use kairo_control::{
    AssignmentReason, Endpoint, RunEvent, RunOutcome, RunRequest, Server, cancel, snapshot, submit,
};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-control-history-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory should be created");
    path
}

fn history_of(endpoint: &Endpoint, id: &str) -> Vec<RunEvent> {
    snapshot(endpoint)
        .expect("snapshot should load")
        .runs
        .into_iter()
        .find(|run| run.id == id)
        .expect("run should be present in snapshot")
        .history
}

#[test]
fn queue_wait_and_assignment_reason_are_recorded_for_a_normal_run() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve_until(&stop));

    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-1","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    submit(&endpoint, run("run-1", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );

    let history = history_of(&endpoint, "run-1");
    let queued_at = history.iter().find_map(|event| match event {
        RunEvent::Queued {
            at_ms,
            reason: AssignmentReason::Initial,
        } => Some(*at_ms),
        _ => None,
    });
    let assigned = history.iter().find_map(|event| match event {
        RunEvent::Assigned {
            worker,
            epoch,
            at_ms,
            reason,
        } => Some((worker.clone(), *epoch, *at_ms, *reason)),
        _ => None,
    });
    let queued_at = queued_at.expect("a Queued(Initial) event should be recorded");
    let (worker, epoch, assigned_at, reason) =
        assigned.expect("an Assigned event should be recorded");
    assert_eq!(worker, "worker-1");
    assert_eq!(epoch, 1);
    assert!(matches!(reason, AssignmentReason::Initial));
    // real timestamps, not invented: assignment cannot precede the queuing that led to it.
    assert!(assigned_at >= queued_at);

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn reassignment_after_lease_expiry_is_recorded_with_the_correct_reason() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve_until(&stop));

    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-1","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    submit(&endpoint, run("run-1", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );

    // past the fixed 3s lease timeout with no heartbeat, `worker-1` is dead: a snapshot request
    // (which sweeps expired leases) requeues the run for a real reassignment below.
    thread::sleep(std::time::Duration::from_millis(3200));
    snapshot(&endpoint).expect("snapshot should load");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-2","pid":2,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-2","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );

    let history = history_of(&endpoint, "run-1");
    let reassigned = history.iter().any(|event| {
        matches!(
            event,
            RunEvent::Assigned {
                worker,
                reason: AssignmentReason::ReassignedAfterLeaseExpiry,
                ..
            } if worker == "worker-2"
        )
    });
    assert!(
        reassigned,
        "expected a ReassignedAfterLeaseExpiry assignment to worker-2, got {history:?}"
    );

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn outcome_events_are_recorded_for_completed_failed_and_canceled_runs() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve_until(&stop));

    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-1","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );

    submit(&endpoint, run("completed", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("completed")
    );
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"completed","epoch":1,"output":42}}}}"#, endpoint.token)).contains("Ok"));
    assert!(has_outcome(
        &history_of(&endpoint, "completed"),
        RunOutcome::Completed
    ));

    submit(&endpoint, run("failed", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("failed")
    );
    assert!(request(&endpoint, format!(r#"{{"Fail":{{"worker":"worker-1","token":"{}","id":"failed","epoch":1,"message":"boom"}}}}"#, endpoint.token)).contains("Ok"));
    assert!(has_outcome(
        &history_of(&endpoint, "failed"),
        RunOutcome::Failed
    ));

    submit(&endpoint, run("canceled", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("canceled")
    );
    cancel(&endpoint, "canceled".to_owned()).expect("cancel should succeed");
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"canceled","epoch":1,"output":0}}}}"#, endpoint.token)).contains("Canceled"));
    assert!(has_outcome(
        &history_of(&endpoint, "canceled"),
        RunOutcome::Canceled
    ));

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

fn has_outcome(history: &[RunEvent], expected: RunOutcome) -> bool {
    history.iter().any(|event| {
        matches!(event, RunEvent::Outcome { outcome, .. } if std::mem::discriminant(outcome) == std::mem::discriminant(&expected))
    })
}

#[test]
fn history_persists_across_restart() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve_until(&stop));

    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-1","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    submit(&endpoint, run("run-1", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"run-1","epoch":1,"output":42}}}}"#, endpoint.token)).contains("Ok"));
    let before = history_of(&endpoint, "run-1");
    assert!(!before.is_empty());

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    drop(server);

    let restarted = Arc::new(Server::start(&directory).expect("server should restart"));
    let restarted_endpoint = restarted.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&restarted);
    let handle = thread::spawn(move || running.serve_until(&stop));

    let after = history_of(&restarted_endpoint, "run-1");
    assert_eq!(before.len(), after.len());
    assert!(after.iter().any(|event| matches!(
        event,
        RunEvent::Outcome {
            outcome: RunOutcome::Completed,
            ..
        }
    )));

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

fn run(id: &str, wait: Option<kairo_control::WaitRequest>) -> RunRequest {
    RunRequest {
        id: id.to_owned(),
        workflow: PathBuf::from("workflow.yaml"),
        state: PathBuf::from("run.db"),
        storage: None,
        wait,
        plan: None,
        resume: None,
        preferred_worker: None,
        preferred_deadline_ms: None,
        shape: None,
        stream_input: None,
    }
}

fn request(endpoint: &Endpoint, value: String) -> String {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let mut stream = TcpStream::connect(endpoint.address).expect("server should accept requests");
    stream
        .write_all(value.as_bytes())
        .expect("request should write");
    stream.write_all(b"\n").expect("newline should write");
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .expect("response should read");
    response
}

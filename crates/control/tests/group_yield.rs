#![allow(clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use kairo_control::{
    AssignmentReason, Endpoint, RunEvent, RunPlan, RunRequest, Server, snapshot, submit,
    yield_group,
};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-control-group-yield-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory should be created");
    path
}

fn run(id: &str) -> RunRequest {
    RunRequest {
        id: id.to_owned(),
        workflow: PathBuf::from("workflow.yaml"),
        state: PathBuf::from("run.db"),
        storage: None,
        wait: None,
        plan: None,
        resume: None,
        preferred_worker: None,
        preferred_deadline_ms: None,
        shape: None,
        stream_input: None,
    }
}

fn register(endpoint: &Endpoint, worker: &str) {
    assert!(
        request(
            endpoint,
            format!(
                r#"{{"Register":{{"worker":"{worker}","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
}

fn next(endpoint: &Endpoint, worker: &str) -> serde_json::Value {
    let response = request(
        endpoint,
        format!(
            r#"{{"Next":{{"worker":"{worker}","token":"{}"}}}}"#,
            endpoint.token
        ),
    );
    serde_json::from_str(&response).expect("response should be valid JSON")
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

fn start(directory: &std::path::Path) -> (Arc<kairo_control::Server>, Endpoint, Arc<AtomicBool>) {
    let server = Arc::new(Server::start(directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    thread::spawn(move || running.serve_until(&stop));
    (server, endpoint, stopped)
}

fn stop(server: Arc<Server>, stopped: Arc<AtomicBool>) {
    stopped.store(true, Ordering::Relaxed);
    drop(server);
}

#[test]
fn a_correctly_fenced_yield_requeues_the_run_with_the_resume_and_plan_fields() {
    let directory = directory();
    let (server, endpoint, stopped) = start(&directory);
    register(&endpoint, "worker-1");
    submit(&endpoint, run("run-1")).expect("run should submit");
    let assignment = next(&endpoint, "worker-1");
    assert!(assignment.to_string().contains("run-1"));

    let plan = RunPlan {
        resolved_durability: BTreeMap::from([(0, true)]),
    };
    yield_group(
        &endpoint,
        "worker-1",
        "run-1".to_owned(),
        1,
        1,
        "sha256:abc".to_owned(),
        "memory".to_owned(),
        Some(plan.clone()),
        None,
        None,
        false,
    )
    .expect("a correctly fenced yield should be accepted");

    let assignment = next(&endpoint, "worker-1");
    let run = &assignment["Assignment"]["run"]["run"];
    assert_eq!(run["id"], "run-1");
    assert_eq!(run["resume"]["from_index"], 1);
    assert_eq!(run["resume"]["artifact_hash"], "sha256:abc");
    assert_eq!(run["resume"]["artifact_backend"], "memory");
    assert_eq!(run["plan"]["resolved_durability"]["0"], true);

    let history = history_of(&endpoint, "run-1");
    assert!(history.iter().any(|event| matches!(
        event,
        RunEvent::Queued {
            reason: AssignmentReason::ReassignedAfterGroupYield {
                target_had_cache: false
            },
            ..
        }
    )));

    stop(server, stopped);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_yield_with_a_stale_epoch_is_rejected() {
    let directory = directory();
    let (server, endpoint, stopped) = start(&directory);
    register(&endpoint, "worker-1");
    submit(&endpoint, run("run-1")).expect("run should submit");
    next(&endpoint, "worker-1");

    let error = yield_group(
        &endpoint,
        "worker-1",
        "run-1".to_owned(),
        99,
        1,
        "sha256:abc".to_owned(),
        "memory".to_owned(),
        None,
        None,
        None,
        false,
    )
    .expect_err("a stale epoch must never be accepted");
    assert!(matches!(
        error,
        kairo_control::ControlError::Rejected { .. }
    ));

    stop(server, stopped);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_preferred_worker_gets_the_continuation_before_the_deadline() {
    let directory = directory();
    let (server, endpoint, stopped) = start(&directory);
    register(&endpoint, "worker-1");
    register(&endpoint, "worker-2");
    submit(&endpoint, run("run-1")).expect("run should submit");
    next(&endpoint, "worker-1");

    yield_group(
        &endpoint,
        "worker-1",
        "run-1".to_owned(),
        1,
        1,
        "sha256:abc".to_owned(),
        "memory".to_owned(),
        None,
        None,
        Some("worker-2".to_owned()),
        true,
    )
    .expect("yield should be accepted");

    // worker-1 asking again must not get the continuation reserved for worker-2.
    let declined = next(&endpoint, "worker-1");
    assert_eq!(declined["Assignment"]["run"], serde_json::Value::Null);

    // worker-2 gets it immediately, even though it didn't ask first.
    let assignment = next(&endpoint, "worker-2");
    let run = &assignment["Assignment"]["run"]["run"];
    assert_eq!(run["id"], "run-1");

    let history = history_of(&endpoint, "run-1");
    assert!(history.iter().any(|event| matches!(
        event,
        RunEvent::Assigned { worker, .. } if worker == "worker-2"
    )));

    stop(server, stopped);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn an_unclaimed_reservation_falls_back_to_any_capable_worker_after_the_deadline() {
    let directory = directory();
    let (server, endpoint, stopped) = start(&directory);
    register(&endpoint, "worker-1");
    submit(&endpoint, run("run-1")).expect("run should submit");
    next(&endpoint, "worker-1");

    yield_group(
        &endpoint,
        "worker-1",
        "run-1".to_owned(),
        1,
        1,
        "sha256:abc".to_owned(),
        "memory".to_owned(),
        None,
        None,
        Some("worker-2".to_owned()),
        false,
    )
    .expect("yield should be accepted");

    // "worker-2" never registers or asks -- past the reservation window, worker-1 must still
    // be able to take it, so a placement choice can never strand a run forever.
    thread::sleep(Duration::from_millis(2200));
    let assignment = next(&endpoint, "worker-1");
    let run = &assignment["Assignment"]["run"]["run"];
    assert_eq!(run["id"], "run-1");

    stop(server, stopped);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn a_run_plan_survives_a_control_plane_restart() {
    let directory = directory();
    let (server, endpoint, stopped) = start(&directory);
    register(&endpoint, "worker-1");
    submit(&endpoint, run("run-1")).expect("run should submit");
    next(&endpoint, "worker-1");

    let plan = RunPlan {
        resolved_durability: BTreeMap::from([(0, true), (2, false)]),
    };
    yield_group(
        &endpoint,
        "worker-1",
        "run-1".to_owned(),
        1,
        1,
        "sha256:abc".to_owned(),
        "memory".to_owned(),
        Some(plan),
        None,
        None,
        false,
    )
    .expect("yield should be accepted");

    stop(server, stopped);

    let (restarted, restarted_endpoint, restarted_stopped) = start(&directory);
    register(&restarted_endpoint, "worker-1");
    let assignment = next(&restarted_endpoint, "worker-1");
    let run = &assignment["Assignment"]["run"]["run"];
    assert_eq!(run["id"], "run-1");
    assert_eq!(run["plan"]["resolved_durability"]["0"], true);
    assert_eq!(run["plan"]["resolved_durability"]["2"], false);
    assert_eq!(run["resume"]["from_index"], 1);

    stop(restarted, restarted_stopped);
    let _ = fs::remove_dir_all(directory);
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

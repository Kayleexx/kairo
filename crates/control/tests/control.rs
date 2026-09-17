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
    Endpoint, RunOutput, RunRequest, RunStatus, Server, cancel, shutdown, snapshot, status, submit,
};

#[test]
fn reads_completed_status_from_an_older_service() {
    let status: RunStatus =
        serde_json::from_str(r#"{"Completed":{"output":42}}"#).expect("older status should decode");
    assert!(matches!(
        status,
        RunStatus::Completed { output: RunOutput::Scalar(42), worker } if worker.is_empty()
    ));
}

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-control-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory should be created");
    path
}

#[test]
fn assigns_queued_runs_to_registered_workers() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint: Endpoint = server.endpoint().clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve_until(&stop));

    let response = request(
        &endpoint,
        format!(
            r#"{{"Register":{{"worker":"worker-1","pid":1,"token":"{}"}}}}"#,
            endpoint.token
        ),
    );
    assert!(response.contains("Ok"));
    submit(
        &endpoint,
        RunRequest {
            id: "run-1".to_owned(),
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
        },
    )
    .expect("run should queue");
    let response = request(
        &endpoint,
        format!(
            r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
            endpoint.token
        ),
    );
    assert!(response.contains("run-1"));
    let response = request(
        &endpoint,
        format!(
            r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"run-1","epoch":1,"output":42}}}}"#,
            endpoint.token
        ),
    );
    assert!(response.contains("Ok"));
    assert!(matches!(
        status(&endpoint, "run-1".to_owned()).expect("status should load"),
        Some(RunStatus::Completed {
            output: RunOutput::Scalar(42),
            worker,
        }) if worker == "worker-1"
    ));
    let current = snapshot(&endpoint).expect("snapshot should load");
    assert_eq!(current.workers.len(), 1);
    assert!(!current.workers[0].busy);

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn stops_a_service_through_the_authenticated_endpoint() {
    let directory = directory();
    let server = Arc::new(Server::start(&directory).expect("server should start"));
    let endpoint = server.endpoint().clone();
    let running = Arc::clone(&server);
    let handle = thread::spawn(move || running.serve());

    shutdown(&endpoint).expect("shutdown should be accepted");
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn cancels_queued_and_waiting_runs_immediately() {
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
    for (id, wait) in [
        ("queued", None),
        (
            "waiting",
            Some(kairo_control::WaitRequest::Signal {
                name: "continue".to_owned(),
            }),
        ),
    ] {
        submit(&endpoint, run(id, wait)).expect("run should submit");
        cancel(&endpoint, id.to_owned()).expect("cancel should succeed");
        assert!(matches!(
            status(&endpoint, id.to_owned()).expect("status should load"),
            Some(RunStatus::Canceled)
        ));
    }
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("null")
    );
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Signal":{{"token":"{}","id":"waiting","signal":"continue"}}}}"#,
                endpoint.token
            )
        )
        .contains("Error")
    );
    assert_eq!(
        snapshot(&endpoint)
            .expect("snapshot should load")
            .runs
            .len(),
        2
    );

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

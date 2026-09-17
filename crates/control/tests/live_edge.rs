#![allow(clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    process,
    sync::atomic::{AtomicUsize, Ordering},
};

use kairo_control::{
    Endpoint, LiveEdgeParticipant, LiveEdgeState, RunRequest, RunStatus, Server, begin_live_edge,
    complete_live_edge, complete_live_edge_output, fail_live_edge, ready_live_edge, snapshot,
    status, streaming_live_edge, submit,
};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-live-edge-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory");
    path
}

fn run(id: &str) -> RunRequest {
    RunRequest {
        id: id.to_owned(),
        workflow: "workflow.yaml".into(),
        state: "run.db".into(),
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

fn assign(server: &Server, worker: &str) {
    let endpoint = server.endpoint().clone();
    request(
        &endpoint,
        format!(
            r#"{{"Register":{{"worker":"{worker}","pid":1,"token":"{}"}}}}"#,
            endpoint.token
        ),
    );
    submit(&endpoint, run("run-1")).expect("submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"{worker}","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );
}

#[test]
fn validates_live_edge_transitions_and_participants() {
    let directory = directory();
    let server = Server::start(&directory).expect("server");
    let running = std::sync::Arc::new(server);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = {
        let server = running.clone();
        let stop = stop.clone();
        std::thread::spawn(move || server.serve_until(&stop))
    };
    assign(&running, "worker-a");
    let endpoint = running.endpoint().clone();
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-b","pid":2,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    running
        .begin_live_edge(
            "edge-1".into(),
            "run-1".into(),
            "a-to-b".into(),
            1,
            0,
            "worker-a".into(),
            1,
            "worker-b".into(),
        )
        .expect("begin");
    assert!(
        running
            .transition_live_edge(
                "edge-1",
                LiveEdgeParticipant::Producer,
                "worker-b",
                1,
                LiveEdgeState::Assigned,
                None
            )
            .is_err()
    );
    assert!(
        running
            .transition_live_edge(
                "edge-1",
                LiveEdgeParticipant::Producer,
                "worker-a",
                2,
                LiveEdgeState::Assigned,
                None
            )
            .is_err()
    );
    running
        .transition_live_edge(
            "edge-1",
            LiveEdgeParticipant::Producer,
            "worker-a",
            1,
            LiveEdgeState::Assigned,
            None,
        )
        .expect("assigned");
    assert!(
        running
            .transition_live_edge(
                "edge-1",
                LiveEdgeParticipant::Producer,
                "worker-a",
                1,
                LiveEdgeState::Ready,
                None
            )
            .is_err()
    );
    running
        .transition_live_edge(
            "edge-1",
            LiveEdgeParticipant::Producer,
            "worker-a",
            1,
            LiveEdgeState::Ready,
            Some("127.0.0.1:1".into()),
        )
        .expect("ready");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-b","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("edge-1")
    );
    running
        .transition_live_edge(
            "edge-1",
            LiveEdgeParticipant::Consumer,
            "worker-b",
            1,
            LiveEdgeState::Streaming,
            None,
        )
        .expect("streaming");
    assert!(matches!(
        running
            .live_edge("edge-1")
            .expect("session")
            .expect("exists")
            .state,
        LiveEdgeState::Streaming
    ));
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("join").expect("serve");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn authenticated_participants_do_not_finish_the_parent_run() {
    let directory = directory();
    let server = Server::start(&directory).expect("server");
    let running = std::sync::Arc::new(server);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = {
        let server = running.clone();
        let stop = stop.clone();
        std::thread::spawn(move || server.serve_until(&stop))
    };
    assign(&running, "worker-a");
    let endpoint = running.endpoint().clone();
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-b","pid":2,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    begin_live_edge(
        &endpoint,
        "worker-a",
        "edge-wire".into(),
        "run-1".into(),
        "a-to-b".into(),
        1,
        0,
        1,
        "worker-b".into(),
    )
    .expect("authenticated begin");
    assert!(
        ready_live_edge(
            &endpoint,
            "wrong-worker",
            "edge-wire".into(),
            "run-1".into(),
            "a-to-b".into(),
            1,
            0,
            "127.0.0.1:1".into(),
        )
        .is_err()
    );
    assert!(
        ready_live_edge(
            &endpoint,
            "worker-a",
            "edge-wire".into(),
            "run-1".into(),
            "a-to-b".into(),
            2,
            0,
            "127.0.0.1:1".into(),
        )
        .is_err()
    );
    ready_live_edge(
        &endpoint,
        "worker-a",
        "edge-wire".into(),
        "run-1".into(),
        "a-to-b".into(),
        1,
        0,
        "127.0.0.1:1".into(),
    )
    .expect("producer ready");
    let response: serde_json::Value = serde_json::from_str(&request(
        &endpoint,
        format!(
            r#"{{"Next":{{"worker":"worker-b","token":"{}"}}}}"#,
            endpoint.token
        ),
    ))
    .expect("response");
    let assignment: kairo_control::Assignment =
        serde_json::from_value(response["Assignment"]["run"].clone()).expect("live assignment");
    streaming_live_edge(&endpoint, "worker-b", &assignment).expect("consumer active");
    assert!(
        snapshot(&endpoint)
            .expect("snapshot")
            .workers
            .iter()
            .filter(|worker| worker.id == "worker-a" || worker.id == "worker-b")
            .all(|worker| worker.busy)
    );
    let producer = kairo_control::Assignment {
        run: run("run-1"),
        epoch: 1,
        live_edge: Some(kairo_control::LiveEdgeAssignment {
            session_id: "edge-wire".into(),
            edge_id: "a-to-b".into(),
            parent_epoch: 1,
            producer_endpoint: "127.0.0.1:1".into(),
            group: 0,
        }),
    };
    complete_live_edge(
        &endpoint,
        "worker-a",
        &producer,
        LiveEdgeParticipant::Producer,
    )
    .expect("producer complete");
    complete_live_edge_output(
        &endpoint,
        "worker-b",
        &assignment,
        LiveEdgeParticipant::Consumer,
        Some(kairo_control::RunOutput::Scalar(1)),
    )
    .expect("consumer complete");
    assert!(matches!(
        status(&endpoint, "run-1".into()).expect("status"),
        Some(RunStatus::Running { .. })
    ));
    assert!(
        snapshot(&endpoint)
            .expect("snapshot")
            .workers
            .iter()
            .all(|worker| worker.id != "worker-b" || !worker.busy)
    );
    assert!(
        complete_live_edge(
            &endpoint,
            "worker-b",
            &assignment,
            LiveEdgeParticipant::Consumer,
        )
        .is_err()
    );
    assert!(
        fail_live_edge(
            &endpoint,
            "worker-a",
            &producer,
            LiveEdgeParticipant::Producer,
            "producer failed".into(),
        )
        .is_err()
    );
    assert!(matches!(
        running
            .live_edge("edge-wire")
            .expect("session")
            .expect("exists")
            .state,
        LiveEdgeState::Completed
    ));
    assert!(
        fail_live_edge(
            &endpoint,
            "worker-a",
            &producer,
            LiveEdgeParticipant::Producer,
            "duplicate".into(),
        )
        .is_err()
    );
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("join").expect("serve");
    let _ = fs::remove_dir_all(directory);
}

fn request(endpoint: &Endpoint, value: String) -> String {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(endpoint.address).expect("connect");
    stream.write_all(value.as_bytes()).expect("write");
    stream.write_all(b"\n").expect("newline");
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .expect("read");
    response
}

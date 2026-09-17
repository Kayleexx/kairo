#![allow(clippy::expect_used)]

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicUsize, Ordering},
};

use kairo_control::{LiveEdgeParticipant, LiveEdgeState, RunRequest, Server, cancel, submit};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-live-edge-recovery-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory");
    path
}

fn run() -> RunRequest {
    RunRequest {
        id: "run-1".into(),
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

fn running_server(
    directory: &Path,
) -> (
    std::sync::Arc<Server>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<Result<(), kairo_control::ControlError>>,
) {
    let server = std::sync::Arc::new(Server::start(directory).expect("server"));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = {
        let server = server.clone();
        let stop = stop.clone();
        std::thread::spawn(move || server.serve_until(&stop))
    };
    (server, stop, handle)
}

fn assign(server: &Server) {
    let endpoint = server.endpoint();
    assert!(
        request(
            endpoint,
            format!(
                r#"{{"Register":{{"worker":"worker-a","pid":1,"token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("Ok")
    );
    submit(server.endpoint(), run()).expect("submit");
    assert!(
        request(
            endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-a","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("run-1")
    );
}

fn request(endpoint: &kairo_control::Endpoint, value: String) -> String {
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

#[test]
fn restart_marks_nonterminal_session_as_interrupted() {
    let directory = directory();
    let (server, stop, handle) = running_server(&directory);
    assign(&server);
    server
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
    server
        .transition_live_edge(
            "edge-1",
            LiveEdgeParticipant::Producer,
            "worker-a",
            1,
            LiveEdgeState::Assigned,
            None,
        )
        .expect("assigned");
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("join").expect("serve");
    drop(server);
    let restarted = Server::start(&directory).expect("restart");
    assert!(matches!(
        restarted
            .live_edge("edge-1")
            .expect("session")
            .expect("exists")
            .state,
        LiveEdgeState::Failed { .. }
    ));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn cancellation_invalidates_a_live_edge_session() {
    let directory = directory();
    let (server, stop, handle) = running_server(&directory);
    assign(&server);
    server
        .begin_live_edge(
            "edge-cancel".into(),
            "run-1".into(),
            "a-to-b".into(),
            1,
            0,
            "worker-a".into(),
            1,
            "worker-b".into(),
        )
        .expect("begin");
    cancel(server.endpoint(), "run-1".into()).expect("cancel");
    assert!(matches!(
        server
            .live_edge("edge-cancel")
            .expect("session")
            .expect("exists")
            .state,
        LiveEdgeState::Cancelled
    ));
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("join").expect("serve");
    let _ = fs::remove_dir_all(directory);
}

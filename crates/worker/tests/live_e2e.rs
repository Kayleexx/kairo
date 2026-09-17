#![allow(clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use kairo_control::{RunOutput, RunRequest, RunStatus, Server, snapshot, status, submit};
use kairo_core::{Config, Workflow};
use kairo_runtime::Runtime;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-live-e2e-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory");
    path
}

fn workflow() -> Workflow {
    let transform = repo("demos/stream/transform.wat");
    let consume = repo("demos/stream/consume.wat");
    Workflow::parse(&format!("workflow: live\nmode: stream\ninput: {}\nsteps:\n  - name: transform\n    component: {}\n  - name: consume\n    component: {}\nedges:\n  - from: transform\n    to: consume\n", repo("demos/stream/input.bin").display(), transform.display(), consume.display()), Path::new("."), 8).expect("workflow")
}

#[test]
fn real_workers_stream_over_controlled_quic_and_complete_the_parent_once() {
    let directory = directory();
    let source = directory.join("live.yaml");
    fs::write(&source, format!("workflow: live\nmode: stream\ninput: {}\nsteps:\n  - name: transform\n    component: {}\n  - name: consume\n    component: {}\nedges:\n  - from: transform\n    to: consume\n", repo("demos/stream/input.bin").display(), repo("demos/stream/transform.wat").display(), repo("demos/stream/consume.wat").display())).expect("workflow file");
    let baseline = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(
            Runtime::new(Config::default())
                .expect("kairo runtime")
                .run_stream_workflow(&workflow(), None, false),
        )
        .expect("baseline");
    let server = Arc::new(Server::start(&directory).expect("server"));
    let stop = Arc::new(AtomicBool::new(false));
    let serving = {
        let server = Arc::clone(&server);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || server.serve_until(&stop))
    };
    let endpoint = server.endpoint().clone();
    let workers = ["worker-a", "worker-b"].map(|worker| {
        let endpoint = endpoint.clone();
        std::thread::spawn(move || kairo_worker::run(endpoint, worker.to_owned(), false))
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while snapshot(&endpoint).map_or(true, |state| state.workers.len() != 2) {
        assert!(Instant::now() < deadline, "workers did not register");
        std::thread::yield_now();
    }
    submit(
        &endpoint,
        RunRequest {
            id: "live-e2e".into(),
            workflow: source,
            state: directory.join("run.db"),
            storage: None,
            wait: None,
            plan: None,
            resume: None,
            preferred_worker: Some("worker-a".into()),
            preferred_deadline_ms: None,
            shape: None,
            stream_input: None,
        },
    )
    .expect("submit");
    let deadline = Instant::now() + Duration::from_secs(20);
    let output = loop {
        match status(&endpoint, "live-e2e".into()).expect("status") {
            Some(RunStatus::Completed { output, .. }) => break output,
            Some(RunStatus::Failed { message }) => panic!("run failed: {message}"),
            _ => assert!(
                Instant::now() < deadline,
                "live run timed out: {:?}",
                snapshot(&endpoint).expect("snapshot")
            ),
        }
        std::thread::yield_now();
    };
    assert!(
        matches!(output, RunOutput::Stream(ref result) if result.bytes == baseline.bytes && result.checksum == baseline.checksum)
    );
    let state = snapshot(&endpoint).expect("snapshot");
    let edge = state
        .live_edges
        .iter()
        .find(|edge| edge.producer_worker == "worker-a" && edge.consumer_worker == "worker-b")
        .expect("the run should use a paired remote-live session");
    assert_eq!(edge.parent_epoch, 1);
    assert!(edge.producer_completed);
    assert!(edge.consumer_completed);
    assert!(matches!(
        edge.state,
        kairo_control::LiveEdgeState::Completed
    ));
    assert!(matches!(
        edge.consumer_output,
        Some(RunOutput::Stream(ref result))
            if result.bytes == baseline.bytes && result.checksum == baseline.checksum
    ));
    let inspection = kairo_runtime::inspect_stream_run(&directory.join("run.db"))
        .expect("read parent stream journal")
        .expect("parent stream journal");
    assert!(matches!(
        inspection.live_edges.as_slice(),
        [edge] if edge.transport == "remote-live"
            && edge.producer_worker == "worker-a"
            && edge.consumer_worker == "worker-b"
            && edge.bytes_sent == Some(baseline.bytes)
            && edge.bytes_received == Some(baseline.bytes)
            && edge.outcome == "completed"
    ));
    stop.store(true, Ordering::Relaxed);
    serving.join().expect("server").expect("serve");
    for worker in workers {
        let _ = worker.join();
    }
    let _ = fs::remove_dir_all(directory);
}

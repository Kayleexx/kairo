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

use kairo_control::{Endpoint, RunRequest, RunStatus, Server, cancel, snapshot, status, submit};

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kairo-control-cancel-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir(&path).expect("fixture directory should be created");
    path
}

#[test]
fn cancel_while_running_lets_the_worker_report_land_as_canceled() {
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
    submit(&endpoint, run("running", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("running")
    );
    cancel(&endpoint, "running".to_owned()).expect("cancel should succeed");
    assert!(matches!(
        status(&endpoint, "running".to_owned()).expect("status should load"),
        Some(RunStatus::CancelRequested { worker, epoch }) if worker == "worker-1" && epoch == 1
    ));

    // the reporting worker/epoch still matches the canceled run: this must be honored as
    // `Canceled`, not rejected -- rejecting it is what used to crash the worker loop.
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"running","epoch":1,"output":42}}}}"#, endpoint.token)).contains("Canceled"));
    assert!(matches!(
        status(&endpoint, "running".to_owned()).expect("status should load"),
        Some(RunStatus::Canceled)
    ));

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn rejects_a_report_with_the_wrong_epoch_or_worker() {
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
    submit(&endpoint, run("running", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("running")
    );

    // no cancellation here: a mismatched epoch or worker against a plain `Running` run is a
    // genuine ownership conflict and must keep failing exactly as before.
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-1","token":"{}","id":"running","epoch":99,"output":42}}}}"#, endpoint.token)).contains("Error"));
    assert!(request(&endpoint, format!(r#"{{"Complete":{{"worker":"worker-2","token":"{}","id":"running","epoch":1,"output":42}}}}"#, endpoint.token)).contains("Error"));
    assert!(matches!(
        status(&endpoint, "running".to_owned()).expect("status should load"),
        Some(RunStatus::Running { worker, epoch }) if worker == "worker-1" && epoch == 1
    ));

    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn restart_finalizes_a_cancel_requested_run_as_canceled() {
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
    submit(&endpoint, run("running", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("running")
    );
    cancel(&endpoint, "running".to_owned()).expect("cancel should succeed");
    assert!(matches!(
        status(&endpoint, "running".to_owned()).expect("status should load"),
        Some(RunStatus::CancelRequested { .. })
    ));

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
    // cancellation is a one-way user intent: unlike `Running`, which requeues, a persisted
    // cancellation finalizes immediately rather than depending on a worker that never returns.
    assert!(matches!(
        status(&restarted_endpoint, "running".to_owned()).expect("persisted status should load"),
        Some(RunStatus::Canceled)
    ));
    stopped.store(true, Ordering::Relaxed);
    handle
        .join()
        .expect("server thread should join")
        .expect("server should stop");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn reclaim_finalizes_a_cancel_requested_run_owned_by_a_dead_worker() {
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
    submit(&endpoint, run("running", None)).expect("run should submit");
    assert!(
        request(
            &endpoint,
            format!(
                r#"{{"Next":{{"worker":"worker-1","token":"{}"}}}}"#,
                endpoint.token
            )
        )
        .contains("running")
    );
    cancel(&endpoint, "running".to_owned()).expect("cancel should succeed");

    // the lease timeout is a fixed 3s; past it with no heartbeat, `worker-1` is dead and a
    // snapshot request (which sweeps expired leases) must finalize the run, not leave it stuck.
    thread::sleep(std::time::Duration::from_millis(3200));
    snapshot(&endpoint).expect("snapshot should load");
    assert!(matches!(
        status(&endpoint, "running".to_owned()).expect("status should load"),
        Some(RunStatus::Canceled)
    ));

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

use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use kairo_control::{ControlError, Endpoint, LiveEdgeState, RunStatus, live_edge, status};
use kairo_runtime::RelaySink;

pub(super) struct Watcher {
    stop: mpsc::Sender<()>,
    task: Option<thread::JoinHandle<()>>,
}

const OWNERSHIP_LEASE: Duration = Duration::from_secs(3);

impl Watcher {
    pub(super) fn start(
        endpoint: Endpoint,
        run_id: String,
        session_id: String,
        sink: RelaySink,
    ) -> Self {
        let (stop, receiver) = mpsc::channel();
        let task = thread::spawn(move || {
            let mut last_confirmed = Instant::now();
            loop {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        match ownership(&endpoint, &run_id, &session_id) {
                            Ok(()) => {
                                last_confirmed = Instant::now();
                            }
                            Err(Ownership::Terminal(reason)) => {
                                sink.fail(reason);
                                break;
                            }
                            Err(Ownership::Transient(error)) => {
                                if last_confirmed.elapsed() >= OWNERSHIP_LEASE {
                                    sink.fail(format!(
                                        "live control-plane ownership was not confirmed for {} ms: {}",
                                        OWNERSHIP_LEASE.as_millis(),
                                        error
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
        Self {
            stop,
            task: Some(task),
        }
    }
}

enum Ownership {
    Terminal(String),
    Transient(String),
}

fn ownership(endpoint: &Endpoint, run_id: &str, session_id: &str) -> Result<(), Ownership> {
    match status(endpoint, run_id.to_owned()) {
        Ok(Some(RunStatus::CancelRequested { .. } | RunStatus::Canceled)) => {
            Err(Ownership::Terminal("live run cancelled".to_owned()))
        }
        Ok(Some(RunStatus::Failed { .. })) | Ok(None) => Err(Ownership::Terminal(
            "live run lost control-plane ownership".to_owned(),
        )),
        Ok(_) => match live_edge(endpoint, session_id.to_owned()) {
            Ok(Some(session))
                if matches!(
                    session.state,
                    LiveEdgeState::Failed { .. } | LiveEdgeState::Cancelled
                ) =>
            {
                Err(Ownership::Terminal(
                    "live edge lost participant authority".to_owned(),
                ))
            }
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(Ownership::Terminal(
                "live edge lost control-plane ownership".to_owned(),
            )),
            Err(error) => Err(control_error(error)),
        },
        Err(error) => Err(control_error(error)),
    }
}

fn control_error(error: ControlError) -> Ownership {
    match error {
        ControlError::Rejected { message } => Ownership::Terminal(format!(
            "live control-plane ownership was rejected: {message}"
        )),
        error => Ownership::Transient(format!("live control-plane request failed: {error}")),
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

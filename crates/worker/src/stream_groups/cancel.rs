use std::{sync::mpsc, thread, time::Duration};

use kairo_control::{Endpoint, LiveEdgeState, RunStatus, live_edge, status};
use kairo_runtime::RelaySink;

pub(super) struct Watcher {
    stop: mpsc::Sender<()>,
    task: Option<thread::JoinHandle<()>>,
}

impl Watcher {
    pub(super) fn start(
        endpoint: Endpoint,
        run_id: String,
        session_id: String,
        sink: RelaySink,
    ) -> Self {
        let (stop, receiver) = mpsc::channel();
        let task = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => match status(&endpoint, run_id.clone())
                    {
                        Ok(Some(RunStatus::CancelRequested { .. } | RunStatus::Canceled)) => {
                            sink.fail("live run cancelled");
                            break;
                        }
                        Ok(Some(RunStatus::Failed { .. })) | Ok(None) | Err(_) => {
                            sink.fail("live run lost control-plane ownership");
                            break;
                        }
                        Ok(_) => match live_edge(&endpoint, session_id.clone()) {
                            Ok(Some(session))
                                if matches!(
                                    session.state,
                                    LiveEdgeState::Failed { .. } | LiveEdgeState::Cancelled
                                ) =>
                            {
                                sink.fail("live edge lost participant authority");
                                break;
                            }
                            Ok(Some(_)) => {}
                            Ok(None) | Err(_) => {
                                sink.fail("live edge lost control-plane ownership");
                                break;
                            }
                        },
                    },
                }
            }
        });
        Self {
            stop,
            task: Some(task),
        }
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

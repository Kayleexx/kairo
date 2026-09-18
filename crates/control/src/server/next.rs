use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

use crate::Response;

use super::{State, worker};

pub(super) fn wait(
    worker_id: String,
    shared: &Mutex<State>,
    shutdown: &std::sync::atomic::AtomicBool,
    assignments: &Condvar,
) -> Response {
    let Ok(mut state) = shared.lock() else {
        return unavailable();
    };
    loop {
        let response = worker::next(&mut state, worker_id.clone());
        if let Err(error) = state.persist() {
            return Response::Error {
                message: error.to_string(),
            };
        }
        if !matches!(response, Response::Assignment { run: None })
            || shutdown.load(std::sync::atomic::Ordering::Relaxed)
        {
            return response;
        }
        let Ok((next, timeout)) = assignments.wait_timeout(state, Duration::from_secs(1)) else {
            return unavailable();
        };
        state = next;
        if timeout.timed_out() {
            return response;
        }
    }
}

fn unavailable() -> Response {
    Response::Error {
        message: "control state is unavailable".into(),
    }
}

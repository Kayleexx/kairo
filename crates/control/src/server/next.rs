use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};

use crate::Response;

use super::{State, lock_state, worker};

pub(super) fn wait(
    worker_id: String,
    shared: &Mutex<State>,
    shutdown: &std::sync::atomic::AtomicBool,
    assignments: &Condvar,
) -> Response {
    let mut state = lock_state(shared);
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
        state = match assignments.wait_timeout(state, Duration::from_secs(1)) {
            Ok((next, timeout)) => {
                if timeout.timed_out() {
                    return response;
                }
                next
            }
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}

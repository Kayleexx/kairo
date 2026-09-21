use crate::{ControlError, Endpoint, RunRequest, RunStatus};
use getrandom::fill;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
mod dispatch;
mod next;
mod worker;

const MAX_QUEUE: usize = 1024;
pub(crate) struct State {
    pub(crate) directory: PathBuf,
    pub(crate) workers: BTreeMap<String, Worker>,
    pub(crate) queued: VecDeque<RunRequest>,
    pub(crate) requests: BTreeMap<String, RunRequest>,
    pub(crate) runs: BTreeMap<String, RunStatus>,
    pub(crate) epochs: BTreeMap<String, u64>,
    pub(crate) waiting: BTreeMap<String, crate::WaitRequest>,
    pub(crate) live_edges: BTreeMap<String, crate::LiveEdgeSession>,
    pub(crate) live_assignments: BTreeMap<String, crate::LiveEdgeAssignment>,
    pub(crate) history: BTreeMap<String, Vec<crate::history::RunEvent>>,
    pub(crate) lineages: BTreeMap<String, crate::ReplayLineage>,
    pub(crate) pending_reason: BTreeMap<String, crate::history::AssignmentReason>,
    pub(crate) dirty: bool,
}
/// a panic inside one connection's dispatch must not permanently brick every future request to
/// this long-running server -- the critical sections here are synchronous field mutations with no
/// partial-write invariant to protect, so recovering the guard is safe and keeps the control plane
/// self-healing instead of failing every request with a generic "state is unavailable" forever.
pub(crate) fn lock_state(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct Worker {
    pub(crate) busy: bool,
    pub(crate) last_seen: Instant,
    pub(crate) pid: u32,
}
pub struct Server {
    listener: TcpListener,
    endpoint: Endpoint,
    state: Arc<Mutex<State>>,
    shutdown: Arc<AtomicBool>,
    assignments: Arc<Condvar>,
}
impl Server {
    pub fn start(directory: &Path) -> Result<Self, ControlError> {
        fs::create_dir_all(directory).map_err(|source| ControlError::CreateDirectory {
            path: directory.to_path_buf(),
            source,
        })?;
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|source| ControlError::Bind { source })?;
        listener
            .set_nonblocking(true)
            .map_err(|source| ControlError::Io { source })?;
        let mut bytes = [0; 32];
        fill(&mut bytes).map_err(|source| ControlError::Random { source })?;
        let endpoint = Endpoint {
            address: listener
                .local_addr()
                .map_err(|source| ControlError::Io { source })?,
            token: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        };
        fs::write(
            directory.join("control.json"),
            serde_json::to_vec(&endpoint).map_err(|source| ControlError::Protocol { source })?,
        )
        .map_err(|source| ControlError::Io { source })?;
        let state = State::load(directory)?;
        Ok(Self {
            listener,
            endpoint,
            state: Arc::new(Mutex::new(state)),
            shutdown: Arc::new(AtomicBool::new(false)),
            assignments: Arc::new(Condvar::new()),
        })
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
    #[allow(clippy::too_many_arguments)]
    pub fn begin_live_edge(
        &self,
        session_id: String,
        run_id: String,
        edge_id: String,
        parent_epoch: u64,
        producer_group: usize,
        producer_worker: String,
        consumer_group: usize,
        consumer_worker: String,
    ) -> Result<(), ControlError> {
        let mut state = lock_state(&self.state);
        state
            .begin_live_edge(
                session_id,
                run_id,
                edge_id,
                parent_epoch,
                producer_group,
                producer_worker,
                consumer_group,
                consumer_worker,
            )
            .map_err(|message| ControlError::Rejected { message })?;
        state.persist()
    }
    pub fn transition_live_edge(
        &self,
        session_id: &str,
        participant: crate::LiveEdgeParticipant,
        worker: &str,
        parent_epoch: u64,
        next: crate::LiveEdgeState,
        endpoint: Option<String>,
    ) -> Result<(), ControlError> {
        let mut state = lock_state(&self.state);
        state
            .transition_live_edge(
                session_id,
                participant,
                worker,
                parent_epoch,
                next,
                endpoint,
            )
            .map_err(|message| ControlError::Rejected { message })?;
        state.persist()?;
        self.assignments.notify_all();
        Ok(())
    }
    pub fn live_edge(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::LiveEdgeSession>, ControlError> {
        let state = lock_state(&self.state);
        Ok(state.live_edges.get(session_id).cloned())
    }
    pub fn serve(&self) -> Result<(), ControlError> {
        self.serve_while(|| !self.shutdown.load(Ordering::Relaxed))
    }
    pub fn serve_until(&self, shutdown: &AtomicBool) -> Result<(), ControlError> {
        self.serve_while(|| !shutdown.load(Ordering::Relaxed))
    }
    fn serve_while(&self, keep: impl Fn() -> bool) -> Result<(), ControlError> {
        let listener = self
            .listener
            .try_clone()
            .map_err(|source| ControlError::Io { source })?;
        listener
            .set_nonblocking(false)
            .map_err(|source| ControlError::Io { source })?;
        let done = Arc::new(AtomicBool::new(false));
        let accept_done = Arc::clone(&done);
        let (sender, receiver) = mpsc::channel();
        let acceptor = thread::spawn(move || {
            loop {
                let accepted = listener.accept().map(|(stream, _)| stream);
                if accept_done.load(Ordering::Relaxed) || sender.send(accepted).is_err() {
                    break;
                }
            }
        });
        let result = self.serve_connections(keep, &receiver);
        done.store(true, Ordering::Relaxed);
        self.shutdown.store(true, Ordering::Relaxed);
        self.assignments.notify_all();
        let _ = TcpStream::connect(self.endpoint.address);
        let _ = acceptor.join();
        let _ = self.listener.set_nonblocking(true);
        result
    }

    fn serve_connections(
        &self,
        keep: impl Fn() -> bool,
        receiver: &mpsc::Receiver<std::io::Result<TcpStream>>,
    ) -> Result<(), ControlError> {
        while keep() {
            {
                let mut state = lock_state(&self.state);
                let changed = crate::leases::resume_waiting(&mut state);
                let reclaimed = crate::leases::reclaim_expired(&mut state);
                if changed || reclaimed {
                    let _ = state.persist();
                    self.assignments.notify_all();
                }
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok(stream)) => {
                    let state = Arc::clone(&self.state);
                    let token = self.endpoint.token.clone();
                    let shutdown = Arc::clone(&self.shutdown);
                    let assignments = Arc::clone(&self.assignments);
                    thread::spawn(move || {
                        let _ = dispatch::handle(stream, &token, state, shutdown, assignments);
                    });
                }
                Ok(Err(source)) => return Err(ControlError::Io { source }),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ControlError::State),
            }
        }
        Ok(())
    }
}
pub fn load_endpoint(directory: &Path) -> Result<Endpoint, ControlError> {
    let text = fs::read_to_string(directory.join("control.json")).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ControlError::Unavailable
        } else {
            ControlError::Io { source }
        }
    })?;
    serde_json::from_str(&text).map_err(|source| ControlError::Protocol { source })
}

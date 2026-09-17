use std::{
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context, Poll, Waker},
};

use wasmtime::{
    AsContextMut, StoreContextMut,
    component::{Accessor, Source, StreamConsumer, StreamReader, StreamResult},
};

use crate::{Runtime, RuntimeError, StoreState, stream_input::BufferProducer};

impl Runtime {
    /// Measures one inter-component edge by draining it into a bounded buffer, recording its
    /// real byte count, then re-feeding those bytes to the next stage exactly like a normal
    /// in-memory input buffer. See this module's docs: a temporary measurement path, not the
    /// real streaming architecture.
    pub(in crate::stream) async fn measure_stream_edge(
        &self,
        store: &mut wasmtime::Store<StoreState>,
        step: &str,
        reader: StreamReader<u8>,
        edge_index: usize,
    ) -> crate::Result<StreamReader<u8>> {
        let limit = self.config.max_stream_output_bytes;
        let outcome: wasmtime::Result<Result<Vec<u8>, RuntimeError>> = store
            .run_concurrent(async |accessor| drain_edge_stream(accessor, reader, limit).await)
            .await;
        let bytes = match outcome {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(source)) => return Err(self.stream_step_error(step, source)),
            Err(source) => {
                return Err(
                    self.stream_step_error(step, RuntimeError::MeasureStreamEdge { source })
                );
            }
        };
        if let Some(metrics) = store.data_mut().stream_metrics.as_mut()
            && let Some(edge) = metrics.edges.get_mut(edge_index)
        {
            let measured = bytes.len() as u64;
            edge.bytes = Some(measured);
            edge.materialized = Some(true);
            edge.materialized_bytes = Some(measured);
            // this path never observes true streaming backpressure, so peak buffered bytes
            // stays honestly unknown rather than reporting the materialized total as if it
            // were a real high-water mark.
            edge.peak_buffered_bytes = None;
        }
        let chunk_bytes = self.config.stream_chunk_bytes;
        StreamReader::new(&mut *store, BufferProducer::from_bytes(bytes, chunk_bytes))
            .map_err(|source| self.stream_step_error(step, RuntimeError::CreateStream { source }))
    }
}

/// Drains a component-produced stream fully into memory, bounded by `limit` bytes.
///
/// This is a temporary, host-mediated MEASUREMENT path, not the real streaming architecture.
/// It is used only when a caller explicitly opts into per-edge byte measurement (e.g. a
/// benchmark run); the default `kairo run` streaming path never calls this and is completely
/// unaffected by it. It does not preserve true streaming backpressure, overlap, or timing --
/// see the Phase 13 design notes for the real (concurrent, bounded-relay) streaming graph this
/// stands in for.
pub(super) async fn drain_edge_stream(
    accessor: &Accessor<StoreState>,
    reader: StreamReader<u8>,
    limit: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let shared = Arc::new(Mutex::new(DrainState::new(limit)));
    let consumer = EdgeConsumer {
        shared: Arc::clone(&shared),
    };
    accessor
        .with(|access| reader.pipe(access, consumer))
        .map_err(|source| RuntimeError::MeasureStreamEdge { source })?;
    std::future::poll_fn(|cx| {
        let mut state = lock(&shared);
        if state.done {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    })
    .await;
    let mut state = lock(&shared);
    if state.overflow {
        return Err(RuntimeError::StreamEdgeTooLarge { max_bytes: limit });
    }
    Ok(std::mem::take(&mut state.bytes))
}

fn lock(shared: &Arc<Mutex<DrainState>>) -> MutexGuard<'_, DrainState> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

struct DrainState {
    bytes: Vec<u8>,
    limit: u64,
    overflow: bool,
    done: bool,
    waker: Option<Waker>,
}

impl DrainState {
    fn new(limit: u64) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            overflow: false,
            done: false,
            waker: None,
        }
    }

    /// idempotent: an overflow reported by `poll_consume` must not be overwritten by the
    /// unconditional `finish(false)` the consumer's own `Drop` issues afterward.
    fn finish(&mut self, overflow: bool) {
        if self.done {
            return;
        }
        self.overflow = overflow;
        self.done = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

struct EdgeConsumer {
    shared: Arc<Mutex<DrainState>>,
}

impl StreamConsumer<StoreState> for EdgeConsumer {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreState>,
        source: Source<'_, u8>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if finish {
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        let available = source.remaining(store.as_context_mut());
        if available == 0 {
            return Poll::Pending;
        }
        let mut direct = source.as_direct(store.as_context_mut());
        let mut chunk = vec![0_u8; available];
        let read = std::io::Read::read(&mut direct, &mut chunk).map_err(wasmtime::Error::new)?;
        chunk.truncate(read);
        let mut state = lock(&self.shared);
        state.bytes.extend_from_slice(&chunk);
        if state.bytes.len() as u64 > state.limit {
            state.finish(true);
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl Drop for EdgeConsumer {
    fn drop(&mut self) {
        lock(&self.shared).finish(false);
    }
}

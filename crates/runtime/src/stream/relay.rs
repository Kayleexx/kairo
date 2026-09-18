use std::{
    collections::VecDeque,
    io::Read,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Instant,
};

use wasmtime::{
    AsContextMut, StoreContextMut,
    component::{Destination, Source, StreamConsumer, StreamProducer, StreamResult},
};

use crate::StoreState;

/// A bounded byte relay used when a physical edge leaves a worker. It deliberately owns only a
/// small queue of chunks: its producer side is a normal component stream consumer and its
/// consumer side is used by a transport task. The inverse pair feeds received chunks back into a
/// normal component stream producer.
pub struct StreamRelay;

#[derive(Clone)]
pub struct RelaySink {
    shared: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub struct RelaySource {
    shared: Arc<Mutex<State>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RelayMetrics {
    pub bytes: u64,
    pub peak_buffered_bytes: u64,
    pub producer_polls: u64,
    pub producer_pending: u64,
    pub send_pending: u64,
    pub first_produced_us: Option<u64>,
}

struct Chunk {
    bytes: Vec<u8>,
    offset: usize,
}

struct State {
    chunks: VecDeque<Chunk>,
    buffered: usize,
    limit: usize,
    closed: bool,
    failed: Option<String>,
    producer_waker: Option<Waker>,
    consumer_waker: Option<Waker>,
    observer_waker: Option<Waker>,
    metrics: RelayMetrics,
    started: Instant,
}

impl StreamRelay {
    pub fn bounded(limit: usize) -> (RelaySink, RelaySource) {
        let shared = Arc::new(Mutex::new(State {
            chunks: VecDeque::new(),
            buffered: 0,
            limit,
            closed: false,
            failed: None,
            producer_waker: None,
            consumer_waker: None,
            observer_waker: None,
            metrics: RelayMetrics::default(),
            started: Instant::now(),
        }));
        (
            RelaySink {
                shared: Arc::clone(&shared),
            },
            RelaySource { shared },
        )
    }
}

impl RelaySink {
    pub fn component_consumer(&self) -> RelayConsumer {
        RelayConsumer {
            shared: Arc::clone(&self.shared),
        }
    }

    pub async fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        std::future::poll_fn(|cx| self.poll_send(cx, bytes.clone())).await
    }

    fn poll_send(&self, cx: &mut Context<'_>, bytes: Vec<u8>) -> Poll<Result<(), String>> {
        let mut state = lock(&self.shared)?;
        if let Some(message) = &state.failed {
            return Poll::Ready(Err(message.clone()));
        }
        if state.closed {
            return Poll::Ready(Err("live stream consumer closed".to_owned()));
        }
        if bytes.len() > state.limit {
            return Poll::Ready(Err("live stream frame exceeds relay capacity".to_owned()));
        }
        if state.buffered.saturating_add(bytes.len()) > state.limit {
            state.metrics.send_pending = state.metrics.send_pending.saturating_add(1);
            wake(&mut state.observer_waker);
            state.producer_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        enqueue(&mut state, bytes);
        Poll::Ready(Ok(()))
    }

    pub fn close(&self) {
        if let Ok(mut state) = self.shared.lock() {
            state.closed = true;
            wake(&mut state.consumer_waker);
        }
    }

    pub fn fail(&self, message: impl Into<String>) {
        if let Ok(mut state) = self.shared.lock() {
            state.failed = Some(message.into());
            state.closed = true;
            wake(&mut state.producer_waker);
            wake(&mut state.consumer_waker);
        }
    }

    pub async fn wait_closed(&self) -> Result<(), String> {
        std::future::poll_fn(|cx| {
            let mut state = lock(&self.shared)?;
            if let Some(message) = &state.failed {
                return Poll::Ready(Err(message.clone()));
            }
            if state.closed {
                return Poll::Ready(Ok(()));
            }
            state.consumer_waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    pub async fn wait_for_producer_polls(&self, minimum: u64) -> Result<(), String> {
        std::future::poll_fn(|cx| {
            let mut state = lock(&self.shared)?;
            if state.metrics.producer_polls >= minimum {
                return Poll::Ready(Ok(()));
            }
            state.observer_waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    pub async fn wait_for_blocked_sends(&self, minimum: u64) -> Result<(), String> {
        std::future::poll_fn(|cx| {
            let mut state = lock(&self.shared)?;
            if state.metrics.send_pending >= minimum {
                return Poll::Ready(Ok(()));
            }
            state.observer_waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    pub fn metrics(&self) -> RelayMetrics {
        self.shared
            .lock()
            .map_or_else(|_| RelayMetrics::default(), |state| state.metrics)
    }
}

impl RelaySource {
    pub async fn next(&self) -> Result<Option<Vec<u8>>, String> {
        std::future::poll_fn(|cx| self.poll_next(cx)).await
    }

    fn poll_next(&self, cx: &mut Context<'_>) -> Poll<Result<Option<Vec<u8>>, String>> {
        let mut state = lock(&self.shared)?;
        if let Some(chunk) = state.chunks.pop_front() {
            let bytes = chunk.bytes[chunk.offset..].to_vec();
            state.buffered = state.buffered.saturating_sub(bytes.len());
            wake(&mut state.producer_waker);
            return Poll::Ready(Ok(Some(bytes)));
        }
        if let Some(message) = &state.failed {
            return Poll::Ready(Err(message.clone()));
        }
        if state.closed {
            return Poll::Ready(Ok(None));
        }
        state.consumer_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    pub fn component_producer(&self) -> RelayProducer {
        RelayProducer {
            shared: Arc::clone(&self.shared),
        }
    }

    pub fn metrics(&self) -> RelayMetrics {
        self.shared
            .lock()
            .map_or_else(|_| RelayMetrics::default(), |state| state.metrics)
    }
}

pub struct RelayConsumer {
    shared: Arc<Mutex<State>>,
}

impl StreamConsumer<StoreState> for RelayConsumer {
    type Item = u8;

    fn poll_consume(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut store: StoreContextMut<'_, StoreState>,
        source: Source<'_, u8>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        let mut state = match lock(&self.shared) {
            Ok(state) => state,
            Err(message) => return Poll::Ready(Err(wasmtime::Error::msg(message))),
        };
        if finish {
            state.closed = true;
            wake(&mut state.consumer_waker);
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        let available = source.remaining(store.as_context_mut());
        if available == 0 {
            return Poll::Pending;
        }
        let free = state.limit.saturating_sub(state.buffered);
        if free == 0 {
            state.producer_waker = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let mut bytes = vec![0_u8; available.min(free)];
        let mut direct = source.as_direct(store.as_context_mut());
        let read = match direct.read(&mut bytes) {
            Ok(read) => read,
            Err(source) => return Poll::Ready(Err(wasmtime::Error::new(source))),
        };
        bytes.truncate(read);
        if bytes.is_empty() {
            return Poll::Pending;
        }
        enqueue(&mut state, bytes);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl Drop for RelayConsumer {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            state.closed = true;
            wake(&mut state.consumer_waker);
        }
    }
}

pub struct RelayProducer {
    shared: Arc<Mutex<State>>,
}

impl StreamProducer<StoreState> for RelayProducer {
    type Item = u8;
    type Buffer = Option<u8>;

    fn poll_produce<'a>(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreState>,
        destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<wasmtime::component::StreamResult>> {
        let mut state = match lock(&self.shared) {
            Ok(state) => state,
            Err(message) => return Poll::Ready(Err(wasmtime::Error::msg(message))),
        };
        state.metrics.producer_polls = state.metrics.producer_polls.saturating_add(1);
        wake(&mut state.observer_waker);
        if finish {
            return Poll::Ready(Ok(wasmtime::component::StreamResult::Cancelled));
        }
        let Some(chunk) = state.chunks.front_mut() else {
            if let Some(message) = &state.failed {
                return Poll::Ready(Err(wasmtime::Error::msg(message.clone())));
            }
            if state.closed {
                return Poll::Ready(Ok(wasmtime::component::StreamResult::Dropped));
            }
            state.metrics.producer_pending = state.metrics.producer_pending.saturating_add(1);
            state.consumer_waker = Some(cx.waker().clone());
            return Poll::Pending;
        };
        let remaining = destination
            .remaining(store.as_context_mut())
            .unwrap_or(chunk.bytes.len().saturating_sub(chunk.offset));
        let count = remaining.min(chunk.bytes.len().saturating_sub(chunk.offset));
        if count == 0 {
            return Poll::Ready(Ok(wasmtime::component::StreamResult::Completed));
        }
        let exhausted;
        {
            let mut direct = destination.as_direct(store.as_context_mut(), count);
            direct.remaining()[..count]
                .copy_from_slice(&chunk.bytes[chunk.offset..chunk.offset + count]);
            direct.mark_written(count);
            chunk.offset += count;
            exhausted = chunk.offset == chunk.bytes.len();
        }
        if state.metrics.first_produced_us.is_none() {
            state.metrics.first_produced_us = Some(
                state
                    .started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            );
        }
        state.buffered = state.buffered.saturating_sub(count);
        if exhausted {
            state.chunks.pop_front();
        }
        wake(&mut state.producer_waker);
        Poll::Ready(Ok(wasmtime::component::StreamResult::Completed))
    }
}

fn enqueue(state: &mut State, bytes: Vec<u8>) {
    state.buffered = state.buffered.saturating_add(bytes.len());
    state.metrics.bytes = state.metrics.bytes.saturating_add(bytes.len() as u64);
    state.metrics.peak_buffered_bytes =
        state.metrics.peak_buffered_bytes.max(state.buffered as u64);
    state.chunks.push_back(Chunk { bytes, offset: 0 });
    wake(&mut state.consumer_waker);
}

fn lock(shared: &Arc<Mutex<State>>) -> Result<std::sync::MutexGuard<'_, State>, String> {
    shared
        .lock()
        .map_err(|_| "live stream relay lock poisoned".to_owned())
}

fn wake(waker: &mut Option<Waker>) {
    if let Some(waker) = waker.take() {
        waker.wake();
    }
}

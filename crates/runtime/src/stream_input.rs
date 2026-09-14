use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use kairo_storage::{ArtifactStore, ByteArtifact};
use sha2::{Digest, Sha256};
use thiserror::Error;
use wasmtime::{
    AsContextMut, Store, StoreContextMut,
    component::{Destination, StreamProducer, StreamReader, StreamResult as ProducerResult},
};

use super::{Result, RuntimeError, StoreState};

const INGEST_CHUNK_BYTES: usize = 64 * 1024;

/// streams a local file into content-addressed artifact storage in bounded chunks, without
/// ever materializing the whole file in memory. Not yet wired into `kairo run`'s default
/// execution path -- this is the ingestion primitive a future worker-submitted stream run
/// would use to resolve a durable input artifact reference instead of a local path.
pub(super) async fn resolve_stream_input(
    path: &Path,
    artifacts: &ArtifactStore,
    max_bytes: u64,
) -> Result<ByteArtifact> {
    let mut file = open_regular(path)?;
    let mut writer = artifacts
        .begin_bytes(max_bytes)
        .await
        .map_err(|source| RuntimeError::Artifact { source })?;
    let mut buffer = vec![0_u8; INGEST_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| RuntimeError::ReadStreamInput {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        if let Err(source) = writer.write(&buffer[..read]) {
            writer.abort();
            return Err(RuntimeError::Artifact { source });
        }
    }
    writer
        .finish()
        .await
        .map_err(|source| RuntimeError::Artifact { source })
}

#[derive(Debug, Error)]
#[error("failed to read stream input `{path}`")]
pub(super) struct StreamReadFailure {
    pub(super) path: PathBuf,
    #[source]
    source: io::Error,
}

#[derive(Debug, Error)]
#[error("stream input `{path}` exceeds the {max_bytes}-byte size limit")]
pub(super) struct StreamLimitFailure {
    pub(super) path: PathBuf,
    pub(super) max_bytes: u64,
}

pub(super) enum StreamInput {
    File(FileProducer),
    Materialized(BufferProducer),
}

pub(super) type InputHasher = Arc<Mutex<Sha256>>;

pub(super) fn finish_hash(hasher: &InputHasher) -> Result<String> {
    let digest = hasher
        .lock()
        .map_err(|_| RuntimeError::InputHash)?
        .clone()
        .finalize();
    Ok(format!("sha256:{digest:x}"))
}

impl StreamInput {
    pub(super) fn open(
        path: &Path,
        max_bytes: u64,
        chunk_bytes: usize,
        materialize: bool,
    ) -> Result<(Self, u64, InputHasher)> {
        let hasher = Arc::new(Mutex::new(Sha256::new()));
        if materialize {
            let bytes = read_stream_input(path, max_bytes)?;
            let materialized_bytes = bytes.len() as u64;
            hash(&hasher, &bytes).map_err(|_| RuntimeError::InputHash)?;
            return Ok((
                Self::Materialized(BufferProducer {
                    bytes,
                    position: 0,
                    chunk_bytes,
                }),
                materialized_bytes,
                hasher,
            ));
        }
        Ok((
            Self::File(FileProducer::open(
                path,
                max_bytes,
                chunk_bytes,
                Arc::clone(&hasher),
            )?),
            0,
            hasher,
        ))
    }

    pub(super) fn reader(self, store: &mut Store<StoreState>) -> Result<StreamReader<u8>> {
        let stream = match self {
            Self::File(producer) => StreamReader::new(store, producer),
            Self::Materialized(producer) => StreamReader::new(store, producer),
        };
        stream.map_err(|source| RuntimeError::CreateStream { source })
    }
}

pub(super) struct FileProducer {
    file: File,
    path: PathBuf,
    max_bytes: u64,
    chunk_bytes: usize,
    bytes: u64,
    hasher: InputHasher,
}

impl FileProducer {
    fn open(path: &Path, max_bytes: u64, chunk_bytes: usize, hasher: InputHasher) -> Result<Self> {
        let file = open_regular(path)?;
        let metadata = file
            .metadata()
            .map_err(|source| RuntimeError::OpenStreamInput {
                path: path.to_path_buf(),
                source,
            })?;
        if metadata.len() > max_bytes {
            return Err(RuntimeError::StreamInputTooLarge {
                path: path.to_path_buf(),
                max_bytes,
            });
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
            max_bytes,
            chunk_bytes,
            bytes: 0,
            hasher,
        })
    }
}

impl StreamProducer<StoreState> for FileProducer {
    type Item = u8;
    type Buffer = Option<u8>;

    fn poll_produce<'a>(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreState>,
        destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<ProducerResult>> {
        if finish {
            return Poll::Ready(Ok(ProducerResult::Cancelled));
        }
        let remaining = destination
            .remaining(store.as_context_mut())
            .unwrap_or(self.chunk_bytes);
        let capacity = remaining.min(self.chunk_bytes);
        if capacity == 0 {
            return Poll::Ready(Ok(ProducerResult::Completed));
        }
        let read = {
            let mut destination = destination.as_direct(store.as_context_mut(), capacity);
            let buffer = &mut destination.remaining()[..capacity];
            let read = self.file.read(buffer).map_err(|source| {
                wasmtime::Error::new(StreamReadFailure {
                    path: self.path.clone(),
                    source,
                })
            })?;
            hash(&self.hasher, &buffer[..read]).map_err(wasmtime::Error::new)?;
            destination.mark_written(read);
            read
        };
        self.bytes = self.bytes.saturating_add(read as u64);
        if self.bytes > self.max_bytes {
            return Poll::Ready(Err(wasmtime::Error::new(StreamLimitFailure {
                path: self.path.clone(),
                max_bytes: self.max_bytes,
            })));
        }
        if let Some(metrics) = store.data_mut().stream_metrics.as_mut() {
            metrics.source_bytes = self.bytes;
            metrics.largest_batch_bytes = metrics.largest_batch_bytes.max(read);
        }
        Poll::Ready(Ok(if read == 0 {
            ProducerResult::Dropped
        } else {
            ProducerResult::Completed
        }))
    }
}

pub(super) struct BufferProducer {
    bytes: Vec<u8>,
    position: usize,
    chunk_bytes: usize,
}

impl BufferProducer {
    /// wraps already-in-memory bytes as a producer, e.g. re-feeding a materialized edge
    /// (see `stream::edge_measure`) into the next stage exactly like a real input buffer.
    pub(super) fn from_bytes(bytes: Vec<u8>, chunk_bytes: usize) -> Self {
        Self {
            bytes,
            position: 0,
            chunk_bytes,
        }
    }
}

impl StreamProducer<StoreState> for BufferProducer {
    type Item = u8;
    type Buffer = Option<u8>;

    fn poll_produce<'a>(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        mut store: StoreContextMut<'a, StoreState>,
        destination: Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<ProducerResult>> {
        if finish {
            return Poll::Ready(Ok(ProducerResult::Cancelled));
        }
        let remaining = destination
            .remaining(store.as_context_mut())
            .unwrap_or(self.chunk_bytes);
        let available = self.bytes.len().saturating_sub(self.position);
        let length = remaining.min(self.chunk_bytes).min(available);
        if length == 0 {
            return Poll::Ready(Ok(ProducerResult::Dropped));
        }
        {
            let mut destination = destination.as_direct(store.as_context_mut(), length);
            destination.remaining()[..length]
                .copy_from_slice(&self.bytes[self.position..self.position + length]);
            destination.mark_written(length);
        }
        self.position += length;
        if let Some(metrics) = store.data_mut().stream_metrics.as_mut() {
            metrics.source_bytes = metrics.source_bytes.saturating_add(length as u64);
            metrics.largest_batch_bytes = metrics.largest_batch_bytes.max(length);
        }
        Poll::Ready(Ok(if self.position == self.bytes.len() {
            ProducerResult::Dropped
        } else {
            ProducerResult::Completed
        }))
    }
}

fn read_stream_input(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let file = open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::ReadStreamInput {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(RuntimeError::StreamInputTooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    Ok(bytes)
}

fn open_regular(path: &Path) -> Result<File> {
    let file = File::open(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            RuntimeError::StreamInputNotFound {
                path: path.to_path_buf(),
            }
        } else {
            RuntimeError::OpenStreamInput {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| RuntimeError::OpenStreamInput {
            path: path.to_path_buf(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(RuntimeError::StreamInputNotRegular {
            path: path.to_path_buf(),
        });
    }
    Ok(file)
}

fn hash(hasher: &InputHasher, bytes: &[u8]) -> std::result::Result<(), StreamHashFailure> {
    hasher.lock().map_err(|_| StreamHashFailure)?.update(bytes);
    Ok(())
}

#[derive(Debug, Error)]
#[error("failed to calculate stream input identity")]
pub(super) struct StreamHashFailure;

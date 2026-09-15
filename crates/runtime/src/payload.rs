use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::journal::JournalError;

// above this size a Bytes value goes to a local blob instead of inline in the journal row.
pub(crate) const INLINE_THRESHOLD_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EventPayload {
    Scalar(u32),
    Inline(Vec<u8>),
    Local { hash: String, bytes: u64 },
    Reuse,
}

#[derive(Debug, Error)]
pub enum LocalBlobError {
    #[error("failed to create local blob directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write a local blob")]
    Write {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to publish local blob `{hash}`")]
    Publish {
        hash: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "local blob `{hash}` is missing on this worker's disk; a local payload is never \
         recoverable on another worker or after this worker's state directory is lost"
    )]
    Missing { hash: String },
    #[error("failed to read local blob `{hash}`")]
    Read {
        hash: String,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) struct LocalBlobStore {
    root: PathBuf,
}

impl LocalBlobStore {
    pub(crate) fn open(root: PathBuf) -> Result<Self, LocalBlobError> {
        fs::create_dir_all(&root).map_err(|source| LocalBlobError::CreateDirectory {
            path: root.clone(),
            source,
        })?;
        Ok(Self { root })
    }

    // crash-safe: temp write -> fsync(file) -> atomic rename -> fsync(dir). callers must not
    // journal a `Local` reference before this returns `Ok`, or the journal could point at a hash
    // that isn't durably on disk yet.
    pub(crate) fn put(&self, bytes: &[u8]) -> Result<(String, u64), LocalBlobError> {
        let hash = format!("sha256:{:x}", Sha256::digest(bytes));
        let final_path = self.root.join(filename(&hash));
        if final_path.exists() {
            return Ok((hash, bytes.len() as u64));
        }
        let temporary = self.root.join(temporary_filename());
        let mut file =
            File::create(&temporary).map_err(|source| LocalBlobError::Write { source })?;
        file.write_all(bytes)
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(|source| LocalBlobError::Write { source })?;
        drop(file);
        fs::rename(&temporary, &final_path).map_err(|source| LocalBlobError::Publish {
            hash: hash.clone(),
            source,
        })?;
        // best-effort: the rename also needs a directory fsync to survive a crash, not just the
        // file's contents; harmless to skip on platforms where opening a dir as a File fails.
        if let Ok(directory) = File::open(&self.root) {
            let _ = directory.sync_all();
        }
        Ok((hash, bytes.len() as u64))
    }

    pub(crate) fn get(&self, hash: &str) -> Result<Vec<u8>, LocalBlobError> {
        fs::read(self.root.join(filename(hash))).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                LocalBlobError::Missing {
                    hash: hash.to_owned(),
                }
            } else {
                LocalBlobError::Read {
                    hash: hash.to_owned(),
                    source,
                }
            }
        })
    }
}

fn filename(hash: &str) -> String {
    hash.replace(':', "-")
}

fn temporary_filename() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(".tmp-{}-{timestamp}-{sequence}", std::process::id())
}

#[derive(Default)]
pub(crate) struct EncodedPayload<'a> {
    pub(crate) kind: Option<&'static str>,
    pub(crate) inline: Option<&'a [u8]>,
    pub(crate) hash: Option<&'a str>,
    pub(crate) bytes: Option<i64>,
}

// (legacy INTEGER column value, new payload_* columns) for one payload slot.
pub(crate) fn encode(payload: &EventPayload) -> (Option<i64>, EncodedPayload<'_>) {
    match payload {
        EventPayload::Scalar(value) => (Some(i64::from(*value)), EncodedPayload::default()),
        EventPayload::Inline(bytes) => (
            None,
            EncodedPayload {
                kind: Some("inline"),
                inline: Some(bytes),
                hash: None,
                bytes: Some(bytes.len().min(i64::MAX as usize) as i64),
            },
        ),
        EventPayload::Local { hash, bytes } => (
            None,
            EncodedPayload {
                kind: Some("local"),
                inline: None,
                hash: Some(hash),
                bytes: Some((*bytes).min(i64::MAX as u64) as i64),
            },
        ),
        EventPayload::Reuse => (
            None,
            EncodedPayload {
                kind: Some("reuse"),
                ..EncodedPayload::default()
            },
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode(
    sequence: i64,
    field: &str,
    value: Option<i64>,
    kind: Option<String>,
    inline: Option<Vec<u8>>,
    hash: Option<String>,
    bytes: Option<i64>,
) -> Result<EventPayload, JournalError> {
    match kind.as_deref() {
        None => {
            if inline.is_some() || hash.is_some() || bytes.is_some() {
                return Err(corrupt(
                    sequence,
                    format!("{field} has conflicting payload columns"),
                ));
            }
            let value = value.ok_or_else(|| corrupt(sequence, format!("missing {field}")))?;
            let value =
                u32::try_from(value).map_err(|_| corrupt(sequence, format!("invalid {field}")))?;
            Ok(EventPayload::Scalar(value))
        }
        Some("inline") => {
            if value.is_some() || hash.is_some() {
                return Err(corrupt(
                    sequence,
                    format!("{field} has conflicting payload columns"),
                ));
            }
            let inline =
                inline.ok_or_else(|| corrupt(sequence, format!("missing {field} inline bytes")))?;
            Ok(EventPayload::Inline(inline))
        }
        Some("local") => {
            if value.is_some() || inline.is_some() {
                return Err(corrupt(
                    sequence,
                    format!("{field} has conflicting payload columns"),
                ));
            }
            let hash = hash.ok_or_else(|| corrupt(sequence, format!("missing {field} hash")))?;
            let bytes = bytes.ok_or_else(|| corrupt(sequence, format!("missing {field} bytes")))?;
            let bytes = u64::try_from(bytes)
                .map_err(|_| corrupt(sequence, format!("invalid {field} bytes")))?;
            Ok(EventPayload::Local { hash, bytes })
        }
        Some("reuse") => {
            if value.is_some() || inline.is_some() || hash.is_some() || bytes.is_some() {
                return Err(corrupt(
                    sequence,
                    format!("{field} has conflicting payload columns"),
                ));
            }
            Ok(EventPayload::Reuse)
        }
        Some(other) => Err(corrupt(
            sequence,
            format!("unknown payload kind `{other}` for {field}"),
        )),
    }
}

#[derive(Default)]
pub(crate) struct PayloadColumns {
    pub(crate) kind: Option<String>,
    pub(crate) inline: Option<Vec<u8>>,
    pub(crate) hash: Option<String>,
    pub(crate) bytes: Option<i64>,
}

impl PayloadColumns {
    pub(crate) fn is_empty(&self) -> bool {
        self.kind.is_none() && self.inline.is_none() && self.hash.is_none() && self.bytes.is_none()
    }
}

pub(crate) fn decode_columns(
    sequence: i64,
    field: &str,
    value: Option<i64>,
    columns: PayloadColumns,
) -> Result<EventPayload, JournalError> {
    decode(
        sequence,
        field,
        value,
        columns.kind,
        columns.inline,
        columns.hash,
        columns.bytes,
    )
}

pub(crate) fn absent_columns(
    sequence: i64,
    columns: &PayloadColumns,
    field: &str,
) -> Result<(), JournalError> {
    if !columns.is_empty() {
        return Err(corrupt(
            sequence,
            format!("unexpected {field} payload columns"),
        ));
    }
    Ok(())
}

pub(crate) fn expect_scalar(
    sequence: i64,
    field: &str,
    payload: EventPayload,
) -> Result<u32, JournalError> {
    match payload {
        EventPayload::Scalar(value) => Ok(value),
        _ => Err(corrupt(
            sequence,
            format!("{field} is not a scalar payload"),
        )),
    }
}

fn corrupt(sequence: i64, message: impl Into<String>) -> JournalError {
    JournalError::Corrupt {
        sequence,
        message: message.into(),
    }
}

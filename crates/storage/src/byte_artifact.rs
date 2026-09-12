use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use object_store::{MultipartUpload, ObjectStore, ObjectStoreExt, PutPayload, path::Path};
use sha2::{Digest, Sha256};

use crate::{ByteArtifact, StorageError};

const MULTIPART_PART_BYTES: usize = 5 * 1024 * 1024;
static NEXT_STAGING_OBJECT: AtomicU64 = AtomicU64::new(0);

pub struct ByteArtifactWriter {
    store: Arc<dyn ObjectStore>,
    temporary: PathBuf,
    file: fs::File,
    hasher: Sha256,
    bytes: u64,
    maximum: u64,
}

impl ByteArtifactWriter {
    pub(super) fn new(store: Arc<dyn ObjectStore>, maximum: u64) -> Result<Self, StorageError> {
        let temporary = temporary_path();
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| StorageError::CreateTemporary { source })?;
        Ok(Self {
            store,
            temporary,
            file,
            hasher: Sha256::new(),
            bytes: 0,
            maximum,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        let next = self
            .bytes
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or(StorageError::OutputTooLarge {
                maximum: self.maximum,
            })?;
        if next > self.maximum {
            return Err(StorageError::OutputTooLarge {
                maximum: self.maximum,
            });
        }
        self.bytes = next;
        self.hasher.update(bytes);
        self.file
            .write_all(bytes)
            .map_err(|source| StorageError::WriteTemporary { source })?;
        Ok(())
    }

    pub async fn finish(mut self) -> Result<ByteArtifact, StorageError> {
        self.file
            .flush()
            .and_then(|()| self.file.sync_all())
            .map_err(|source| StorageError::WriteTemporary { source })?;
        let hash = format!("sha256:{:x}", self.hasher.finalize());
        let staging = staging_path();
        let mut upload = self
            .store
            .put_multipart(&staging)
            .await
            .map_err(|source| StorageError::BeginUpload { source })?;
        let mut input = fs::File::open(&self.temporary)
            .map_err(|source| StorageError::ReadTemporary { source })?;
        let mut buffer = vec![0; MULTIPART_PART_BYTES];
        let mut wrote = false;
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|source| StorageError::ReadTemporary { source })?;
            if count == 0 {
                break;
            }
            wrote = true;
            upload
                .put_part(PutPayload::from(buffer[..count].to_vec()))
                .await
                .map_err(|source| StorageError::WriteUpload { source })?;
        }
        if !wrote {
            upload
                .put_part(PutPayload::from(Vec::new()))
                .await
                .map_err(|source| StorageError::WriteUpload { source })?;
        }
        upload
            .complete()
            .await
            .map_err(|source| StorageError::CompleteUpload { source })?;
        let artifact_path = path(&hash);
        let published = self
            .store
            .copy_if_not_exists(&staging, &artifact_path)
            .await;
        if let Err(source) = published
            && !matches!(source, object_store::Error::AlreadyExists { .. })
        {
            return Err(StorageError::Publish { hash, source });
        }
        let _ = self.store.delete(&staging).await;
        let _ = fs::remove_file(&self.temporary);
        Ok(ByteArtifact {
            reference: artifact_path.to_string(),
            hash,
            bytes: self.bytes,
        })
    }

    pub fn abort(self) {
        let _ = fs::remove_file(self.temporary);
    }
}

pub(super) fn path(hash: &str) -> Path {
    Path::from(format!("outputs/{hash}"))
}

fn staging_path() -> Path {
    let sequence = NEXT_STAGING_OBJECT.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    Path::from(format!("staging/{timestamp}-{sequence}"))
}

fn temporary_path() -> PathBuf {
    let sequence = NEXT_STAGING_OBJECT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("kairo-output-{}-{sequence}", std::process::id()))
}

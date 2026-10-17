//! File-backed storage: one `repos.json` in the per-user data directory.

use crate::repo_registry::storage::{RepoStorage, RepoStore, StoreError};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

/// Stores the registry in `repos.json`.
///
/// Access is serialized with an OS advisory lock on a sidecar `repos.lock` file (the data file
/// itself is replaced by `rename`, so it can't carry the lock). This guards against other
/// threads and other processes. Writes go through `repos.json.tmp` + `rename` so a crash never
/// leaves a half-written data file.
pub struct JsonStorage {
    data_path: PathBuf,
    temp_data_path: PathBuf,
    lock_path: PathBuf,
}
impl JsonStorage {
    /// Creates a storage that keeps its files inside `dir` (created when missing).
    pub fn with_dir(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            data_path: dir.join("repos.json"),
            temp_data_path: dir.join("repos.json.tmp"),
            lock_path: dir.join("repos.lock"),
        })
    }

    /// Blocks until the lock is held; it is released when the returned file is dropped.
    fn lock(&self, exclusive: bool) -> Result<File, StoreError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)?;

        if exclusive {
            file.lock()?;
        } else {
            file.lock_shared()?;
        }
        Ok(file)
    }

    fn read_store(&self) -> Result<RepoStore, StoreError> {
        match std::fs::read(&self.data_path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(RepoStore::new()),
            Err(err) => Err(err.into()),
        }
    }

    /// Caller must hold the exclusive lock.
    fn write_store(&self, store: &RepoStore) -> Result<(), StoreError> {
        let tmp = file_write_or_create().open(&self.temp_data_path)?;
        serde_json::to_writer(&tmp, store)?;
        tmp.sync_all()?;
        std::fs::rename(&self.temp_data_path, &self.data_path)?;
        Ok(())
    }
}
impl RepoStorage for JsonStorage {
    fn new() -> Result<Self, StoreError> {
        let project_path = directories::ProjectDirs::from("nl", "tkibi", "gitDesktop")
            .ok_or(JsonError::FailedToMakeProjectDir)?;
        Self::with_dir(project_path.data_dir())
    }

    fn load(&self) -> Result<RepoStore, StoreError> {
        let _lock = self.lock(false)?;
        self.read_store()
    }

    fn save(&self, store: RepoStore) -> Result<(), StoreError> {
        let _lock = self.lock(true)?;
        self.write_store(&store)
    }

    fn update(
        &self,
        update: impl FnOnce(&mut RepoStore) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let _lock = self.lock(true)?;
        let mut store = self.read_store()?;
        update(&mut store)?;
        self.write_store(&store)
    }
}

/// Errors specific to [`JsonStorage`].
#[derive(Debug, Clone, Error)]
pub enum JsonError {
    /// `repos.json` couldn't be parsed or serialized.
    #[error("{0}")]
    Json(Arc<serde_json::Error>),
    /// The OS did not provide a per-user data directory.
    #[error(
        "error while trying to open/create projectDir: valid home directory path could be retrieved from the operating system"
    )]
    FailedToMakeProjectDir,
}
impl From<serde_json::Error> for JsonError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(Arc::new(value))
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        let error: JsonError = value.into();
        error.into()
    }
}

fn file_write_or_create() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).truncate(true).create(true);

    options
}

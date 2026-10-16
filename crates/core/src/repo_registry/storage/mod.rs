pub mod json_storage;
pub mod memory_storage;
use std::{path::PathBuf, sync::Arc};
#[cfg(test)]
mod tests;

use thiserror::Error;
use vecmap::{ids::IdGenerator, vec_map::VecMap};

use crate::repo_registry::{RepoEntry, RepoId};

pub trait RepoStorage: Sized {
    fn new() -> Result<Self, StoreError>;
    fn load(&self) -> Result<RepoStore, StoreError>;
    fn save(&self, store: RepoStore) -> Result<(), StoreError>;
    fn update(
        &self,
        update: impl FnOnce(&mut RepoStore) -> Result<(), StoreError>,
    ) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, Error)]
pub enum StoreError {
    #[error("{0}")]
    Unknown(Box<str>),
    #[error("path `{0}` is already in use")]
    DuplicatePath(PathBuf),
    #[error("name `{0}` is already in use")]
    DuplicateName(Box<str>),
    #[error("{0}")]
    Io(Arc<std::io::Error>),
    #[error("{0}")]
    Json(#[from] json_storage::JsonError),
    #[error("{0}")]
    Memory(#[from] memory_storage::MemoryError),
}
impl StoreError {
    pub fn io(value: std::io::Error) -> Self {
        Self::Io(Arc::new(value))
    }
}
impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::io(value)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RepoStore {
    pub entries: VecMap<RepoId, RepoEntry>,
    pub alloc: IdGenerator<RepoId>,
}
impl RepoStore {
    pub fn new() -> Self {
        Self::default()
    }
}

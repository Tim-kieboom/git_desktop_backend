//! Storage backends for the repository registry.

pub mod json_storage;
pub mod memory_storage;
use std::{path::PathBuf, sync::Arc};
#[cfg(test)]
mod tests;
use thiserror::Error;
use vecmap::{
    ids::IdGenerator,
    vec_map::{
        VecMap,
        iter::{VecMapValuesIntoIter, VecMapValuesIter, VecMapValuesIterMut},
    },
};

use crate::repo_registry::{RepoEntry, RepoId};

/// Where the registry keeps its data.
///
/// Prefer [`update`](RepoStorage::update) for read-modify-write: implementations make it atomic.
/// A separate [`load`](RepoStorage::load) followed by [`save`](RepoStorage::save) can lose
/// concurrent changes.
pub trait RepoStorage: Sized {
    /// Opens the storage at its default location.
    fn new() -> Result<Self, StoreError>;
    /// Returns a snapshot of the stored data (empty when nothing was saved yet).
    fn load(&self) -> Result<RepoStore, StoreError>;
    /// Replaces the stored data.
    fn save(&self, store: RepoStore) -> Result<(), StoreError>;
    /// Atomically loads the data, applies `update` and stores the result.
    ///
    /// Nothing is stored when `update` returns an error, and the error is passed through.
    fn update(
        &self,
        update: impl FnOnce(&mut RepoStore) -> Result<(), StoreError>,
    ) -> Result<(), StoreError>;
}

/// Errors from a [`RepoStorage`] backend or from the changes made to it.
#[derive(Debug, Clone, Error)]
pub enum StoreError {
    /// Anything that doesn't fit another variant.
    #[error("{0}")]
    Unknown(Box<str>),
    /// A repository with this path is already registered.
    #[error("path `{0}` is already in use")]
    DuplicatePath(PathBuf),
    /// A repository with this name is already registered.
    #[error("name `{0}` is already in use")]
    DuplicateName(Box<str>),
    /// Filesystem error (also used for paths that can't be resolved).
    #[error("{0}")]
    Io(Arc<std::io::Error>),
    /// Error from the JSON backend.
    #[error("{0}")]
    Json(#[from] json_storage::JsonError),
    /// Error from the in-memory backend.
    #[error("{0}")]
    Memory(#[from] memory_storage::MemoryError),
}
impl StoreError {
    /// Wraps an [`std::io::Error`].
    pub fn io(value: std::io::Error) -> Self {
        Self::Io(Arc::new(value))
    }
}
impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::io(value)
    }
}

/// Everything the registry persists: the entries plus the id allocator.
///
/// The allocator is stored with the entries so ids stay unique after removals and restarts.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RepoStore {
    /// Registered repositories by id.
    pub entries: VecMap<RepoId, RepoEntry>,
    /// Hands out the next [`RepoId`].
    pub alloc: IdGenerator<RepoId>,
}
impl RepoStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> VecMapValuesIter<'_, RepoEntry> {
        self.entries.values()
    }

    pub fn iter_mut(&mut self) -> VecMapValuesIterMut<'_, RepoEntry> {
        self.entries.values_mut()
    }
}

impl<'a> IntoIterator for &'a RepoStore {
    type Item = &'a RepoEntry;
    type IntoIter = VecMapValuesIter<'a, RepoEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a> IntoIterator for &'a mut RepoStore {
    type Item = &'a mut RepoEntry;
    type IntoIter = VecMapValuesIterMut<'a, RepoEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
impl IntoIterator for RepoStore {
    type Item = RepoEntry;
    type IntoIter = VecMapValuesIntoIter<RepoEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_values()
    }
}

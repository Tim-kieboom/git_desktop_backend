use crate::repo_registry::storage::{RepoStorage, RepoStore, StoreError};
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use thiserror::Error;

pub struct MemoryStorage {
    entries: RwLock<RepoStore>,
}
impl RepoStorage for MemoryStorage {
    fn new() -> Result<Self, StoreError> {
        Ok(Self {
            entries: RwLock::new(RepoStore::new()),
        })
    }

    fn load(&self) -> Result<RepoStore, StoreError> {
        let entries = mem_result(self.entries.read())?;
        Ok(entries.clone())
    }

    fn save(&self, store: RepoStore) -> Result<(), StoreError> {
        let mut entries = mem_result(self.entries.write())?;
        *entries = store;
        Ok(())
    }

    fn update(
        &self,
        update: impl FnOnce(&mut RepoStore) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        let mut entries = mem_result(self.entries.write())?;
        update(&mut entries)
    }
}

#[derive(Debug, Clone, Error)]
pub enum MemoryError {
    #[error("lock error: {0}")]
    Lock(Box<str>),
}

type ReadError<'a> = PoisonError<RwLockReadGuard<'a, RepoStore>>;
type WriteError<'a> = PoisonError<RwLockWriteGuard<'a, RepoStore>>;
impl<'a> From<ReadError<'a>> for MemoryError {
    fn from(value: ReadError<'a>) -> Self {
        Self::Lock(value.to_string().into_boxed_str())
    }
}
impl<'a> From<WriteError<'a>> for MemoryError {
    fn from(value: WriteError<'a>) -> Self {
        Self::Lock(value.to_string().into_boxed_str())
    }
}

fn mem_result<T, E: Into<MemoryError>>(res: Result<T, E>) -> Result<T, MemoryError> {
    res.map_err(|err| err.into())
}

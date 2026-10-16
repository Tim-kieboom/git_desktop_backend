use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use thiserror::Error;
use vecmap::{ids::IdGenerator, impl_ids};

use crate::repo_registry::storage::{RepoStorage, RepoStore, StoreError};

pub mod storage;
#[cfg(test)]
mod tests;

pub struct RepoRegistry<S: RepoStorage> {
    storage: S,
}
impl<S: RepoStorage> RepoRegistry<S> {
    pub fn new() -> Result<Self, RepoError> {
        let storage = S::new().map_err(RepoError::Store)?;
        Ok(Self { storage })
    }

    pub fn with_storage(storage: S) -> Self {
        Self { storage }
    }

    pub fn add(
        &self,
        path: impl Into<PathBuf>,
        name: impl Into<RepoName>,
    ) -> Result<(), RepoError> {
        let name = name.into();
        let path = path.into().canonicalize().map_err(StoreError::io)?;

        self.storage.update(|store| {
            if let Some(entry) = self.find_by_path_or_name(store, &name, &path) {
                return Err(if entry.path == path {
                    StoreError::DuplicatePath(path)
                } else {
                    StoreError::DuplicateName(name)
                });
            }

            let entry = RepoEntry::new(&mut store.alloc, name, path);
            store.entries.insert(entry.id, entry);
            Ok(())
        })?;
        Ok(())
    }

    pub fn remove(&self, id: RepoId) -> Result<bool, RepoError> {
        let mut is_removed = false;
        self.storage.update(|store| {
            is_removed = store.entries.remove(id).is_some();
            Ok(())
        })?;
        Ok(is_removed)
    }

    pub fn get(&self, id: RepoId) -> Result<Option<RepoEntry>, RepoError> {
        let store = self.storage.load()?;
        Ok(store.entries.get(id).cloned())
    }

    pub fn find(&self, path: &Path) -> Result<Option<RepoEntry>, RepoError> {
        let path = path.canonicalize().map_err(StoreError::io)?;
        let store = self.storage.load()?;
        Ok(self.find_by_path(&store, &path).cloned())
    }

    pub fn list(&self) -> Result<RepoStore, RepoError> {
        self.storage.load().map_err(RepoError::Store)
    }

    fn find_by_path<'a>(&self, store: &'a RepoStore, path: &Path) -> Option<&'a RepoEntry> {
        store.entries.values().find(|entry| entry.path == path)
    }

    fn find_by_path_or_name<'a>(
        &self,
        store: &'a RepoStore,
        name: &str,
        path: &Path,
    ) -> Option<&'a RepoEntry> {
        store
            .entries
            .values()
            .find(|entry| *entry.name == *name || entry.path == path)
    }
}

impl_ids!(RepoId);
pub type RepoName = Box<str>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoEntry {
    pub id: RepoId,
    pub name: RepoName,
    pub path: PathBuf,
    pub added_at: DateTime<Utc>,
}
impl RepoEntry {
    pub fn new(alloc: &mut IdGenerator<RepoId>, name: RepoName, path: PathBuf) -> Self {
        Self {
            name,
            path,
            id: alloc.alloc(),
            added_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum RepoError {
    #[error("{0}")]
    Store(#[from] storage::StoreError),
}

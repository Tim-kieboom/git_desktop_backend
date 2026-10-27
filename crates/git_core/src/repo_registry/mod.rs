//! Registry of the local git repositories the app knows about.

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use thiserror::Error;
use vecmap::{ids::IdGenerator, impl_ids};

use crate::error::GitError;
use crate::repo_registry::storage::{RepoStorage, RepoStore, StoreError};
use crate::{CoreError, CoreResult};

pub mod storage;
#[cfg(test)]
mod tests;

/// Persistent list of registered repositories, backed by a [`RepoStorage`].
///
/// Every mutation runs inside [`RepoStorage::update`], so concurrent callers never lose updates.
pub struct RepoRegistry<S: RepoStorage> {
    storage: S,
}
impl<S: RepoStorage> RepoRegistry<S> {
    /// Creates a registry on top of `S`'s default storage location.
    pub fn new() -> CoreResult<Self> {
        let storage = S::new().map_err(RepoError::Store)?;
        Ok(Self { storage })
    }

    /// Creates a registry on top of an already configured storage (e.g. a custom directory).
    pub fn with_storage(storage: S) -> Self {
        Self { storage }
    }

    /// Registers the repository at `path` under `name`.
    ///
    /// The path is canonicalized, so different spellings of the same directory count as one.
    ///
    /// # Errors
    /// - [`StoreError::Io`] if `path` doesn't exist
    /// - [`RepoError::Git`] if `path` is not a git repository
    /// - [`StoreError::DuplicatePath`] / [`StoreError::DuplicateName`] if the path or name is taken
    pub fn add(&self, path: impl Into<PathBuf>, name: impl Into<RepoName>) -> CoreResult {
        self.inner_add(path, name).map_err(CoreError::Repo)
    }

    fn inner_add(&self, path: impl Into<PathBuf>, name: impl Into<RepoName>) -> RepoResult {
        let name = name.into();
        let path = path.into().canonicalize().map_err(StoreError::io)?;

        Self::check_if_real_git_repo(&path)?;

        self.storage.update(|store| {
            if let Some(entry) = self.find_by_name_or_path(store, &name, &path) {
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

    /// Unregisters a repository. Returns `false` when `id` wasn't registered.
    ///
    /// Only the registry entry is removed; nothing on disk is touched.
    pub fn remove(&self, id: RepoId) -> CoreResult<bool> {
        let mut is_removed = false;
        self.storage
            .update(|store| {
                is_removed = store.entries.remove(id).is_some();
                Ok(())
            })
            .map_err(RepoError::from)?;
        Ok(is_removed)
    }

    /// Looks up a repository by id.
    pub fn get(&self, id: RepoId) -> CoreResult<Option<RepoEntry>> {
        let store = self.storage.load().map_err(RepoError::from)?;
        Ok(store.entries.get(id).cloned())
    }

    /// Looks up a repository by path (canonicalized first, so `path` must exist).
    pub fn find(&self, path: &Path) -> CoreResult<Option<RepoEntry>> {
        self.inner_find(path).map_err(CoreError::Repo)
    }

    fn inner_find(&self, path: &Path) -> RepoResult<Option<RepoEntry>> {
        let path = path.canonicalize().map_err(|e| StoreError::io(e))?;

        let store = self.storage.load()?;
        Ok(self.find_by_path(&store, &path).cloned())
    }

    /// Snapshot of all registered repositories.
    pub fn list(&self) -> CoreResult<RepoStore> {
        self.storage.load().map_err(|e| RepoError::Store(e).into())
    }

    fn check_if_real_git_repo(path: &Path) -> Result<(), git2::Error> {
        git2::Repository::open(path)?;
        Ok(())
    }

    fn find_by_path<'a>(&self, store: &'a RepoStore, path: &Path) -> Option<&'a RepoEntry> {
        store.entries.values().find(|entry| entry.path == path)
    }

    fn find_by_name_or_path<'a>(
        &self,
        store: &'a RepoStore,
        name: &str,
        path: &Path,
    ) -> Option<&'a RepoEntry> {
        store
            .entries
            .values()
            .find(|e| *e.name == *name || e.path == path)
    }
}

impl_ids!(RepoId);
/// Display name of a registered repository; unique within a registry.
pub type RepoName = Box<str>;

/// One registered repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoEntry {
    /// Stable id, unique within the registry and never reused.
    pub id: RepoId,
    /// Display name, unique within the registry.
    pub name: RepoName,
    /// Canonical absolute path of the working directory.
    pub path: PathBuf,
    /// When the repository was registered.
    pub added_at: DateTime<Utc>,
}
impl RepoEntry {
    /// Creates an entry with a fresh id from `alloc`, stamped with the current time.
    fn new(alloc: &mut IdGenerator<RepoId>, name: RepoName, path: PathBuf) -> Self {
        Self {
            name,
            path,
            id: alloc.alloc(),
            added_at: Utc::now(),
        }
    }
}

type RepoResult<T = ()> = Result<T, RepoError>;

/// Errors from [`RepoRegistry`] operations.
#[derive(Debug, Clone, Error)]
pub enum RepoError {
    /// The path could not be opened as a git repository.
    #[error("{0}")]
    Git(#[from] GitError),
    /// The underlying storage failed or rejected the change.
    #[error("{0}")]
    Store(#[from] storage::StoreError),
}
impl From<git2::Error> for RepoError {
    fn from(value: git2::Error) -> Self {
        Self::Git(value.into())
    }
}

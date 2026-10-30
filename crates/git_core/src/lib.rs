//! Host-agnostic git engine: a persistent registry of local repositories and file diffs and status.

mod branch;
mod commit;
mod commit_id;
mod diff;
mod error;
mod log;
mod repo_registry;
mod repository;
mod revision;
mod staging;
mod status;
mod tag;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub use branch::{AheadBehind, BranchInfo, BranchKind};
pub use commit_id::{CommitId, ParseCommitIdError};
pub use diff::{DiffLine, DiffTarget, FileDiff, FileStatus, Hunk, LineKind};
pub use error::{CoreError, CoreResult, GitError, GitResult};
pub use log::{CommitDetail, CommitInfo, LogPage, LogQuery, Person};
pub use repo_registry::*;
pub use status::{ChangeKind, StatusEntry};
pub use tag::TagInfo;

use crate::repository::Repository;

/// Owns one lock per repository directory. Create one and share it (`Arc<RepositoryHub>`); handles from
/// different hubs don't know about each other's locks.
#[derive(Debug, Default)]
pub struct RepositoryHub {
    /// Canonical repository path -> its lock.
    locks: Mutex<HashMap<PathBuf, Arc<tokio::sync::RwLock<()>>>>,
}
impl RepositoryHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle for the repository at `path`. Every handle for the same directory (also through
    /// another spelling of the path, or a symlink) shares one lock.
    ///
    /// # Errors
    /// - [`CoreError::Git`](crate::CoreError::Git) if `path` is not a git repository
    pub fn open(&self, path: &Path) -> CoreResult<AsyncRepository> {
        let repo = Repository::open(path)?;
        let path = repo.as_raw().path();

        let mut store = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        let lock = store.entry(path.to_path_buf()).or_default();
        Ok(AsyncRepository {
            path: path.into(),
            lock: lock.clone(),
        })
    }

    /// [`RepoHub::open`] for a registered repository.
    pub fn open_entry(&self, entry: &RepoEntry) -> CoreResult<AsyncRepository> {
        self.open(&entry.path)
    }
}

/// A cheap, cloneable handle to one repository. Reads run in parallel, writes run alone.
///
/// can be created using [`RepositoryHub`](crate::RepositoryHub).
#[derive(Clone)]
pub struct AsyncRepository {
    path: Arc<Path>,
    lock: Arc<tokio::sync::RwLock<()>>,
}
impl AsyncRepository {
    /// Runs `work` on the blocking pool while holding the repository's read lock.
    pub(crate) async fn run_read<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Repository) -> CoreResult<T> + Send + 'static,
    ) -> CoreResult<T> {
        let guard = self.lock.clone().read_owned().await;
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            let _guard_ = guard;
            let repo = Repository::open(&path)?;
            work(&repo)
        })
        .await
        .map_err(|e| CoreError::Task(e.to_string()))?
    }

    /// Runs `work` on the blocking pool while holding the repository's write lock.
    pub(crate) async fn run_write<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Repository) -> CoreResult<T> + Send + 'static,
    ) -> CoreResult<T> {
        let guard = self.lock.clone().write_owned().await;
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            let _guard_ = guard;
            let repo = Repository::open(&path)?;
            work(&repo)
        })
        .await
        .map_err(|e| CoreError::Task(e.to_string()))?
    }

    // ---- reades ----

    pub async fn status(&self) -> CoreResult<Vec<StatusEntry>> {
        self.run_read(|repo| repo.status()).await
    }

    pub async fn file_diff(
        &self,
        target: DiffTarget,
        path: impl Into<PathBuf>,
    ) -> CoreResult<Option<FileDiff>> {
        let path = path.into();
        self.run_read(move |repo| repo.file_diff(target, &path))
            .await
    }

    pub async fn diff_files(
        &self,
        target: DiffTarget,
        path: Option<PathBuf>,
    ) -> CoreResult<Vec<FileDiff>> {
        self.run_read(move |repo| repo.diff_files(target, path.as_deref()))
            .await
    }

    pub async fn log(&self, query: LogQuery) -> CoreResult<LogPage> {
        self.run_read(move |repo| repo.log(&query)).await
    }

    pub async fn commit_detail(&self, id: CommitId) -> CoreResult<CommitDetail> {
        self.run_read(move |repo| repo.commit_detail(id)).await
    }

    pub async fn branches(&self) -> CoreResult<Vec<BranchInfo>> {
        self.run_read(|repo| repo.branches()).await
    }

    pub async fn ahead_behind(&self, local: &str, upstream: &str) -> CoreResult<AheadBehind> {
        let (local, upstream) = (local.to_owned(), upstream.to_owned());
        self.run_read(move |repo| repo.ahead_behind(&local, &upstream))
            .await
    }

    pub async fn tags(&self) -> CoreResult<Vec<TagInfo>> {
        self.run_read(|repo| repo.tags()).await
    }

    // ---- writes ----

    pub async fn stage(&self, path: impl Into<PathBuf>) -> CoreResult {
        let path = path.into();
        self.run_write(move |repo| repo.stage(&path)).await
    }

    pub async fn unstage(&self, path: impl Into<PathBuf>) -> CoreResult {
        let path = path.into();
        self.run_write(move |repo| repo.unstage(&path)).await
    }

    pub async fn stage_hunk(&self, path: impl Into<PathBuf>, hunk: &Hunk) -> CoreResult {
        let (path, hunk) = (path.into(), hunk.clone());
        self.run_write(move |repo| repo.stage_hunk(&path, &hunk))
            .await
    }

    pub async fn unstage_hunk(&self, path: impl Into<PathBuf>, hunk: &Hunk) -> CoreResult {
        let (path, hunk) = (path.into(), hunk.clone());
        self.run_write(move |repo| repo.unstage_hunk(&path, &hunk))
            .await
    }

    pub async fn commit(&self, message: &str) -> CoreResult<CommitId> {
        let message = message.to_owned();
        self.run_write(move |repo| repo.commit(&message)).await
    }

    pub async fn create_branch(&self, name: &str, start: Option<&str>) -> CoreResult<BranchInfo> {
        let (name, start) = (name.to_owned(), start.map(str::to_owned));
        self.run_write(move |repo| repo.create_branch(&name, start.as_deref()))
            .await
    }

    pub async fn checkout_branch(&self, name: &str) -> CoreResult {
        let name = name.to_owned();
        self.run_write(move |repo| repo.checkout_branch(&name))
            .await
    }

    pub async fn delete_branch(&self, name: &str, force: bool) -> CoreResult {
        let name = name.to_owned();
        self.run_write(move |repo| repo.delete_branch(&name, force))
            .await
    }

    pub async fn create_tag(
        &self,
        name: &str,
        start: Option<&str>,
        message: Option<&str>,
    ) -> CoreResult<TagInfo> {
        let (name, start, message) = (
            name.to_owned(),
            start.map(str::to_owned),
            message.map(str::to_owned),
        );
        self.run_write(move |repo| repo.create_tag(&name, start.as_deref(), message.as_deref()))
            .await
    }

    pub async fn delete_tag(&self, name: &str) -> CoreResult {
        let name = name.to_owned();
        self.run_write(move |repo| repo.delete_tag(&name)).await
    }
}

//! Async facade over [`Repository`] for the daemon.
//!
//! libgit2 is blocking, so every git call runs on tokio's blocking pool. A [`RepoHub`] (one per
//! process) hands out cheap, cloneable [`AsyncRepository`] handles and makes sure that, per
//! repository directory, there is either one writer or any number of readers at a time.
//!
//! # How it should work
//! Per call (`run_read` / `run_write`):
//! 1. wait for the repository's `tokio::sync::RwLock` with `read_owned()` / `write_owned()`
//!    **before** touching the blocking pool, so waiting tasks don't occupy blocking threads;
//! 2. `tokio::task::spawn_blocking` a closure that **owns the guard** (it moves in), opens a fresh
//!    [`Repository`] and runs the work. Owning the guard keeps the lock until the work is really
//!    finished, even when the caller's future is dropped halfway;
//! 3. turn a `JoinError` (panic) into [`CoreError::Task`].
//!
//! `git2::Repository` is `Send` but not `Sync`, so it can't sit behind the `RwLock` and be shared.
//! Opening one per call is cheap and gives readers real parallelism.

use crate::{
    AheadBehind, BranchInfo, CommitDetail, CommitId, CoreError, CoreResult, DiffTarget, FileDiff,
    Hunk, LogPage, LogQuery, RepoEntry, Repository, StatusEntry, TagInfo,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::RwLock;

#[cfg(test)]
mod tests;

/// Owns one lock per repository directory. Create one and share it (`Arc<RepoHub>`); handles from
/// different hubs don't know about each other's locks.
pub struct RepoHub {
    /// Canonical repository path -> its lock.
    locks: Mutex<HashMap<PathBuf, Arc<RwLock<()>>>>,
}
impl Default for RepoHub {
    fn default() -> Self {
        Self::new()
    }
}
impl RepoHub {
    pub fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// A handle for the repository at `path`. Every handle for the same directory (also through
    /// another spelling of the path, or a symlink) shares one lock.
    ///
    /// # Errors
    /// - [`CoreError::Git`](crate::CoreError::Git) if `path` is not a git repository
    pub fn open(&self, path: &Path) -> CoreResult<AsyncRepository> {
        let repo = Repository::open(path)?;
        let path = repo.raw.path();

        let mut store = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        let lock = store.entry(path.to_path_buf()).or_default();
        Ok(AsyncRepository {
            lock: lock.clone(),
            path: path.into(),
        })
    }

    /// [`RepoHub::open`] for a registered repository.
    pub fn open_entry(&self, entry: &RepoEntry) -> CoreResult<AsyncRepository> {
        self.open(&entry.path)
    }
}

/// A cheap, cloneable handle to one repository. Reads run in parallel, writes run alone.
#[derive(Clone)]
pub struct AsyncRepository {
    path: Arc<Path>,
    lock: Arc<RwLock<()>>,
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

    // ---- reads: the async twin of the same method on `Repository` ----

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

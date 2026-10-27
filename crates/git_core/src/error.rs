//! The error type shared by the git operations (`diff`, `status`, `staging`, ...).

use std::{path::PathBuf, sync::Arc};
use thiserror::Error;

use crate::repo_registry::RepoError;

pub type CoreResult<T = ()> = Result<T, CoreError>;
pub type GitResult<T = ()> = Result<T, GitError>;

#[derive(Debug, Clone, Error)]
pub enum CoreError {
    #[error("{0}")]
    Repo(#[from] RepoError),
    #[error("{0}")]
    Git(#[from] GitError),
}

/// Why a git operation on a repository failed.
///
/// Cheap to clone: the libgit2 error is shared.
#[derive(Debug, Clone, Error)]
pub enum GitError {
    /// libgit2 failed (corrupt repository, unreadable file, invalid path, ...).
    #[error("{0}")]
    Git2(Arc<git2::Error>),
    /// The path is not known to the repository: not in `HEAD`, not in the index and not on disk.
    #[error("path not found: {}", .0.display())]
    PathNotFound(PathBuf),
}
impl From<git2::Error> for GitError {
    fn from(value: git2::Error) -> Self {
        Self::Git2(Arc::new(value))
    }
}

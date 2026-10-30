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
    /// A background git task panicked or was cancelled before it finished.
    #[error("git task failed: {0}")]
    Task(String),
    #[error("{0}")]
    Io(Arc<std::io::Error>),
}
impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(Arc::new(value))
    }
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
    /// The path can't be used here: it must be a relative path to a single file inside the
    /// working directory (not absolute, not a directory, no `..`).
    #[error("invalid path: {}", .0.display())]
    InvalidPath(PathBuf),
    /// The operation needs a working directory, but the repository is bare.
    #[error("repository has no working directory")]
    BareRepository,
    /// Nothing is staged, or what is staged equals the last commit.
    #[error("nothing to commit")]
    NothingToCommit,
    /// The commit message is empty or only whitespace.
    #[error("commit message is empty")]
    EmptyMessage,
    /// `user.name` or `user.email` is not configured, so there is no author for the commit.
    #[error("git identity is not configured (user.name / user.email)")]
    MissingIdentity,
    /// The index still has merge conflicts; resolve and stage the conflicted files first.
    #[error("unresolved merge conflicts")]
    UnresolvedConflicts,
    /// The hunk is not part of the file's current diff: the file changed after the diff was
    /// taken, or the hunk was already staged / unstaged.
    #[error("hunk not found in the current diff of {}", .0.display())]
    HunkNotFound(PathBuf),
    /// The revision (branch, tag, commit id, `HEAD~2`, ...) doesn't name a commit.
    #[error("revision not found: {0}")]
    RevisionNotFound(String),
    /// A branch, tag or reference name that git doesn't allow (`a b`, `a..b`, `x.lock`, ...).
    #[error("invalid name: {0:?}")]
    InvalidName(String),
    /// There is no local branch with this name.
    #[error("branch not found: {0}")]
    BranchNotFound(String),
    /// A branch with this name already exists.
    #[error("branch already exists: {0}")]
    BranchAlreadyExists(String),
    /// The branch is checked out and can't be deleted.
    #[error("cannot delete the checked-out branch: {0}")]
    CurrentBranch(String),
    /// The branch has commits that are not in `HEAD`; deleting it needs `force`.
    #[error("branch is not fully merged: {0}")]
    BranchNotMerged(String),
    /// Switching branches would overwrite uncommitted changes.
    #[error("local changes would be overwritten by the checkout")]
    WouldOverwriteChanges,
    /// There is no tag with this name.
    #[error("tag not found: {0}")]
    TagNotFound(String),
    /// A tag with this name already exists.
    #[error("tag already exists: {0}")]
    TagAlreadyExists(String),
}
impl From<git2::Error> for GitError {
    fn from(value: git2::Error) -> Self {
        Self::Git2(Arc::new(value))
    }
}

//! Host-agnostic git engine: a persistent registry of local repositories and file diffs and status.

mod diff;
mod error;
mod repo_registry;
mod staging;
mod status;
#[cfg(test)]
mod test_utils;

pub use diff::{DiffLine, DiffTarget, FileDiff, FileStatus, Hunk, LineKind};
pub use error::{CoreError, CoreResult, GitError, GitResult};
pub use repo_registry::*;

pub fn status(repo: &git2::Repository) -> CoreResult<Vec<status::StatusEntry>> {
    status::status(repo).map_err(CoreError::from)
}

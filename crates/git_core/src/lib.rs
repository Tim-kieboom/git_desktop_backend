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

pub struct Repository {
    raw: git2::Repository,
}
impl Repository {
    pub fn from_repo_entry(entry: &RepoEntry) -> CoreResult<Self> {
        git2::Repository::open(&entry.path)
            .map(|raw| Self { raw })
            .map_err(|e| GitError::from(e).into())
    }

    /// Status of every changed file, sorted by path; empty when the working tree is clean.
    ///
    /// Ignored files are left out, untracked directories are expanded to their files.
    pub fn status(&self) -> CoreResult<Vec<status::StatusEntry>> {
        status::status(&self.raw).map_err(CoreError::from)
    }

    /// Stages the current state of `path`: a new file is added, a modified file is updated and a
    /// deleted file has its deletion staged. A merge conflict on `path` counts as resolved.
    ///
    /// Staging an unchanged or already staged file does nothing. Never modifies the working tree.
    ///
    /// # Errors
    /// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
    /// - [`GitError`] if `path` is absolute or a directory
    pub fn stage(&self, path: &std::path::Path) -> CoreResult {
        staging::stage(&self.raw, path).map_err(CoreError::from)
    }

    /// Puts the index entry of `path` back to what `HEAD` has (a new file becomes untracked again),
    /// like `git restore --staged`. The working tree is left untouched, so edits are kept.
    ///
    /// Unstaging a file that isn't staged does nothing.
    ///
    /// # Errors
    /// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
    /// - [`GitError`] if `path` is absolute or a directory
    pub fn unstage(&self, path: &std::path::Path) -> CoreResult {
        staging::unstage(&self.raw, path).map_err(CoreError::from)
    }

    /// Diff of one file, or `None` when that file has no changes for `target`.
    ///
    /// `path` is relative to the working directory and matched literally (no globbing).
    pub fn file_diff(
        &self,
        target: DiffTarget,
        path: &std::path::Path,
    ) -> CoreResult<Option<FileDiff>> {
        diff::file_diff(&self.raw, target, path).map_err(CoreError::from)
    }

    /// Diffs of all changed files (or just `path` when given), in path order.
    ///
    /// Renames are detected, so a moved file is one [`FileStatus::Renamed`] entry. Diffing an
    /// unborn branch with [`DiffTarget::Staged`] treats everything staged as added.
    pub fn diff_files(
        &self,
        target: DiffTarget,
        path: Option<&std::path::Path>,
    ) -> CoreResult<Vec<FileDiff>> {
        diff::diff_files(&self.raw, target, path).map_err(CoreError::from)
    }
}

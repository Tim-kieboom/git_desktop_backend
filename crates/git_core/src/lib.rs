//! Host-agnostic git engine: a persistent registry of local repositories and file diffs and status.

mod branch;
mod commit;
mod commit_id;
mod diff;
mod error;
mod log;
mod repo_registry;
mod revision;
mod staging;
mod status;
mod tag;
#[cfg(test)]
mod tests;

pub use branch::{BranchInfo, BranchKind};
pub use commit_id::{CommitId, ParseCommitIdError};
pub use diff::{DiffLine, DiffTarget, FileDiff, FileStatus, Hunk, LineKind};
pub use log::{CommitDetail, CommitInfo, LogPage, LogQuery, Person};
pub use error::{CoreError, CoreResult, GitError, GitResult};
pub use repo_registry::*;
pub use status::{ChangeKind, StatusEntry};
pub use tag::TagInfo;

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

    /// Stages one hunk of the unstaged changes of `path`, like picking a hunk in `git add -p`.
    ///
    /// `hunk` must come from the current [`Repository::file_diff`] with [`DiffTarget::Unstaged`];
    /// positions shift as hunks get staged, so fetch the diff again between calls. A hunk that is
    /// no longer part of the diff gives [`GitError::HunkNotFound`] and changes nothing.
    pub fn stage_hunk(&self, path: &std::path::Path, hunk: &Hunk) -> CoreResult {
        staging::stage_hunk(&self.raw, path, hunk).map_err(CoreError::from)
    }

    /// Unstages one hunk of the staged changes of `path`, like `git restore -p --staged`.
    ///
    /// `hunk` must come from the current [`Repository::file_diff`] with [`DiffTarget::Staged`].
    /// See [`Repository::stage_hunk`].
    pub fn unstage_hunk(&self, path: &std::path::Path, hunk: &Hunk) -> CoreResult {
        staging::unstage_hunk(&self.raw, path, hunk).map_err(CoreError::from)
    }

    /// Commits what is staged and moves the current branch, like `git commit -m`.
    ///
    /// Returns the id of the new commit. See [`GitError`] for the ways this can fail: blank
    /// message, nothing staged, no git identity configured, or unresolved merge conflicts.
    pub fn commit(&self, message: &str) -> CoreResult<CommitId> {
        commit::commit(&self.raw, message)
            .map(CommitId::from_oid)
            .map_err(CoreError::from)
    }

    /// The commit history, newest first. See [`LogQuery`] for the revision range, path and author
    /// filters and for pagination.
    pub fn log(&self, query: &LogQuery) -> CoreResult<LogPage> {
        log::log(&self.raw, query).map_err(CoreError::from)
    }

    /// A commit with the files it changed (its diff against the first parent).
    pub fn commit_detail(&self, id: CommitId) -> CoreResult<CommitDetail> {
        log::commit_detail(&self.raw, id).map_err(CoreError::from)
    }

    /// Local branches followed by remote ones, each sorted by name.
    pub fn branches(&self) -> CoreResult<Vec<BranchInfo>> {
        branch::list_branches(&self.raw).map_err(CoreError::from)
    }

    /// Creates a local branch at `start` (a revision, `HEAD` when `None`) without checking it out.
    pub fn create_branch(&self, name: &str, start: Option<&str>) -> CoreResult<BranchInfo> {
        branch::create_branch(&self.raw, name, start).map_err(CoreError::from)
    }

    /// Switches to a local branch. Uncommitted changes come along unless they would be
    /// overwritten ([`GitError::WouldOverwriteChanges`]).
    pub fn checkout_branch(&self, name: &str) -> CoreResult {
        branch::checkout_branch(&self.raw, name).map_err(CoreError::from)
    }

    /// Deletes a local branch; one with unmerged commits needs `force`.
    pub fn delete_branch(&self, name: &str, force: bool) -> CoreResult {
        branch::delete_branch(&self.raw, name, force).map_err(CoreError::from)
    }

    /// All tags that point at a commit, sorted by name.
    pub fn tags(&self) -> CoreResult<Vec<TagInfo>> {
        tag::list_tags(&self.raw).map_err(CoreError::from)
    }

    /// Creates a lightweight tag, or an annotated one when `message` is given.
    pub fn create_tag(
        &self,
        name: &str,
        start: Option<&str>,
        message: Option<&str>,
    ) -> CoreResult<TagInfo> {
        tag::create_tag(&self.raw, name, start, message).map_err(CoreError::from)
    }

    /// Deletes a tag; the commit stays.
    pub fn delete_tag(&self, name: &str) -> CoreResult {
        tag::delete_tag(&self.raw, name).map_err(CoreError::from)
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

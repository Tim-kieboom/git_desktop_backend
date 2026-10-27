//! Staging area: move changes to and from the index (`git add` / `git restore --staged`).
//!
//! Paths are relative to the working directory, name one file (not a directory) and are matched
//! literally, not as globs.

use crate::error::GitResult;
use git2::Repository;
use std::path::Path;

#[cfg(test)]
mod tests;

/// Stages the current state of `path`: a new file is added, a modified file is updated and a
/// deleted file has its deletion staged. A merge conflict on `path` counts as resolved.
///
/// Staging an unchanged or already staged file does nothing. Never modifies the working tree.
///
/// # Errors
/// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
/// - [`GitError`] if `path` is absolute or a directory
pub fn stage(_repo: &Repository, _path: &Path) -> GitResult {
    todo!("implement stage")
}

/// Puts the index entry of `path` back to what `HEAD` has (a new file becomes untracked again),
/// like `git restore --staged`. The working tree is left untouched, so edits are kept.
///
/// Unstaging a file that isn't staged does nothing.
///
/// # Errors
/// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
/// - [`GitError`] if `path` is absolute or a directory
pub fn unstage(_repo: &Repository, _path: &Path) -> GitResult {
    todo!("implement unstage")
}

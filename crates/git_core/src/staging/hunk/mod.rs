//! Staging single hunks of a file (`git add -p` / `git restore -p --staged`).
//!
//! A hunk is identified by the [`Hunk`] you got from [`file_diff`](crate::diff::file_diff). It is
//! checked against the file's *current* diff first, so a hunk that went stale (the file changed,
//! or the hunk was already staged) is rejected instead of patching the wrong lines. The patching
//! itself is done by libgit2, so line endings and a missing final newline are preserved.

use super::{check_path, head_tree, stage, unstage};
use crate::{
    diff::{DiffTarget, FileStatus, Hunk, file_diff},
    error::{GitError, GitResult},
};
use git2::{ApplyLocation, ApplyOptions, Diff, DiffOptions, Repository};
use std::{cell::Cell, path::Path};

#[cfg(test)]
mod tests;

/// Stages one hunk of the unstaged changes of `path`; the rest of the file stays unstaged.
///
/// `hunk` must be a hunk of `file_diff(repo, DiffTarget::Unstaged, path)` as it is now. Hunk
/// positions shift when other hunks of the file are staged, so fetch the diff again between calls.
/// For a new (untracked) or deleted file, whose single hunk is the whole file, the whole file is
/// staged. The working tree is never modified, and nothing changes when this fails.
///
/// # Errors
/// - [`GitError::HunkNotFound`] if `hunk` is not part of the current unstaged diff of `path`
/// - the same path errors as [`stage`]: [`GitError::PathNotFound`], [`GitError::InvalidPath`], ...
pub fn stage_hunk(repo: &Repository, path: &Path, hunk: &Hunk) -> GitResult {
    let mut index = repo.index()?;
    let head = head_tree(repo)?;
    check_path(repo, &index, head.as_ref(), path)?;

    let status = current_status(repo, DiffTarget::Unstaged, path, hunk)?;
    if matches!(status, FileStatus::Added | FileStatus::Deleted) {
        return stage(repo, path);
    }

    let mut options = diff_options(path);
    let diff = repo.diff_index_to_workdir(Some(&index), Some(&mut options))?;
    apply_hunk(repo, &diff, path, (hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines))?;
    index.write()?;
    Ok(())
}

/// Unstages one hunk of the staged changes of `path`; the rest of the file stays staged.
///
/// `hunk` must be a hunk of `file_diff(repo, DiffTarget::Staged, path)` as it is now. See
/// [`stage_hunk`] for staleness and whole-file cases (a staged new or deleted file is unstaged
/// completely). The working tree is never modified, and nothing changes when this fails.
///
/// Hunks of a renamed file can't be unstaged one by one: unstage the file instead.
///
/// # Errors
/// - [`GitError::HunkNotFound`] if `hunk` is not part of the current staged diff of `path`
/// - the same path errors as [`unstage`]: [`GitError::PathNotFound`], [`GitError::InvalidPath`], ...
pub fn unstage_hunk(repo: &Repository, path: &Path, hunk: &Hunk) -> GitResult {
    let mut index = repo.index()?;
    let head = head_tree(repo)?;
    check_path(repo, &index, head.as_ref(), path)?;

    let status = current_status(repo, DiffTarget::Staged, path, hunk)?;
    if matches!(status, FileStatus::Added | FileStatus::Deleted) {
        return unstage(repo, path);
    }

    // the reverse diff (index -> HEAD) is what has to be applied to the index to undo the change;
    // its hunks mirror the staged ones, so old and new swap places
    let mut options = diff_options(path);
    options.reverse(true);
    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut options))?;
    apply_hunk(repo, &diff, path, (hunk.new_start, hunk.new_lines, hunk.old_start, hunk.old_lines))?;
    index.write()?;
    Ok(())
}

/// Status of `path` in the current diff, provided `hunk` is one of its hunks.
fn current_status(
    repo: &Repository,
    target: DiffTarget,
    path: &Path,
    hunk: &Hunk,
) -> Result<FileStatus, GitError> {
    match file_diff(repo, target, path)? {
        Some(diff) if diff.hunks.contains(hunk) => Ok(diff.status),
        _ => Err(GitError::HunkNotFound(path.to_path_buf())),
    }
}

fn diff_options(path: &Path) -> DiffOptions {
    let mut options = DiffOptions::new();
    // literal match: file names may contain glob characters like `[`
    options.pathspec(path).disable_pathspec_match(true);
    options
}

/// Applies only the hunk at `(old_start, old_lines, new_start, new_lines)` of `diff` to the index.
fn apply_hunk(
    repo: &Repository,
    diff: &Diff<'_>,
    path: &Path,
    (old_start, old_lines, new_start, new_lines): (u32, u32, u32, u32),
) -> GitResult {
    let matched = Cell::new(0);
    let mut options = ApplyOptions::new();
    options.hunk_callback(|hunk| {
        let is_selected = hunk.is_some_and(|h| {
            (h.old_start(), h.old_lines(), h.new_start(), h.new_lines())
                == (old_start, old_lines, new_start, new_lines)
        });
        if is_selected {
            matched.set(matched.get() + 1);
        }
        is_selected
    });

    repo.apply(diff, ApplyLocation::Index, Some(&mut options))?;
    drop(options);

    // never report success when no hunk matched (nothing was applied)
    if matched.get() == 0 {
        return Err(GitError::HunkNotFound(path.to_path_buf()));
    }
    Ok(())
}

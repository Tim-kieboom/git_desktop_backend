//! Staging area: move changes to and from the index (`git add` / `git restore --staged`).
//!
//! Paths are relative to the working directory, name one file (not a directory) and are matched
//! literally, not as globs.

use crate::error::{GitError, GitResult};
use git2::{Index, ObjectType, Repository, Tree};
use std::path::{Component, Path};

#[cfg(test)]
mod tests;

/// Stages the current state of `path`: a new file is added, a modified file is updated and a
/// deleted file has its deletion staged. A merge conflict on `path` counts as resolved.
///
/// Staging an unchanged or already staged file does nothing. Never modifies the working tree.
///
/// # Errors
/// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
/// - [`GitError::InvalidPath`] if `path` is absolute, contains `..` or is a directory
/// - [`GitError::BareRepository`] if the repository has no working directory
pub fn stage(repo: &Repository, path: &Path) -> GitResult {
    let mut index = repo.index()?;
    let head_tree = head_tree(repo)?;
    check_path(repo, &index, head_tree.as_ref(), path)?;

    if exists_on_disk(repo, path) {
        index.add_path(path)?;
    } else {
        index.remove_path(path)?;
    }

    index.write()?;
    Ok(())
}

/// Puts the index entry of `path` back to what `HEAD` has (a new file becomes untracked again),
/// like `git restore --staged`. The working tree is left untouched, so edits are kept.
///
/// Unstaging a file that isn't staged does nothing.
///
/// # Errors
/// - [`GitError::PathNotFound`] if `path` is not in `HEAD`, the index or the working tree
/// - [`GitError::InvalidPath`] if `path` is absolute, contains `..` or is a directory
/// - [`GitError::BareRepository`] if the repository has no working directory
pub fn unstage(repo: &Repository, path: &Path) -> GitResult {
    let mut index = repo.index()?;
    let head_tree = head_tree(repo)?;
    check_path(repo, &index, head_tree.as_ref(), path)?;

    if head_tree.as_ref().is_some_and(|t| t.get_path(path).is_ok()) {
        let head_commit = repo.head()?.peel_to_commit()?;
        repo.reset_default(Some(head_commit.as_object()), [escape_pathspec(path)])?;
    } else {
        // not in HEAD (new file, or no commits yet): unstaging means dropping the index entry
        index.remove_path(path)?;
        index.write()?;
    }

    Ok(())
}

/// The tree of `HEAD`, or `None` when there are no commits yet.
fn head_tree(repo: &Repository) -> Result<Option<Tree<'_>>, GitError> {
    match repo.head() {
        Ok(head) => Ok(Some(head.peel_to_tree()?)),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Shared validation: a relative path to a single file that `HEAD`, the index or the disk knows.
fn check_path(
    repo: &Repository,
    index: &Index,
    head_tree: Option<&Tree<'_>>,
    path: &Path,
) -> GitResult {
    let invalid = || GitError::InvalidPath(path.to_path_buf());

    let workdir = repo.workdir().ok_or(GitError::BareRepository)?;
    let escapes_workdir = path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    });
    if path.as_os_str().is_empty() || path.is_absolute() || escapes_workdir {
        return Err(invalid());
    }

    let on_disk = workdir.join(path).symlink_metadata();
    if on_disk.as_ref().is_ok_and(|meta| meta.is_dir()) {
        return Err(invalid());
    }

    let in_head = head_tree.and_then(|tree| tree.get_path(path).ok());
    if in_head
        .as_ref()
        .is_some_and(|e| e.kind() == Some(ObjectType::Tree))
    {
        return Err(invalid()); // a directory that only exists in HEAD
    }

    // stages 1-3 only exist while the file is in a merge conflict
    let in_index = (0..=3).any(|stage| index.get_path(path, stage).is_some());
    if on_disk.is_ok() || in_head.is_some() || in_index {
        Ok(())
    } else {
        Err(GitError::PathNotFound(path.to_path_buf()))
    }
}

fn exists_on_disk(repo: &Repository, path: &Path) -> bool {
    repo.workdir()
        .is_some_and(|workdir| workdir.join(path).symlink_metadata().is_ok())
}

/// libgit2 treats paths given to `reset_default` as pathspecs; escape the glob characters so
/// `a[1].txt` means exactly that file.
fn escape_pathspec(path: &Path) -> Vec<u8> {
    let mut escaped = Vec::new();
    for &byte in path_bytes(path) {
        if matches!(byte, b'\\' | b'[' | b']' | b'*' | b'?') {
            escaped.push(b'\\');
        }
        escaped.push(byte);
    }
    escaped
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.to_str().unwrap_or_default().as_bytes()
}

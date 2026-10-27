//! Working tree status: which files changed and whether the change is staged.

use crate::error::GitResult;
use git2::{Repository, StatusOptions};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

/// The kind of change to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// Not tracked by git yet. Only ever used for [`StatusEntry::unstaged`].
    Untracked,
}

/// Status of every changed file, sorted by path; empty when the working tree is clean.
///
/// Ignored files are left out, untracked directories are expanded to their files.
pub fn status(repo: &Repository) -> GitResult<Vec<StatusEntry>> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true);

    let statuses = repo.statuses(Some(&mut options))?;
    let mut entries = statuses.iter().map(StatusEntry::new).collect::<Vec<_>>();

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Status of one file; a file with both staged and unstaged changes is still one entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    /// Path relative to the working directory.
    pub path: Box<Path>,
    /// Previous path, only for staged renames.
    pub old_path: Option<Box<Path>>,
    /// Change from `HEAD` to the index (what the next commit will contain).
    pub staged: Option<ChangeKind>,
    /// Change from the index to the working tree.
    pub unstaged: Option<ChangeKind>,
    /// Unresolved merge conflict.
    pub conflicted: bool,
}
impl StatusEntry {
    fn new(entry: git2::StatusEntry<'_>) -> Self {
        let status = entry.status();
        let is_rename = status.contains(git2::Status::INDEX_RENAMED);
        let conflicted = status.contains(git2::Status::CONFLICTED);

        let rename = entry.head_to_index().filter(|_| is_rename);
        let old_path = rename
            .as_ref()
            .and_then(|d| d.old_file().path_bytes())
            .map(path_from_bytes);

        let path = path_from_bytes(
            rename
                .as_ref()
                .and_then(|d| d.new_file().path_bytes())
                .unwrap_or(entry.path_bytes()),
        );

        Self {
            path,
            old_path,
            conflicted,
            staged: parse_staged(status),
            unstaged: parse_unstaged(status),
        }
    }
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Box<Path> {
    PathBuf::from(OsStr::from_bytes(bytes)).into()
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Box<Path> {
    PathBuf::from(String::from_utf8_lossy(bytes).as_ref()).into()
}

fn parse_staged(status: git2::Status) -> Option<ChangeKind> {
    if status.contains(git2::Status::INDEX_NEW) {
        Some(ChangeKind::Added)
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        Some(ChangeKind::Modified)
    } else if status.contains(git2::Status::INDEX_DELETED) {
        Some(ChangeKind::Deleted)
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        Some(ChangeKind::Renamed)
    } else {
        None
    }
}

fn parse_unstaged(status: git2::Status) -> Option<ChangeKind> {
    if status.contains(git2::Status::WT_NEW) {
        Some(ChangeKind::Untracked)
    } else if status.contains(git2::Status::WT_MODIFIED) {
        Some(ChangeKind::Modified)
    } else if status.contains(git2::Status::WT_DELETED) {
        Some(ChangeKind::Deleted)
    } else {
        None
    }
}

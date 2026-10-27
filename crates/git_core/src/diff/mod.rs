//! File diffs: structured hunks for UIs and unified patch text for display/export.
//!
//! All paths are relative to the repository working directory.

use git2::{Delta, DiffOptions, Patch, Repository};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::GitError;

#[cfg(test)]
mod tests;

/// Which two states of the repository are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffTarget {
    /// Index vs working tree: changes that are not staged yet (includes untracked files).
    Unstaged,
    /// `HEAD` vs index: changes that will go into the next commit.
    Staged,
}

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    /// New file (also untracked files in an [`DiffTarget::Unstaged`] diff).
    Added,
    /// File removed.
    Deleted,
    /// Contents changed in place.
    Modified,
    /// Moved to a new path (see [`FileDiff::old_path`]).
    Renamed,
    /// Copied from another path (see [`FileDiff::old_path`]).
    Copied,
    /// File changed kind, e.g. regular file to symlink.
    TypeChange,
}

/// Role of a line within a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Unchanged line shown for context around a change.
    Context,
    /// Line only present in the new file.
    Added,
    /// Line only present in the old file.
    Removed,
}

/// One line of a hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    /// Whether the line is context, added or removed.
    pub kind: LineKind,
    /// Line number in the old file; `None` for added lines.
    pub old_lineno: Option<u32>,
    /// Line number in the new file; `None` for removed lines.
    pub new_lineno: Option<u32>,
    /// Line text without the trailing newline.
    pub content: String,
    /// The file ends without a newline after this line.
    pub missing_newline: bool,
}

/// A contiguous block of changes with its surrounding context lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// First line of the hunk in the old file (1-based).
    pub old_start: u32,
    /// Number of old-file lines covered (context + removed).
    pub old_lines: u32,
    /// First line of the hunk in the new file (1-based).
    pub new_start: u32,
    /// Number of new-file lines covered (context + added).
    pub new_lines: u32,
    /// The `@@ -a,b +c,d @@ …` line.
    pub header: String,
    /// The lines in file order, context included.
    pub lines: Vec<DiffLine>,
}

/// All changes to one file for a given [`DiffTarget`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    /// Current path (the old path for deleted files).
    pub path: PathBuf,
    /// Previous path for renames and copies.
    pub old_path: Option<PathBuf>,
    /// How the file changed.
    pub status: FileStatus,
    /// Binary files have no hunks; only the status is meaningful.
    pub is_binary: bool,
    /// Empty for binary files and for pure renames.
    pub hunks: Vec<Hunk>,
    /// Unified patch text for this file (`git diff` style, including the file header).
    pub patch: String,
}

/// Diff of one file, or `None` when that file has no changes for `target`.
///
/// `path` is relative to the working directory and matched literally (no globbing).
pub fn file_diff(
    repo: &Repository,
    target: DiffTarget,
    path: &Path,
) -> Result<Option<FileDiff>, GitError> {
    Ok(diff_files(repo, target, Some(path))?.into_iter().next())
}

/// Diffs of all changed files (or just `path` when given), in path order.
///
/// Renames are detected, so a moved file is one [`FileStatus::Renamed`] entry. Diffing an
/// unborn branch with [`DiffTarget::Staged`] treats everything staged as added.
pub fn diff_files(
    repo: &Repository,
    target: DiffTarget,
    path: Option<&Path>,
) -> Result<Vec<FileDiff>, GitError> {
    let mut options = DiffOptions::new();
    if let Some(path) = path {
        // literal match: file names may contain glob characters like `[` or `*`
        options.pathspec(path).disable_pathspec_match(true);
    }

    let mut diff = match target {
        DiffTarget::Unstaged => {
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);

            repo.diff_index_to_workdir(None, Some(&mut options))?
        }
        DiffTarget::Staged => {
            // an unborn branch has no HEAD: everything staged counts as added
            let head_tree = match repo.head() {
                Ok(head) => Some(head.peel_to_tree()?),
                Err(err) if err.code() == git2::ErrorCode::UnbornBranch => None,
                Err(err) => return Err(err.into()),
            };
            repo.diff_tree_to_index(head_tree.as_ref(), None, Some(&mut options))?
        }
    };

    diff.find_similar(None)?;

    let mut files = Vec::new();
    for index in 0..diff.deltas().len() {
        if let Some(mut patch) = Patch::from_diff(&diff, index)? {
            files.push(convert_patch(&mut patch)?);
        }
    }
    Ok(files)
}

fn convert_patch(patch: &mut Patch<'_>) -> Result<FileDiff, GitError> {
    let delta = patch.delta();
    let status = match delta.status() {
        Delta::Added | Delta::Untracked => FileStatus::Added,
        Delta::Deleted => FileStatus::Deleted,
        Delta::Renamed => FileStatus::Renamed,
        Delta::Copied => FileStatus::Copied,
        Delta::Typechange => FileStatus::TypeChange,
        _ => FileStatus::Modified,
    };

    let (new_path, old_path) = (delta.new_file().path(), delta.old_file().path());
    let path = match status {
        FileStatus::Deleted => old_path,
        _ => new_path.or(old_path),
    }
    .map(Path::to_path_buf)
    .unwrap_or_default();

    let old_path = matches!(status, FileStatus::Renamed | FileStatus::Copied)
        .then(|| old_path.map(Path::to_path_buf))
        .flatten();

    let is_binary = delta.flags().is_binary();

    let mut hunks = Vec::with_capacity(patch.num_hunks());
    for hunk_index in 0..patch.num_hunks() {
        let (hunk, line_count) = patch.hunk(hunk_index)?;
        let mut lines: Vec<DiffLine> = Vec::with_capacity(line_count);

        for line_index in 0..line_count {
            let line = patch.line_in_hunk(hunk_index, line_index)?;
            let kind = match line.origin() {
                '+' => LineKind::Added,
                '-' => LineKind::Removed,
                ' ' => LineKind::Context,
                // "\ No newline at end of file" markers belong to the previous line
                '=' | '>' | '<' => {
                    if let Some(last) = lines.last_mut() {
                        last.missing_newline = true;
                    }
                    continue;
                }
                _ => continue,
            };

            let content = String::from_utf8_lossy(line.content());
            lines.push(DiffLine {
                kind,
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
                content: content.trim_end_matches(['\n', '\r']).to_owned(),
                missing_newline: false,
            });
        }

        hunks.push(Hunk {
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            header: String::from_utf8_lossy(hunk.header()).trim_end().to_owned(),
            lines,
        });
    }

    let patch_text = patch
        .to_buf()?
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default();

    Ok(FileDiff {
        path,
        old_path,
        status,
        is_binary,
        hunks,
        patch: patch_text,
    })
}

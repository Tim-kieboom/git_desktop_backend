//! Commit history: a paginated, filterable log and the details of a single commit.

use crate::{
    commit_id::CommitId,
    diff::{self, FileDiff},
    error::{GitError, GitResult},
    revision::resolve_commit,
};
use chrono::{DateTime, Utc};
use git2::{Commit, DiffOptions, Repository, Signature, Sort};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[cfg(test)]
mod tests;

/// Who did something and when: the author or committer of a commit, or the tagger of a tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub time: DateTime<Utc>,
}
impl Person {
    pub(crate) fn from_signature(signature: &Signature<'_>) -> Self {
        Self {
            name: String::from_utf8_lossy(signature.name_bytes()).into_owned(),
            email: String::from_utf8_lossy(signature.email_bytes()).into_owned(),
            time: DateTime::from_timestamp(signature.when().seconds(), 0).unwrap_or_default(),
        }
    }
}

/// One commit in the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub id: CommitId,
    /// Empty for a root commit, two or more for a merge.
    pub parents: Vec<CommitId>,
    pub author: Person,
    pub committer: Person,
    /// The first line of the message.
    pub summary: String,
    /// The whole message, without trailing whitespace.
    pub message: String,
}
impl CommitInfo {
    pub(crate) fn from_commit(commit: &Commit<'_>) -> Self {
        let message = String::from_utf8_lossy(commit.message_bytes());
        let message = message.trim_end().to_owned();
        Self {
            id: CommitId::from_oid(commit.id()),
            parents: commit.parent_ids().map(CommitId::from_oid).collect(),
            author: Person::from_signature(&commit.author()),
            committer: Person::from_signature(&commit.committer()),
            summary: message.lines().next().unwrap_or_default().trim().to_owned(),
            message,
        }
    }
}

/// What to list. The default is the newest 50 commits reachable from `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    /// Where to start: a branch, tag, commit id or any other revision. `HEAD` when `None`.
    pub start: Option<String>,
    /// Leave out everything reachable from this revision, like `git log <exclude>..<start>`.
    pub exclude: Option<String>,
    /// Only commits that changed this file or directory (relative to the working directory).
    /// Merge commits that equal one of their parents at this path are left out.
    pub path: Option<PathBuf>,
    /// Only commits whose author name or email contains this text, ignoring case.
    pub author: Option<String>,
    /// Number of matching commits to skip, for pagination.
    pub skip: usize,
    /// Maximum number of commits to return.
    pub limit: usize,
}
impl Default for LogQuery {
    fn default() -> Self {
        Self {
            start: None,
            exclude: None,
            path: None,
            author: None,
            skip: 0,
            limit: 50,
        }
    }
}

/// One page of the log, newest commits first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPage {
    pub commits: Vec<CommitInfo>,
    /// There are more matching commits after this page.
    pub has_more: bool,
}

/// A commit with the files it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    pub info: CommitInfo,
    /// The diff against the first parent (against the empty tree for a root commit).
    pub files: Vec<FileDiff>,
}

/// Lists commits, newest first (children always before their parents).
///
/// A repository without commits has an empty log. Filters combine, and `skip` / `limit` count
/// only commits that match them.
///
/// # Errors
/// - [`GitError::RevisionNotFound`] if `start` or `exclude` doesn't name a commit
/// - [`GitError::InvalidPath`] if `path` is absolute or contains `..`
pub fn log(repo: &Repository, query: &LogQuery) -> GitResult<LogPage> {
    let empty = LogPage {
        commits: Vec::new(),
        has_more: false,
    };

    if let Some(path) = &query.path {
        let escapes = path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir));
        if escapes || path.as_os_str().is_empty() {
            return Err(GitError::InvalidPath(path.clone()));
        }
    }

    let start = match &query.start {
        Some(spec) => resolve_commit(repo, spec)?,
        None => match resolve_commit(repo, "HEAD") {
            Ok(head) => head,
            Err(GitError::RevisionNotFound(_)) => return Ok(empty), // no commits yet
            Err(err) => return Err(err),
        },
    };

    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    walk.push(start.id())?;
    if let Some(spec) = &query.exclude {
        walk.hide(resolve_commit(repo, spec)?.id())?;
    }

    let author = query.author.as_ref().map(|text| text.to_lowercase());
    let (mut skipped, mut commits, mut has_more) = (0, Vec::new(), false);
    for id in walk {
        let commit = repo.find_commit(id?)?;
        if let Some(text) = &author {
            if !author_matches(&commit, text) {
                continue;
            }
        }
        if let Some(path) = &query.path {
            if !touches_path(repo, &commit, path)? {
                continue;
            }
        }

        if skipped < query.skip {
            skipped += 1;
        } else if commits.len() == query.limit {
            has_more = true;
            break;
        } else {
            commits.push(CommitInfo::from_commit(&commit));
        }
    }

    Ok(LogPage { commits, has_more })
}

/// The details of one commit: its message and the files it changed.
///
/// # Errors
/// - [`GitError::RevisionNotFound`] if `id` is not a commit of this repository
pub fn commit_detail(repo: &Repository, id: CommitId) -> GitResult<CommitDetail> {
    let commit = repo
        .find_commit(id.to_oid())
        .map_err(|_| GitError::RevisionNotFound(id.to_string()))?;

    Ok(CommitDetail {
        info: CommitInfo::from_commit(&commit),
        files: diff::commit_diff(repo, &commit)?,
    })
}

fn author_matches(commit: &Commit<'_>, lowercase_text: &str) -> bool {
    let author = commit.author();
    let name = String::from_utf8_lossy(author.name_bytes()).to_lowercase();
    let email = String::from_utf8_lossy(author.email_bytes()).to_lowercase();
    name.contains(lowercase_text) || email.contains(lowercase_text)
}

/// Whether `commit` changed `path`: it differs there from *all* of its parents (from the empty
/// tree for a root commit), which is how `git log <path>` simplifies history by default.
fn touches_path(repo: &Repository, commit: &Commit<'_>, path: &Path) -> GitResult<bool> {
    let tree = commit.tree()?;
    let differs_from = |parent: Option<&git2::Tree<'_>>| -> GitResult<bool> {
        let mut options = DiffOptions::new();
        // literal match: file names may contain glob characters like `[`
        options.pathspec(path).disable_pathspec_match(true);
        let diff = repo.diff_tree_to_tree(parent, Some(&tree), Some(&mut options))?;
        Ok(diff.deltas().len() > 0)
    };

    if commit.parent_count() == 0 {
        return differs_from(None);
    }
    for parent in commit.parents() {
        if !differs_from(Some(&parent.tree()?))? {
            return Ok(false);
        }
    }
    Ok(true)
}

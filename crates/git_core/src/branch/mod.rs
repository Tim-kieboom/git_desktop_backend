//! Branches: list, create, check out and delete.

use crate::{
    commit_id::CommitId,
    error::{GitError, GitResult},
    revision::resolve_commit,
};
use git2::{Branch, BranchType, ErrorCode, Repository, build::CheckoutBuilder};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// Whether a branch is local or tracks a branch of a remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchKind {
    Local,
    Remote,
}

/// How far two lines of history have moved apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AheadBehind {
    /// Commits that are in the first revision but not in the second.
    pub ahead: usize,
    /// Commits that are in the second revision but not in the first.
    pub behind: usize,
}

/// A branch and where it points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Short name: `main`, `feature/login`, or `origin/main` for a remote branch.
    pub name: String,
    pub kind: BranchKind,
    /// This is the checked-out branch. Never true for remote branches.
    pub is_head: bool,
    /// The commit the branch points at.
    pub target: CommitId,
    /// The branch this one tracks (`origin/main`), if it has an upstream.
    pub upstream: Option<String>,
    /// How this branch compares to its upstream; `None` when it has no (valid) upstream.
    pub ahead_behind: Option<AheadBehind>,
}

/// All local branches (sorted by name) followed by all remote branches (sorted by name).
///
/// A repository without commits has no branches yet.
pub fn list_branches(repo: &Repository) -> GitResult<Vec<BranchInfo>> {
    let mut branches = Vec::new();
    for item in repo.branches(None)? {
        let (branch, kind) = item?;
        let kind = match kind {
            BranchType::Local => BranchKind::Local,
            BranchType::Remote => BranchKind::Remote,
        };

        let Some(info) = branch_info(repo, &branch, kind)? else {
            continue;
        };
        // `origin/HEAD` is a pointer to the remote's default branch, not a branch of its own
        if kind == BranchKind::Remote && info.name.ends_with("/HEAD") {
            continue;
        }
        branches.push(info);
    }

    branches.sort_by(|a, b| {
        (a.kind == BranchKind::Remote, &a.name).cmp(&(b.kind == BranchKind::Remote, &b.name))
    });
    Ok(branches)
}

/// Compares two revisions: how many commits `local` has that `upstream` lacks (`ahead`) and the
/// other way around (`behind`), like `git rev-list --left-right --count local...upstream`.
///
/// Both are counted from the merge base, so diverged histories report both numbers, and
/// histories without a common ancestor count all of their commits. Merge commits and the commits
/// of merged branches are counted like any other.
///
/// # Errors
/// - [`GitError::RevisionNotFound`] if either revision isn't a commit
pub fn ahead_behind(repo: &Repository, local: &str, upstream: &str) -> GitResult<AheadBehind> {
    let local = resolve_commit(repo, local)?;
    let upstream = resolve_commit(repo, upstream)?;
    let (ahead, behind) = repo.graph_ahead_behind(local.id(), upstream.id())?;
    Ok(AheadBehind { ahead, behind })
}

/// Creates a local branch at `start` (a revision; `HEAD` when `None`) without checking it out.
///
/// # Errors
/// - [`GitError::InvalidName`] if git doesn't allow `name`
/// - [`GitError::BranchAlreadyExists`] if the branch exists
/// - [`GitError::RevisionNotFound`] if `start` isn't a commit, or there are no commits yet
pub fn create_branch(repo: &Repository, name: &str, start: Option<&str>) -> GitResult<BranchInfo> {
    if !Branch::name_is_valid(name)? {
        return Err(GitError::InvalidName(name.to_owned()));
    }
    let commit = resolve_commit(repo, start.unwrap_or("HEAD"))?;

    let branch = repo.branch(name, &commit, false).map_err(|err| match err.code() {
        ErrorCode::Exists => GitError::BranchAlreadyExists(name.to_owned()),
        _ => err.into(),
    })?;
    Ok(branch_info(repo, &branch, BranchKind::Local)?.expect("a branch we just made has a valid name"))
}

/// Switches to the local branch `name`: moves `HEAD` and updates the working tree.
///
/// Uncommitted changes are carried over to the new branch, as long as they don't touch files
/// that differ between the two branches. Checking out the current branch does nothing.
///
/// # Errors
/// - [`GitError::BranchNotFound`] if there is no local branch `name`
/// - [`GitError::WouldOverwriteChanges`] if uncommitted changes are in the way. Nothing changed.
pub fn checkout_branch(repo: &Repository, name: &str) -> GitResult {
    let branch = find_local(repo, name)?;
    let tree = branch.get().peel_to_tree()?;

    // files first: if local changes are in the way this fails and HEAD has not moved
    let mut options = CheckoutBuilder::new();
    options.safe();
    repo.checkout_tree(tree.as_object(), Some(&mut options))
        .map_err(|err| match err.code() {
            ErrorCode::Conflict | ErrorCode::Uncommitted => GitError::WouldOverwriteChanges,
            _ => err.into(),
        })?;

    let reference = branch.get().name().expect("branch references have utf-8 names");
    repo.set_head(reference)?;
    Ok(())
}

/// Deletes the local branch `name`. The commits stay in the repository.
///
/// Unless `force` is set, a branch with commits that are not in `HEAD` is kept.
///
/// # Errors
/// - [`GitError::BranchNotFound`] if there is no local branch `name`
/// - [`GitError::CurrentBranch`] if it is the checked-out branch
/// - [`GitError::BranchNotMerged`] if it has unmerged commits and `force` is not set
pub fn delete_branch(repo: &Repository, name: &str, force: bool) -> GitResult {
    let mut branch = find_local(repo, name)?;
    if branch.is_head() {
        return Err(GitError::CurrentBranch(name.to_owned()));
    }

    if !force {
        let tip = branch.get().peel_to_commit()?.id();
        let merged = match repo.head().and_then(|head| head.peel_to_commit()) {
            Ok(head) => head.id() == tip || repo.graph_descendant_of(head.id(), tip)?,
            Err(_) => false,
        };
        if !merged {
            return Err(GitError::BranchNotMerged(name.to_owned()));
        }
    }

    branch.delete()?;
    Ok(())
}

fn find_local<'r>(repo: &'r Repository, name: &str) -> GitResult<Branch<'r>> {
    repo.find_branch(name, BranchType::Local).map_err(|err| match err.code() {
        ErrorCode::NotFound | ErrorCode::InvalidSpec => GitError::BranchNotFound(name.to_owned()),
        _ => err.into(),
    })
}

fn branch_info(
    repo: &Repository,
    branch: &Branch<'_>,
    kind: BranchKind,
) -> GitResult<Option<BranchInfo>> {
    let Ok(Some(name)) = branch.name() else {
        return Ok(None); // not valid utf-8
    };

    let target = branch.get().peel_to_commit()?.id();

    // a configured upstream whose branch is gone is treated as no upstream
    let (mut upstream, mut ahead_behind) = (None, None);
    if kind == BranchKind::Local {
        if let Ok(up) = branch.upstream() {
            upstream = up.name().ok().flatten().map(str::to_owned);
            if let Ok(up_commit) = up.get().peel_to_commit() {
                let (ahead, behind) = repo.graph_ahead_behind(target, up_commit.id())?;
                ahead_behind = Some(AheadBehind { ahead, behind });
            }
        }
    }

    Ok(Some(BranchInfo {
        name: name.to_owned(),
        kind,
        is_head: kind == BranchKind::Local && branch.is_head(),
        target: CommitId::from_oid(target),
        upstream,
        ahead_behind,
    }))
}

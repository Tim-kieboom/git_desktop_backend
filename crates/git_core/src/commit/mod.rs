//! Creating commits from the staging area (`git commit`).

use crate::error::{GitError, GitResult};
use git2::{Commit, ErrorCode, Oid, Repository, RepositoryState};

#[cfg(test)]
mod tests;

/// Commits what is staged and moves the current branch to the new commit, like `git commit -m`.
///
/// The author and committer come from the repository's git config, with the current time. The
/// message is stored without trailing whitespace plus a single newline. While a merge is in
/// progress the commit also gets `MERGE_HEAD` as second parent and the merge state is cleaned up.
///
/// Nothing is changed when this fails.
///
/// # Errors
/// - [`GitError::EmptyMessage`](crate::error::GitError::EmptyMessage) if `message` is blank
/// - [`GitError::NothingToCommit`](crate::error::GitError::NothingToCommit) if the index equals `HEAD`
/// - [`GitError::MissingIdentity`](crate::error::GitError::MissingIdentity) if `user.name` or `user.email` is not set
/// - [`GitError::UnresolvedConflicts`](crate::error::GitError::UnresolvedConflicts) if the index still has conflicts
pub fn commit(repo: &Repository, message: &str) -> GitResult<Oid> {
    let message = message.trim_end();
    if message.trim().is_empty() {
        return Err(GitError::EmptyMessage);
    }

    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Err(GitError::UnresolvedConflicts);
    }

    let signature = repo.signature().map_err(|err| match err.code() {
        ErrorCode::NotFound => GitError::MissingIdentity,
        _ => err.into(),
    })?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head = head_commit(repo)?;
    let merge_heads = merge_heads(repo)?;

    // a merge commit is allowed to have the same tree as HEAD, a normal commit is not
    let unchanged = match &head {
        Some(head) => head.tree_id() == tree_id,
        None => tree.is_empty(),
    };
    if unchanged && merge_heads.is_empty() {
        return Err(GitError::NothingToCommit);
    }

    let mut parents = Vec::with_capacity(1 + merge_heads.len());
    parents.extend(head);
    for id in &merge_heads {
        parents.push(repo.find_commit(*id)?);
    }
    let parents: Vec<&Commit<'_>> = parents.iter().collect();

    let id = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        &format!("{message}\n"),
        &tree,
        &parents,
    )?;

    if !merge_heads.is_empty() {
        repo.cleanup_state()?;
    }
    Ok(id)
}

/// The commit `HEAD` points at, or `None` when there are no commits yet.
fn head_commit(repo: &Repository) -> Result<Option<Commit<'_>>, GitError> {
    match repo.head() {
        Ok(head) => Ok(Some(head.peel_to_commit()?)),
        Err(err) if err.code() == ErrorCode::UnbornBranch => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// The commits being merged into `HEAD` (one per line of `MERGE_HEAD`); empty when not merging.
fn merge_heads(repo: &Repository) -> Result<Vec<Oid>, GitError> {
    if repo.state() != RepositoryState::Merge {
        return Ok(Vec::new());
    }

    // read the file, not the ref: an octopus merge lists several commits
    let content = std::fs::read_to_string(repo.path().join("MERGE_HEAD"))
        .map_err(|err| git2::Error::from_str(&err.to_string()))?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Oid::from_str(line.trim()).map_err(GitError::from))
        .collect()
}

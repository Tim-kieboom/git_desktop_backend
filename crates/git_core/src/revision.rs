//! Resolving user supplied revisions (branch, tag, commit id, `HEAD~2`, ...) to commits.

use crate::error::{GitError, GitResult};
use git2::{Commit, ErrorCode, Repository};

/// The commit that `spec` names, peeling tags. Anything that doesn't resolve to a commit (unknown
/// name, bad syntax, a tree or blob, an unborn `HEAD`) is [`GitError::RevisionNotFound`].
pub(crate) fn resolve_commit<'r>(repo: &'r Repository, spec: &str) -> GitResult<Commit<'r>> {
    let not_found = || GitError::RevisionNotFound(spec.to_owned());

    let object = repo.revparse_single(spec).map_err(|err| match err.code() {
        ErrorCode::NotFound
        | ErrorCode::InvalidSpec
        | ErrorCode::UnbornBranch
        | ErrorCode::Ambiguous => not_found(),
        _ => err.into(),
    })?;
    object.peel_to_commit().map_err(|_| not_found())
}

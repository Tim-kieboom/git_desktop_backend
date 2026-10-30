//! Tags: list, create and delete.

use crate::{
    commit_id::CommitId,
    error::{GitError, GitResult},
    log::Person,
    revision::resolve_commit,
};
use git2::{ErrorCode, ObjectType, Oid, Repository, Tag};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

const TAG_PREFIX: &str = "refs/tags/";

/// A tag and the commit it marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagInfo {
    /// Short name: `v1.0.0`.
    pub name: String,
    /// The commit the tag points at (annotated tags are followed to their commit).
    pub target: CommitId,
    /// An annotated tag is an object of its own with a message and a tagger; a lightweight tag is
    /// only a name for a commit.
    pub annotated: bool,
    /// The message of an annotated tag, without trailing whitespace.
    pub message: Option<String>,
    /// Who made an annotated tag, and when.
    pub tagger: Option<Person>,
}

/// All tags that point at a commit, sorted by name. Tags of trees or blobs are left out.
///
/// The order is plain text order (`v1.10` comes before `v1.9`); sort by version yourself if you
/// need that.
pub fn list_tags(repo: &Repository) -> GitResult<Vec<TagInfo>> {
    let mut found = Vec::new();
    repo.tag_foreach(|oid, name| {
        if let Some(name) = std::str::from_utf8(name).ok().and_then(|n| n.strip_prefix(TAG_PREFIX)) {
            found.push((oid, name.to_owned()));
        }
        true
    })?;

    let mut tags = Vec::with_capacity(found.len());
    for (oid, name) in found {
        if let Some(info) = tag_info(repo, oid, name)? {
            tags.push(info);
        }
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tags)
}

/// Creates a tag on `start` (a revision; `HEAD` when `None`).
///
/// Without a `message` the tag is lightweight. With one it is annotated: the tagger comes from
/// the git config and the message is stored without trailing whitespace plus one newline.
///
/// # Errors
/// - [`GitError::InvalidName`] if git doesn't allow `name`
/// - [`GitError::TagAlreadyExists`] if there is a tag with this name
/// - [`GitError::RevisionNotFound`] if `start` isn't a commit, or there are no commits yet
/// - [`GitError::EmptyMessage`] if `message` is blank
/// - [`GitError::MissingIdentity`] for an annotated tag when `user.name` / `user.email` is unset
pub fn create_tag(
    repo: &Repository,
    name: &str,
    start: Option<&str>,
    message: Option<&str>,
) -> GitResult<TagInfo> {
    if !Tag::is_valid_name(name) {
        return Err(GitError::InvalidName(name.to_owned()));
    }
    let commit = resolve_commit(repo, start.unwrap_or("HEAD"))?;
    if repo.find_reference(&format!("{TAG_PREFIX}{name}")).is_ok() {
        return Err(GitError::TagAlreadyExists(name.to_owned()));
    }

    let created = match message {
        None => repo.tag_lightweight(name, commit.as_object(), false),
        Some(message) => {
            let message = message.trim_end();
            if message.trim().is_empty() {
                return Err(GitError::EmptyMessage);
            }
            let tagger = repo.signature().map_err(|err| match err.code() {
                ErrorCode::NotFound => GitError::MissingIdentity,
                _ => err.into(),
            })?;
            repo.tag(name, commit.as_object(), &tagger, &format!("{message}\n"), false)
        }
    };
    let oid = created.map_err(|err| match err.code() {
        ErrorCode::Exists => GitError::TagAlreadyExists(name.to_owned()),
        _ => err.into(),
    })?;

    Ok(tag_info(repo, oid, name.to_owned())?.expect("a tag on a commit is listed"))
}

/// Deletes the tag `name`. The commit it pointed at stays.
///
/// # Errors
/// - [`GitError::TagNotFound`] if there is no tag with this name
pub fn delete_tag(repo: &Repository, name: &str) -> GitResult {
    repo.tag_delete(name).map_err(|err| match err.code() {
        ErrorCode::NotFound | ErrorCode::InvalidSpec => GitError::TagNotFound(name.to_owned()),
        _ => err.into(),
    })
}

/// `oid` is what `refs/tags/<name>` points at: a commit (lightweight) or a tag object.
fn tag_info(repo: &Repository, oid: Oid, name: String) -> GitResult<Option<TagInfo>> {
    let object = repo.find_object(oid, None)?;
    if object.kind() != Some(ObjectType::Tag) {
        return Ok(object.as_commit().map(|commit| TagInfo {
            name,
            target: CommitId::from_oid(commit.id()),
            annotated: false,
            message: None,
            tagger: None,
        }));
    }

    let tag = object.as_tag().expect("kind was checked");
    let Ok(commit) = tag.target()?.peel_to_commit() else {
        return Ok(None); // an annotated tag of a tree or blob
    };
    Ok(Some(TagInfo {
        name,
        target: CommitId::from_oid(commit.id()),
        annotated: true,
        message: tag
            .message_bytes()
            .map(|bytes| String::from_utf8_lossy(bytes).trim_end().to_owned()),
        tagger: tag.tagger().map(|signature| Person::from_signature(&signature)),
    }))
}

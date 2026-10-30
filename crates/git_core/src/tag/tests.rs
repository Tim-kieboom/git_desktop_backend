//! Tests for listing, creating and deleting tags.
//!
//! # Behaviour the tests pin down
//! - `list_tags` returns tags sorted by name, each with the commit it marks. Annotated tags are
//!   followed to their commit and carry their message and tagger; tags of trees or blobs are left
//!   out.
//! - `create_tag` makes a lightweight tag, or an annotated one when given a message (tagger from
//!   the git config). Bad names are `InvalidName`, existing ones `TagAlreadyExists`.
//! - `delete_tag` removes the tag only, never the commit.
use super::*;
use crate::tests::TestRepo;
use git2::Oid;

fn id(oid: Oid) -> CommitId {
    CommitId::from_oid(oid)
}

fn head(repo: &TestRepo) -> Oid {
    repo.repo.head().unwrap().target().unwrap()
}

fn tag_names(repo: &TestRepo) -> Vec<String> {
    list_tags(&repo.repo).unwrap().into_iter().map(|t| t.name).collect()
}

fn find(repo: &TestRepo, name: &str) -> TagInfo {
    list_tags(&repo.repo).unwrap().into_iter().find(|t| t.name == name).expect(name)
}

// ================================================================================ list

#[test]
fn no_tags_gives_an_empty_list() {
    assert!(list_tags(&TestRepo::with_initial_commit().repo).unwrap().is_empty());
    assert!(list_tags(&TestRepo::init().repo).unwrap().is_empty());
}

#[test]
fn a_lightweight_tag_has_no_message_or_tagger() {
    let repo = TestRepo::with_initial_commit();
    repo.tag("v1.0.0");

    let tag = find(&repo, "v1.0.0");
    assert_eq!(tag.target, id(head(&repo)));
    assert!(!tag.annotated);
    assert_eq!(tag.message, None);
    assert_eq!(tag.tagger, None);
}

#[test]
fn an_annotated_tag_is_followed_to_its_commit() {
    let repo = TestRepo::with_initial_commit();
    let commit = repo.repo.find_commit(head(&repo)).unwrap();
    let sig = git2::Signature::new("Release Bot", "bot@example.com", &git2::Time::new(1_700_000_000, 0)).unwrap();
    repo.repo.tag("v2.0.0", commit.as_object(), &sig, "Release 2.0\n\nBig changes.\n", false).unwrap();

    let tag = find(&repo, "v2.0.0");
    assert_eq!(tag.target, id(commit.id()), "the commit, not the tag object");
    assert!(tag.annotated);
    assert_eq!(tag.message.as_deref(), Some("Release 2.0\n\nBig changes."));
    let tagger = tag.tagger.unwrap();
    assert_eq!((tagger.name.as_str(), tagger.email.as_str()), ("Release Bot", "bot@example.com"));
    assert_eq!(tagger.time.timestamp(), 1_700_000_000);
}

#[test]
fn tags_are_sorted_by_name_as_text() {
    let repo = TestRepo::with_initial_commit();
    for name in ["v1.9", "v1.10", "alpha", "v1.0", "release/2"] {
        repo.tag(name);
    }
    assert_eq!(tag_names(&repo), ["alpha", "release/2", "v1.0", "v1.10", "v1.9"]);
}

#[test]
fn tags_on_different_commits_report_their_own_target() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    repo.tag("old");
    let second = repo.commit_file("a.txt", "a\n", "second");
    repo.tag("new");

    assert_eq!(find(&repo, "old").target, id(first));
    assert_eq!(find(&repo, "new").target, id(second));
}

#[test]
fn tags_that_do_not_point_at_a_commit_are_left_out() {
    let repo = TestRepo::with_initial_commit();
    repo.tag("v1");
    let tree = repo.repo.find_commit(head(&repo)).unwrap().tree().unwrap();
    repo.repo.tag_lightweight("a-tree", tree.as_object(), false).unwrap();
    let blob = repo.repo.blob(b"hello").unwrap();
    repo.repo.tag_lightweight("a-blob", &repo.repo.find_object(blob, None).unwrap(), false).unwrap();

    assert_eq!(tag_names(&repo), ["v1"]);
}

// ============================================================================== create

#[test]
fn create_a_lightweight_tag_on_head() {
    let repo = TestRepo::with_initial_commit();

    let tag = create_tag(&repo.repo, "v1.0.0", None, None).unwrap();
    assert_eq!(tag.name, "v1.0.0");
    assert_eq!(tag.target, id(head(&repo)));
    assert!(!tag.annotated);
    assert_eq!(find(&repo, "v1.0.0"), tag);
}

#[test]
fn create_an_annotated_tag_with_the_configured_tagger() {
    let repo = TestRepo::with_initial_commit();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;

    let tag = create_tag(&repo.repo, "v1.0.0", None, Some("First release  \n\n")).unwrap();
    assert!(tag.annotated);
    assert_eq!(tag.message.as_deref(), Some("First release"));
    assert_eq!(tag.target, id(head(&repo)));
    let tagger = tag.tagger.as_ref().unwrap();
    assert_eq!((tagger.name.as_str(), tagger.email.as_str()), ("Test User", "test@example.com"));
    assert!((tagger.time.timestamp() - now).abs() < 60);

    // stored with exactly one trailing newline, like `git tag -m`
    let object = repo.repo.revparse_single("refs/tags/v1.0.0").unwrap();
    assert_eq!(object.as_tag().unwrap().message().unwrap(), Some("First release\n"));
}

#[test]
fn create_a_tag_on_another_revision() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    repo.commit_file("a.txt", "a\n", "second");
    create_tag(&repo.repo, "by-id", Some(&first.to_string()), None).unwrap();
    create_tag(&repo.repo, "by-parent", Some("HEAD~1"), None).unwrap();
    create_tag(&repo.repo, "annotated-old", Some(&first.to_string()), Some("old")).unwrap();

    for name in ["by-id", "by-parent", "annotated-old"] {
        assert_eq!(find(&repo, name).target, id(first), "{name}");
    }
}

#[test]
fn tag_names_may_contain_slashes() {
    let repo = TestRepo::with_initial_commit();
    create_tag(&repo.repo, "release/1.0", None, None).unwrap();
    assert_eq!(tag_names(&repo), ["release/1.0"]);
}

#[test]
fn creating_an_existing_tag_fails_and_keeps_the_old_one() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    create_tag(&repo.repo, "v1", None, None).unwrap();
    repo.commit_file("a.txt", "a\n", "second");

    for message in [None, Some("annotated this time")] {
        let err = create_tag(&repo.repo, "v1", None, message).unwrap_err();
        assert!(matches!(err, GitError::TagAlreadyExists(ref n) if n == "v1"), "{err:?}");
    }
    assert_eq!(find(&repo, "v1").target, id(first), "the old tag was not moved");
    assert!(!find(&repo, "v1").annotated);
}

#[test]
fn invalid_tag_names_are_rejected() {
    let repo = TestRepo::with_initial_commit();
    for name in ["", "a b", "a..b", "x.lock", "a~b", "a:b", "a^b", "a?b", "a*b", "a[b", "a//b", "a\\b", "/a", "a/"] {
        let err = create_tag(&repo.repo, name, None, None).unwrap_err();
        assert!(matches!(err, GitError::InvalidName(ref n) if n == name), "{name:?}: {err:?}");
    }
    assert!(tag_names(&repo).is_empty());
}

#[test]
fn a_tag_on_an_unknown_revision_fails() {
    let repo = TestRepo::with_initial_commit();
    for start in ["nope", "HEAD~9", &"0".repeat(40)] {
        let err = create_tag(&repo.repo, "v1", Some(start), None).unwrap_err();
        assert!(matches!(err, GitError::RevisionNotFound(ref s) if s == start), "{start}: {err:?}");
    }
    assert!(tag_names(&repo).is_empty());
}

#[test]
fn a_tag_in_a_repository_without_commits_fails() {
    let repo = TestRepo::init();
    let err = create_tag(&repo.repo, "v1", None, None).unwrap_err();
    assert!(matches!(err, GitError::RevisionNotFound(_)), "{err:?}");
}

#[test]
fn a_blank_message_is_rejected() {
    let repo = TestRepo::with_initial_commit();
    for message in ["", "  ", "\n\t\n"] {
        let err = create_tag(&repo.repo, "v1", None, Some(message)).unwrap_err();
        assert!(matches!(err, GitError::EmptyMessage), "{message:?}: {err:?}");
    }
    assert!(tag_names(&repo).is_empty());
}

#[test]
fn an_annotated_tag_needs_an_identity_but_a_lightweight_one_does_not() {
    let repo = TestRepo::with_initial_commit();
    repo.isolate_config(&[]);

    let err = create_tag(&repo.repo, "v1", None, Some("release")).unwrap_err();
    assert!(matches!(err, GitError::MissingIdentity), "{err:?}");
    assert!(tag_names(&repo).is_empty());

    create_tag(&repo.repo, "v1", None, None).unwrap();
    assert_eq!(tag_names(&repo), ["v1"]);
}

// ============================================================================== delete

#[test]
fn delete_removes_the_tag_but_not_the_commit() {
    let repo = TestRepo::with_initial_commit();
    let commit = head(&repo);
    create_tag(&repo.repo, "light", None, None).unwrap();
    create_tag(&repo.repo, "heavy", None, Some("annotated")).unwrap();

    delete_tag(&repo.repo, "light").unwrap();
    delete_tag(&repo.repo, "heavy").unwrap();
    assert!(tag_names(&repo).is_empty());
    assert!(repo.repo.find_commit(commit).is_ok());
}

#[test]
fn delete_only_touches_the_named_tag() {
    let repo = TestRepo::with_initial_commit();
    for name in ["a", "b", "c"] {
        repo.tag(name);
    }
    delete_tag(&repo.repo, "b").unwrap();
    assert_eq!(tag_names(&repo), ["a", "c"]);
}

#[test]
fn deleting_an_unknown_tag_fails() {
    let repo = TestRepo::with_initial_commit();
    repo.tag("v1");

    for name in ["nope", "", "v1 "] {
        let err = delete_tag(&repo.repo, name).unwrap_err();
        assert!(matches!(err, GitError::TagNotFound(ref n) if n == name), "{name:?}: {err:?}");
    }
    assert_eq!(tag_names(&repo), ["v1"]);
}

#[test]
fn a_deleted_tag_name_can_be_used_again() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    create_tag(&repo.repo, "v1", None, None).unwrap();
    delete_tag(&repo.repo, "v1").unwrap();
    repo.commit_file("a.txt", "a\n", "second");

    create_tag(&repo.repo, "v1", None, None).unwrap();
    assert_ne!(find(&repo, "v1").target, id(first));
}

#[test]
fn tags_survive_a_json_round_trip() {
    let repo = TestRepo::with_initial_commit();
    create_tag(&repo.repo, "light", None, None).unwrap();
    create_tag(&repo.repo, "heavy", None, Some("release")).unwrap();
    let tags = list_tags(&repo.repo).unwrap();

    let json = serde_json::to_string(&tags).unwrap();
    assert_eq!(serde_json::from_str::<Vec<TagInfo>>(&json).unwrap(), tags);
}

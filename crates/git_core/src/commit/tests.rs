//! Tests for `commit` (written first; the implementation lives in `commit/mod.rs`).
//!
//! # API these tests expect
//!
//! ```ignore
//! /// `git commit -m <message>`: commits what is staged and moves the current branch.
//! pub fn commit(repo: &Repository, message: &str) -> GitResult<git2::Oid>;
//! ```
//!
//! # Behaviour the tests pin down
//! - The commit holds exactly the index. Unstaged changes and untracked files stay out of it and
//!   keep showing up in `status`.
//! - Author and committer both come from the repository's git config (`user.name`/`user.email`),
//!   stamped with the current time.
//! - `HEAD`'s branch moves to the new commit (a detached `HEAD` stays detached and moves). The
//!   first commit of a repository has no parent.
//! - The stored message is `message` without trailing whitespace plus one `\n`. A message that is
//!   empty or only whitespace is rejected.
//! - Nothing is committed (and nothing changes) when:
//!   - `GitError::EmptyMessage`: the message is blank
//!   - `GitError::NothingToCommit`: the index equals `HEAD` (or is empty on an unborn branch)
//!   - `GitError::MissingIdentity`: `user.name` or `user.email` is not configured
//!   - `GitError::UnresolvedConflicts`: the index still has conflicts. This is reported before
//!     `NothingToCommit`.
//! - Finishing a merge: after the conflicts are resolved and staged, the commit gets two parents
//!   (`HEAD` and `MERGE_HEAD`) and the repository leaves the merge state.
use super::*;
use crate::{
    error::GitError,
    status::{ChangeKind, status},
    tests::TestRepo,
};
use git2::{Oid, RepositoryState};
use std::path::Path;

fn head_oid(repo: &TestRepo) -> Option<Oid> {
    repo.repo.head().ok().and_then(|h| h.target())
}

/// Contents of `path` inside the tree of commit `oid`; `None` if the commit has no such file.
fn committed(repo: &TestRepo, oid: Oid, path: &str) -> Option<String> {
    let tree = repo.repo.find_commit(oid).unwrap().tree().unwrap();
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.repo.find_blob(entry.id()).unwrap();
    Some(String::from_utf8(blob.content().to_vec()).unwrap())
}

fn message_of(repo: &TestRepo, oid: Oid) -> String {
    repo.repo
        .find_commit(oid)
        .unwrap()
        .message()
        .unwrap()
        .to_owned()
}

// ================================================================ what gets committed

#[test]
fn commit_records_the_staged_content() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "hello\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "add a").unwrap();
    assert_eq!(committed(&repo, oid, "a.txt").as_deref(), Some("hello\n"));
    assert_eq!(
        committed(&repo, oid, "README.md").as_deref(),
        Some("# test\n"),
        "old files stay"
    );
}

#[test]
fn commit_returns_the_new_head() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "add a").unwrap();
    assert_eq!(head_oid(&repo), Some(oid));
}

#[test]
fn first_commit_has_no_parent() {
    let repo = TestRepo::init();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "root").unwrap();
    assert_eq!(repo.repo.find_commit(oid).unwrap().parent_count(), 0);
    assert_eq!(head_oid(&repo), Some(oid));
}

#[test]
fn next_commit_has_the_previous_head_as_its_parent() {
    let repo = TestRepo::with_initial_commit();
    let before = head_oid(&repo).unwrap();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "add a").unwrap();
    let parents: Vec<_> = repo.repo.find_commit(oid).unwrap().parent_ids().collect();
    assert_eq!(parents, [before]);
}

#[test]
fn commit_moves_the_current_branch() {
    let repo = TestRepo::with_initial_commit();
    let branch = repo.repo.head().unwrap().name().unwrap().to_owned();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "add a").unwrap();
    let head = repo.repo.head().unwrap();
    assert_eq!(head.name().unwrap(), branch, "still on the same branch");
    assert_eq!(head.target(), Some(oid));
}

#[test]
fn commit_on_a_detached_head_stays_detached() {
    let repo = TestRepo::with_initial_commit();
    repo.repo
        .set_head_detached(head_oid(&repo).unwrap())
        .unwrap();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "on detached head").unwrap();
    assert!(repo.repo.head_detached().unwrap());
    assert_eq!(head_oid(&repo), Some(oid));
}

#[test]
fn only_staged_changes_are_committed() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a1\n", "a");
    repo.commit_file("b.txt", "b1\n", "b");
    repo.write("a.txt", "a2\n");
    repo.write("b.txt", "b2\n");
    repo.write("c.txt", "c\n");
    repo.stage("a.txt");

    let oid = commit(&repo.repo, "only a").unwrap();
    assert_eq!(committed(&repo, oid, "a.txt").as_deref(), Some("a2\n"));
    assert_eq!(
        committed(&repo, oid, "b.txt").as_deref(),
        Some("b1\n"),
        "b was not staged"
    );
    assert_eq!(committed(&repo, oid, "c.txt"), None, "c was never added");

    // the rest is still pending
    let pending: Vec<_> = status(&repo.repo)
        .unwrap()
        .into_iter()
        .map(|e| (e.path.to_string_lossy().into_owned(), e.staged, e.unstaged))
        .collect();
    assert_eq!(
        pending,
        [
            ("b.txt".to_owned(), None, Some(ChangeKind::Modified)),
            ("c.txt".to_owned(), None, Some(ChangeKind::Untracked)),
        ]
    );
}

#[test]
fn staged_edits_made_after_staging_are_not_committed() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "staged version\n");
    repo.stage("README.md");
    repo.write("README.md", "staged version\nedited later\n");

    let oid = commit(&repo.repo, "snapshot").unwrap();
    assert_eq!(
        committed(&repo, oid, "README.md").as_deref(),
        Some("staged version\n")
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("README.md")).unwrap(),
        "staged version\nedited later\n",
        "the working tree is never touched"
    );
}

#[test]
fn staged_deletion_removes_the_file_from_the_commit() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    repo.stage_all();

    let oid = commit(&repo.repo, "remove readme").unwrap();
    assert_eq!(committed(&repo, oid, "README.md"), None);
}

#[test]
fn committing_everything_leaves_a_clean_status() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.write("dir/new.txt", "x\n");
    repo.stage_all();

    commit(&repo.repo, "save").unwrap();
    assert!(status(&repo.repo).unwrap().is_empty());
}

// ======================================================================= author + message

#[test]
fn author_and_committer_come_from_the_git_config() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let oid = commit(&repo.repo, "add a").unwrap();
    let c = repo.repo.find_commit(oid).unwrap();
    for sig in [c.author(), c.committer()] {
        assert_eq!(sig.name().unwrap(), "Test User");
        assert_eq!(sig.email().unwrap(), "test@example.com");
        assert!(
            (sig.when().seconds() - now).abs() < 60,
            "timestamp is current"
        );
    }
}

#[test]
fn message_gets_a_single_trailing_newline() {
    let repo = TestRepo::with_initial_commit();
    for (i, (given, stored)) in [
        ("subject", "subject\n"),
        ("subject\n", "subject\n"),
        ("subject  \n\n\n", "subject\n"),
        ("  indented subject", "  indented subject\n"),
    ]
    .into_iter()
    .enumerate()
    {
        repo.write("a.txt", &format!("{i}\n"));
        repo.stage("a.txt");
        let oid = commit(&repo.repo, given).unwrap();
        assert_eq!(message_of(&repo, oid), stored, "given {given:?}");
    }
}

#[test]
fn multi_line_and_unicode_messages_are_kept() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let message = "feat: café ☕\n\nBody line one.\nBody line two.\n\nCloses #12";
    let oid = commit(&repo.repo, message).unwrap();
    assert_eq!(message_of(&repo, oid), format!("{message}\n"));
}

#[test]
fn blank_message_is_rejected_and_nothing_is_committed() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");
    let before = head_oid(&repo);

    for message in ["", " ", "\n", " \t\n  \n"] {
        let err = commit(&repo.repo, message).unwrap_err();
        assert!(
            matches!(err, GitError::EmptyMessage),
            "{message:?}: {err:?}"
        );
    }
    assert_eq!(head_oid(&repo), before);
    assert_eq!(
        status(&repo.repo).unwrap()[0].staged,
        Some(ChangeKind::Added),
        "still staged"
    );
}

// ============================================================================ nothing to commit

#[test]
fn nothing_staged_is_nothing_to_commit() {
    let repo = TestRepo::with_initial_commit();
    let before = head_oid(&repo);

    let err = commit(&repo.repo, "empty").unwrap_err();
    assert!(matches!(err, GitError::NothingToCommit), "{err:?}");
    assert_eq!(head_oid(&repo), before);
}

#[test]
fn only_unstaged_changes_is_nothing_to_commit() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.write("new.txt", "x\n");

    let err = commit(&repo.repo, "nothing staged").unwrap_err();
    assert!(matches!(err, GitError::NothingToCommit), "{err:?}");
}

#[test]
fn empty_repository_with_nothing_staged_is_nothing_to_commit() {
    let repo = TestRepo::init();

    let err = commit(&repo.repo, "first").unwrap_err();
    assert!(matches!(err, GitError::NothingToCommit), "{err:?}");
    assert_eq!(head_oid(&repo), None, "the branch is still unborn");
}

#[test]
fn staged_content_equal_to_head_is_nothing_to_commit() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "temporary\n");
    repo.stage("README.md");
    repo.write("README.md", "# test\n"); // back to what HEAD has
    repo.stage("README.md");

    let err = commit(&repo.repo, "no real change").unwrap_err();
    assert!(matches!(err, GitError::NothingToCommit), "{err:?}");
}

#[test]
fn committing_twice_in_a_row_fails_the_second_time() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");
    let first = commit(&repo.repo, "add a").unwrap();

    let err = commit(&repo.repo, "again").unwrap_err();
    assert!(matches!(err, GitError::NothingToCommit), "{err:?}");
    assert_eq!(head_oid(&repo), Some(first));
}

// ================================================================================== identity

#[test]
fn missing_name_is_missing_identity() {
    let repo = TestRepo::init();
    repo.isolate_config(&[("user.email", "test@example.com")]);
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let err = commit(&repo.repo, "no name").unwrap_err();
    assert!(matches!(err, GitError::MissingIdentity), "{err:?}");
    assert_eq!(head_oid(&repo), None);
}

#[test]
fn missing_email_is_missing_identity() {
    let repo = TestRepo::init();
    repo.isolate_config(&[("user.name", "Test User")]);
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let err = commit(&repo.repo, "no email").unwrap_err();
    assert!(matches!(err, GitError::MissingIdentity), "{err:?}");
}

#[test]
fn no_identity_at_all_is_missing_identity() {
    let repo = TestRepo::init();
    repo.isolate_config(&[]);
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let err = commit(&repo.repo, "nobody").unwrap_err();
    assert!(matches!(err, GitError::MissingIdentity), "{err:?}");
}

// ====================================================================================== merges

#[test]
fn unresolved_conflicts_block_the_commit() {
    let repo = TestRepo::init();
    repo.make_conflict();
    let before = head_oid(&repo);

    let err = commit(&repo.repo, "merge").unwrap_err();
    assert!(matches!(err, GitError::UnresolvedConflicts), "{err:?}");
    assert_eq!(head_oid(&repo), before);
    assert_eq!(repo.repo.state(), RepositoryState::Merge, "still merging");
}

#[test]
fn committing_a_resolved_merge_creates_a_merge_commit() {
    let repo = TestRepo::init();
    repo.make_conflict();
    let ours = head_oid(&repo).unwrap();
    let theirs = repo
        .repo
        .find_reference("MERGE_HEAD")
        .unwrap()
        .target()
        .unwrap();
    repo.write("c.txt", "resolved\n");
    repo.stage("c.txt");

    let oid = commit(&repo.repo, "Merge feature").unwrap();
    let parents: Vec<_> = repo.repo.find_commit(oid).unwrap().parent_ids().collect();
    assert_eq!(
        parents,
        [ours, theirs],
        "HEAD first, then the merged commit"
    );
    assert_eq!(
        committed(&repo, oid, "c.txt").as_deref(),
        Some("resolved\n")
    );
    assert_eq!(
        repo.repo.state(),
        RepositoryState::Clean,
        "merge state is cleaned up"
    );
    assert!(status(&repo.repo).unwrap().is_empty());
}

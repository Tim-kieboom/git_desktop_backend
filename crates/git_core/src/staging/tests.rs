//! Tests for `stage` / `unstage` (written first; the implementation lives in `staging/mod.rs`).
//!
//! # API these tests expect
//!
//! ```ignore
//! /// `git add <path>`: stages the file's current state (new, modified or deleted).
//! pub fn stage(repo: &Repository, path: &Path) -> Result<(), GitError>;
//!
//! /// `git restore --staged <path>`: puts the index entry back to `HEAD`, working tree untouched.
//! pub fn unstage(repo: &Repository, path: &Path) -> Result<(), GitError>;
//! ```
//!
//! # Behaviour the tests pin down
//! - `path` is relative to the working directory, one **file** (not a directory), matched
//!   **literally** (`a[1].txt` is just that file, not a glob).
//! - Both are idempotent: staging an already staged / unchanged file, or unstaging a file that
//!   isn't staged, is `Ok` and changes nothing.
//! - A path that is in `HEAD`, in the index, or on disk is "known". Anything else is
//!   `GitError::PathNotFound`.
//! - Absolute paths and directories are rejected with an error (any `GitError`).
//! - `stage` also stages deletions and marks a merge conflict as resolved.
//! - Neither function ever modifies files in the working tree.
use super::*;
use crate::{
    diff::{DiffTarget, file_diff},
    error::GitError,
    status::{ChangeKind, StatusEntry, status},
    test_utils::TestRepo,
};
use std::path::Path;

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn entry(repo: &TestRepo, path: &str) -> Option<StatusEntry> {
    status(&repo.repo)
        .unwrap()
        .into_iter()
        .find(|e| &*e.path == Path::new(path))
}

/// `(staged, unstaged)` of a file; `(None, None)` when it has no status entry (clean).
fn state(repo: &TestRepo, path: &str) -> (Option<ChangeKind>, Option<ChangeKind>) {
    entry(repo, path).map_or((None, None), |e| (e.staged, e.unstaged))
}

fn read(repo: &TestRepo, path: &str) -> String {
    std::fs::read_to_string(repo.path().join(path)).unwrap()
}

const ADDED: Option<ChangeKind> = Some(ChangeKind::Added);
const MODIFIED: Option<ChangeKind> = Some(ChangeKind::Modified);
const DELETED: Option<ChangeKind> = Some(ChangeKind::Deleted);
const UNTRACKED: Option<ChangeKind> = Some(ChangeKind::Untracked);

// ====================================================================== stage

#[test]
fn stage_untracked_file_makes_it_staged_added() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");

    stage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "new.txt"), (ADDED, None));
}

#[test]
fn stage_untracked_file_before_first_commit() {
    let repo = TestRepo::init();
    repo.write("first.txt", "x\n");

    stage(&repo.repo, Path::new("first.txt")).unwrap();
    assert_eq!(state(&repo, "first.txt"), (ADDED, None));
}

#[test]
fn stage_modified_file() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");

    stage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (MODIFIED, None));
    assert_eq!(
        read(&repo, "README.md"),
        "changed\n",
        "working tree untouched"
    );
}

#[test]
fn stage_deleted_file_stages_the_deletion() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");

    stage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (DELETED, None));
    assert!(!repo.path().join("README.md").exists());
}

#[test]
fn stage_file_in_subdirectory() {
    let repo = TestRepo::with_initial_commit();
    repo.write("src/deep/new.txt", "x\n");

    stage(&repo.repo, Path::new("src/deep/new.txt")).unwrap();
    assert_eq!(state(&repo, "src/deep/new.txt"), (ADDED, None));
}

#[test]
fn stage_only_touches_the_given_file() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a\n", "a");
    repo.commit_file("b.txt", "b\n", "b");
    repo.write("a.txt", "A\n");
    repo.write("b.txt", "B\n");
    repo.write("c.txt", "c\n");

    stage(&repo.repo, Path::new("a.txt")).unwrap();
    assert_eq!(state(&repo, "a.txt"), (MODIFIED, None));
    assert_eq!(state(&repo, "b.txt"), (None, MODIFIED));
    assert_eq!(state(&repo, "c.txt"), (None, UNTRACKED));
}

#[test]
fn stage_unchanged_tracked_file_is_a_noop() {
    let repo = TestRepo::with_initial_commit();

    stage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, None));
}

#[test]
fn staging_twice_is_a_noop() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");

    stage(&repo.repo, Path::new("new.txt")).unwrap();
    stage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "new.txt"), (ADDED, None));
}

#[test]
fn staging_again_picks_up_later_edits() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "first\n");
    stage(&repo.repo, Path::new("README.md")).unwrap();
    repo.write("README.md", "first\nsecond\n");
    assert_eq!(state(&repo, "README.md"), (MODIFIED, MODIFIED));

    stage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (MODIFIED, None));

    let staged = file_diff(&repo.repo, DiffTarget::Staged, Path::new("README.md"))
        .unwrap()
        .unwrap();
    assert!(staged.patch.contains("+second"), "{}", staged.patch);
    assert!(
        file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("README.md"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn staging_a_file_deleted_after_it_was_staged_removes_it_from_the_index() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    stage(&repo.repo, Path::new("new.txt")).unwrap();
    repo.remove("new.txt");
    assert_eq!(state(&repo, "new.txt"), (ADDED, DELETED));

    stage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(
        state(&repo, "new.txt"),
        (None, None),
        "never existed in HEAD: no trace left"
    );
}

#[test]
fn staging_an_already_staged_deletion_is_a_noop() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    stage(&repo.repo, Path::new("README.md")).unwrap();

    // gone from disk and from the index, but still in HEAD: known path, nothing to do
    stage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (DELETED, None));
}

#[test]
fn staging_both_halves_of_a_move_shows_a_rename() {
    let repo = TestRepo::init();
    repo.commit_file("old.txt", &numbered(20), "add");
    repo.remove("old.txt");
    repo.write("new.txt", &numbered(20));

    stage(&repo.repo, Path::new("old.txt")).unwrap();
    stage(&repo.repo, Path::new("new.txt")).unwrap();

    let all = status(&repo.repo).unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].staged, Some(ChangeKind::Renamed));
    assert_eq!(all[0].old_path.as_deref(), Some(Path::new("old.txt")));
}

#[test]
fn staging_a_conflicted_file_resolves_the_conflict() {
    let repo = TestRepo::init();
    repo.make_conflict();
    assert!(entry(&repo, "c.txt").unwrap().conflicted);

    repo.write("c.txt", "resolved\n");
    stage(&repo.repo, Path::new("c.txt")).unwrap();
    assert!(!entry(&repo, "c.txt").unwrap().conflicted);
    assert!(!repo.repo.index().unwrap().has_conflicts());
}

#[test]
fn stage_unknown_path_is_path_not_found() {
    let repo = TestRepo::with_initial_commit();

    let err = stage(&repo.repo, Path::new("nope.txt")).unwrap_err();
    assert!(
        matches!(err, GitError::PathNotFound(ref p) if p == Path::new("nope.txt")),
        "{err:?}"
    );
}

#[test]
fn stage_directory_is_rejected() {
    let repo = TestRepo::with_initial_commit();
    repo.write("dir/a.txt", "a\n");

    assert!(stage(&repo.repo, Path::new("dir")).is_err());
    assert_eq!(
        state(&repo, "dir/a.txt"),
        (None, UNTRACKED),
        "nothing was staged"
    );
}

#[test]
fn stage_absolute_path_is_rejected() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");

    assert!(stage(&repo.repo, &repo.path().join("new.txt")).is_err());
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
}

#[test]
fn stage_matches_the_path_literally_not_as_a_glob() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", "x\n", "add");
    repo.commit_file("a1.txt", "x\n", "add");
    repo.write("a[1].txt", "y\n");
    repo.write("a1.txt", "y\n");

    stage(&repo.repo, Path::new("a[1].txt")).unwrap();
    assert_eq!(state(&repo, "a[1].txt"), (MODIFIED, None));
    assert_eq!(
        state(&repo, "a1.txt"),
        (None, MODIFIED),
        "a1.txt must not be staged"
    );
}

#[cfg(unix)]
#[test]
fn stage_non_utf8_file_name() {
    use std::os::unix::ffi::OsStrExt;
    let repo = TestRepo::with_initial_commit();
    let name = std::ffi::OsStr::from_bytes(b"bad-\xff\xfe.txt");
    std::fs::write(repo.path().join(name), "x\n").unwrap();

    stage(&repo.repo, Path::new(name)).unwrap();
    let all = status(&repo.repo).unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].staged, ADDED);
}

// ==================================================================== unstage

#[test]
fn unstage_new_file_makes_it_untracked_again() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    stage(&repo.repo, Path::new("new.txt")).unwrap();

    unstage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
    assert_eq!(read(&repo, "new.txt"), "x\n", "the file stays on disk");
}

#[test]
fn unstage_new_file_before_first_commit() {
    let repo = TestRepo::init();
    repo.write("first.txt", "x\n");
    stage(&repo.repo, Path::new("first.txt")).unwrap();

    unstage(&repo.repo, Path::new("first.txt")).unwrap();
    assert_eq!(state(&repo, "first.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_modified_file_keeps_the_working_tree_change() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    stage(&repo.repo, Path::new("README.md")).unwrap();

    unstage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, MODIFIED));
    assert_eq!(read(&repo, "README.md"), "changed\n");
}

#[test]
fn unstage_staged_deletion_keeps_the_file_deleted() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    stage(&repo.repo, Path::new("README.md")).unwrap();

    unstage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, DELETED));
    assert!(!repo.path().join("README.md").exists());
}

#[test]
fn unstage_partially_staged_file_keeps_the_later_edits() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "staged\n");
    stage(&repo.repo, Path::new("README.md")).unwrap();
    repo.write("README.md", "staged\nand more\n");

    unstage(&repo.repo, Path::new("README.md")).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, MODIFIED));
    assert_eq!(read(&repo, "README.md"), "staged\nand more\n");
}

#[test]
fn unstage_file_that_is_not_staged_is_a_noop() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    repo.write("README.md", "changed\n");

    unstage(&repo.repo, Path::new("README.md")).unwrap();
    unstage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, MODIFIED));
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_clean_file_is_a_noop() {
    let repo = TestRepo::with_initial_commit();

    unstage(&repo.repo, Path::new("README.md")).unwrap();
    assert!(status(&repo.repo).unwrap().is_empty());
}

#[test]
fn unstaging_twice_is_a_noop() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    stage(&repo.repo, Path::new("new.txt")).unwrap();

    unstage(&repo.repo, Path::new("new.txt")).unwrap();
    unstage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_only_touches_the_given_file() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a\n", "a");
    repo.commit_file("b.txt", "b\n", "b");
    repo.write("a.txt", "A\n");
    repo.write("b.txt", "B\n");
    stage(&repo.repo, Path::new("a.txt")).unwrap();
    stage(&repo.repo, Path::new("b.txt")).unwrap();

    unstage(&repo.repo, Path::new("a.txt")).unwrap();
    assert_eq!(state(&repo, "a.txt"), (None, MODIFIED));
    assert_eq!(state(&repo, "b.txt"), (MODIFIED, None));
}

#[test]
fn unstaging_one_half_of_a_rename_leaves_the_other_staged() {
    let repo = TestRepo::init();
    repo.commit_file("old.txt", &numbered(20), "add");
    repo.remove("old.txt");
    repo.write("new.txt", &numbered(20));
    repo.stage_all();

    unstage(&repo.repo, Path::new("new.txt")).unwrap();
    assert_eq!(state(&repo, "old.txt"), (DELETED, None));
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_unknown_path_is_path_not_found() {
    let repo = TestRepo::with_initial_commit();

    let err = unstage(&repo.repo, Path::new("nope.txt")).unwrap_err();
    assert!(
        matches!(err, GitError::PathNotFound(ref p) if p == Path::new("nope.txt")),
        "{err:?}"
    );
}

#[test]
fn unstage_directory_is_rejected() {
    let repo = TestRepo::with_initial_commit();
    repo.write("dir/a.txt", "a\n");
    stage(&repo.repo, Path::new("dir/a.txt")).unwrap();

    assert!(unstage(&repo.repo, Path::new("dir")).is_err());
    assert_eq!(
        state(&repo, "dir/a.txt"),
        (ADDED, None),
        "nothing was unstaged"
    );
}

#[test]
fn unstage_absolute_path_is_rejected() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    stage(&repo.repo, Path::new("new.txt")).unwrap();

    assert!(unstage(&repo.repo, &repo.path().join("new.txt")).is_err());
    assert_eq!(state(&repo, "new.txt"), (ADDED, None));
}

#[test]
fn unstage_matches_the_path_literally_not_as_a_glob() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", "x\n", "add");
    repo.commit_file("a1.txt", "x\n", "add");
    repo.write("a[1].txt", "y\n");
    repo.write("a1.txt", "y\n");
    repo.stage_all();

    unstage(&repo.repo, Path::new("a[1].txt")).unwrap();
    assert_eq!(state(&repo, "a[1].txt"), (None, MODIFIED));
    assert_eq!(
        state(&repo, "a1.txt"),
        (MODIFIED, None),
        "a1.txt must stay staged"
    );
}

#[cfg(unix)]
#[test]
fn unstage_non_utf8_file_name() {
    use std::os::unix::ffi::OsStrExt;
    let repo = TestRepo::with_initial_commit();
    let name = std::ffi::OsStr::from_bytes(b"bad-\xff\xfe.txt");
    std::fs::write(repo.path().join(name), "x\n").unwrap();
    stage(&repo.repo, Path::new(name)).unwrap();

    unstage(&repo.repo, Path::new(name)).unwrap();
    let all = status(&repo.repo).unwrap();
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!((all[0].staged, all[0].unstaged), (None, UNTRACKED));
}

// ============================================================ stage + unstage

#[test]
fn stage_then_unstage_restores_the_original_status() {
    let repo = TestRepo::init();
    repo.commit_file("modified.txt", "a\n", "a");
    repo.commit_file("deleted.txt", "b\n", "b");
    repo.write("modified.txt", "changed\n");
    repo.remove("deleted.txt");
    repo.write("untracked.txt", "u\n");
    let before = status(&repo.repo).unwrap();

    for path in ["modified.txt", "deleted.txt", "untracked.txt"] {
        stage(&repo.repo, Path::new(path)).unwrap();
    }
    assert_ne!(status(&repo.repo).unwrap(), before);
    for path in ["modified.txt", "deleted.txt", "untracked.txt"] {
        unstage(&repo.repo, Path::new(path)).unwrap();
    }

    assert_eq!(status(&repo.repo).unwrap(), before);
}

#[test]
fn staged_changes_end_up_in_the_next_commit() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "a\n");
    repo.write("b.txt", "b\n");
    stage(&repo.repo, Path::new("a.txt")).unwrap();
    repo.commit("only a");

    assert_eq!(state(&repo, "a.txt"), (None, None));
    assert_eq!(state(&repo, "b.txt"), (None, UNTRACKED));
}

//! Tests for `stage_hunk` / `unstage_hunk`.
//!
//! Most tests use a 30 line file `f.txt` with two changes far enough apart (line 3 and line 28)
//! to form two separate hunks. `build(a, b)` is the file with change `a` at line 3 and change `b`
//! at line 28, so the expected index content after staging one hunk is just `build(a, None)`.
use super::*;
use crate::{
    commit::commit,
    diff::{DiffTarget, FileStatus, file_diff},
    error::GitError,
    status::{ChangeKind, status},
    tests::TestRepo,
};
use std::path::Path;

const PATH: &str = "f.txt";

#[derive(Clone, Copy, PartialEq)]
enum Change {
    None,
    Modify,
    /// Three new lines after the line.
    Insert,
    /// The line is removed.
    Delete,
}
use Change::{Delete, Insert, Modify};
const NONE: Change = Change::None;

/// The 30 line file with change `at3` applied to line 3 and `at28` to line 28.
fn build(at3: Change, at28: Change) -> String {
    let mut out = Vec::new();
    for i in 1..=30 {
        let change = match i {
            3 => at3,
            28 => at28,
            _ => NONE,
        };
        match change {
            Delete => continue,
            Modify => out.push(format!("CHANGED {i}")),
            _ => out.push(format!("line {i}")),
        }
        if change == Insert {
            out.extend(["added a", "added b", "added c"].map(String::from));
        }
    }
    out.join("\n") + "\n"
}

/// `f.txt` committed as `build(NONE, NONE)` and edited in the working tree to `build(at3, at28)`.
fn setup(at3: Change, at28: Change) -> TestRepo {
    let repo = TestRepo::init();
    repo.commit_file(PATH, &build(NONE, NONE), "base");
    repo.write(PATH, &build(at3, at28));
    repo
}

fn hunks(repo: &TestRepo, target: DiffTarget, path: &str) -> Vec<Hunk> {
    file_diff(&repo.repo, target, Path::new(path))
        .unwrap()
        .map(|d| d.hunks)
        .unwrap_or_default()
}

fn staged(repo: &TestRepo) -> Vec<Hunk> {
    hunks(repo, DiffTarget::Staged, PATH)
}

fn unstaged(repo: &TestRepo) -> Vec<Hunk> {
    hunks(repo, DiffTarget::Unstaged, PATH)
}

/// Content of `path` in the index, read through a fresh `Repository` so it proves the index was
/// written to disk.
fn index_text(repo: &TestRepo, path: &str) -> String {
    let fresh = git2::Repository::open(repo.path()).unwrap();
    let index = fresh.index().unwrap();
    let entry = index.get_path(Path::new(path), 0).expect("not in the index");
    String::from_utf8(fresh.find_blob(entry.id).unwrap().content().to_vec()).unwrap()
}

fn disk_text(repo: &TestRepo, path: &str) -> String {
    std::fs::read_to_string(repo.path().join(path)).unwrap()
}

/// `(staged, unstaged)` of a file; `(None, None)` when it has no status entry.
fn state(repo: &TestRepo, path: &str) -> (Option<ChangeKind>, Option<ChangeKind>) {
    status(&repo.repo)
        .unwrap()
        .into_iter()
        .find(|e| &*e.path == Path::new(path))
        .map_or((None, None), |e| (e.staged, e.unstaged))
}

const ADDED: Option<ChangeKind> = Some(ChangeKind::Added);
const MODIFIED: Option<ChangeKind> = Some(ChangeKind::Modified);
const DELETED: Option<ChangeKind> = Some(ChangeKind::Deleted);
const UNTRACKED: Option<ChangeKind> = Some(ChangeKind::Untracked);

fn is_hunk_not_found(err: &GitError) -> bool {
    matches!(err, GitError::HunkNotFound(_))
}

// ================================================================================ stage_hunk

#[test]
fn stage_hunk_stages_only_the_first_hunk() {
    let repo = setup(Modify, Modify);
    let all = unstaged(&repo);
    assert_eq!(all.len(), 2);

    stage_hunk(&repo.repo, Path::new(PATH), &all[0]).unwrap();

    assert_eq!(index_text(&repo, PATH), build(Modify, NONE));
    assert_eq!(staged(&repo), [all[0].clone()]);
    assert_eq!(unstaged(&repo), [all[1].clone()]);
    assert_eq!(state(&repo, PATH), (MODIFIED, MODIFIED));
}

#[test]
fn stage_hunk_stages_only_the_second_hunk() {
    let repo = setup(Modify, Modify);
    let all = unstaged(&repo);

    stage_hunk(&repo.repo, Path::new(PATH), &all[1]).unwrap();

    assert_eq!(index_text(&repo, PATH), build(NONE, Modify));
    assert_eq!(staged(&repo), [all[1].clone()]);
    assert_eq!(unstaged(&repo), [all[0].clone()]);
}

#[test]
fn stage_hunk_never_modifies_the_working_tree() {
    let repo = setup(Modify, Modify);
    let first = unstaged(&repo).remove(0);

    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();
    assert_eq!(disk_text(&repo, PATH), build(Modify, Modify));
}

#[test]
fn staging_every_hunk_equals_staging_the_whole_file() {
    let repo = setup(Modify, Modify);

    while let Some(hunk) = unstaged(&repo).first().cloned() {
        stage_hunk(&repo.repo, Path::new(PATH), &hunk).unwrap();
    }

    assert_eq!(index_text(&repo, PATH), build(Modify, Modify));
    assert_eq!(state(&repo, PATH), (MODIFIED, None));
}

#[test]
fn hunks_can_be_staged_in_any_order() {
    let repo = setup(Modify, Modify);
    let second = unstaged(&repo).remove(1);
    stage_hunk(&repo.repo, Path::new(PATH), &second).unwrap();

    let first = unstaged(&repo).remove(0);
    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();
    assert_eq!(index_text(&repo, PATH), build(Modify, Modify));
}

#[test]
fn insertion_and_deletion_hunks_are_staged_independently() {
    let repo = setup(Insert, Delete);
    let all = unstaged(&repo);
    assert_eq!(all.len(), 2);

    stage_hunk(&repo.repo, Path::new(PATH), &all[1]).unwrap(); // the deletion
    assert_eq!(index_text(&repo, PATH), build(NONE, Delete));

    let insertion = unstaged(&repo).remove(0);
    stage_hunk(&repo.repo, Path::new(PATH), &insertion).unwrap();
    assert_eq!(index_text(&repo, PATH), build(Insert, Delete));
}

#[test]
fn staging_an_insertion_shifts_the_positions_of_later_hunks() {
    let repo = setup(Insert, Modify);
    let before = unstaged(&repo);
    stage_hunk(&repo.repo, Path::new(PATH), &before[0]).unwrap();
    assert_eq!(index_text(&repo, PATH), build(Insert, NONE));

    // the old copy of the second hunk points at lines that moved: it is stale ...
    let err = stage_hunk(&repo.repo, Path::new(PATH), &before[1]).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
    assert_eq!(index_text(&repo, PATH), build(Insert, NONE), "nothing changed");

    // ... and a freshly fetched one works
    let fresh = unstaged(&repo).remove(0);
    stage_hunk(&repo.repo, Path::new(PATH), &fresh).unwrap();
    assert_eq!(index_text(&repo, PATH), build(Insert, Modify));
}

#[test]
fn a_hunk_that_changed_since_the_diff_is_rejected() {
    let repo = setup(Modify, Modify);
    let first = unstaged(&repo).remove(0);
    let mut edited = build(Modify, Modify);
    edited = edited.replace("CHANGED 3\n", "SOMETHING ELSE 3\n");
    repo.write(PATH, &edited);

    let err = stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
    assert_eq!(index_text(&repo, PATH), build(NONE, NONE), "nothing was staged");
    assert_eq!(disk_text(&repo, PATH), edited);
}

#[test]
fn a_hunk_that_is_already_staged_is_rejected() {
    let repo = setup(Modify, Modify);
    let first = unstaged(&repo).remove(0);
    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();

    let err = stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
    assert_eq!(index_text(&repo, PATH), build(Modify, NONE));
}

#[test]
fn a_hunk_of_a_file_without_changes_is_rejected() {
    let repo = setup(Modify, Modify);
    let first = unstaged(&repo).remove(0);
    repo.write(PATH, &build(NONE, NONE)); // reverted

    let err = stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
}

#[test]
fn a_hunk_of_another_file_is_rejected() {
    let repo = setup(Modify, Modify);
    repo.commit_file("other.txt", "one\ntwo\n", "other");
    repo.write("other.txt", "one\nTWO\n");
    let foreign = hunks(&repo, DiffTarget::Unstaged, "other.txt").remove(0);

    let err = stage_hunk(&repo.repo, Path::new(PATH), &foreign).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
}

#[test]
fn stage_hunk_of_an_untracked_file_stages_the_whole_file() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "one\ntwo\n");
    let hunk = hunks(&repo, DiffTarget::Unstaged, "new.txt").remove(0);

    stage_hunk(&repo.repo, Path::new("new.txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "new.txt"), (ADDED, None));
    assert_eq!(index_text(&repo, "new.txt"), "one\ntwo\n");
}

#[test]
fn stage_hunk_of_a_new_file_before_the_first_commit() {
    let repo = TestRepo::init();
    repo.write("first.txt", "x\n");
    let hunk = hunks(&repo, DiffTarget::Unstaged, "first.txt").remove(0);

    stage_hunk(&repo.repo, Path::new("first.txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "first.txt"), (ADDED, None));
}

#[test]
fn stage_hunk_of_a_deleted_file_stages_the_deletion() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    let hunk = hunks(&repo, DiffTarget::Unstaged, "README.md").remove(0);

    stage_hunk(&repo.repo, Path::new("README.md"), &hunk).unwrap();
    assert_eq!(state(&repo, "README.md"), (DELETED, None));
}

#[test]
fn stage_hunk_works_without_a_final_newline() {
    let repo = TestRepo::init();
    repo.commit_file(PATH, "a\nb\nc", "base");
    repo.write(PATH, "a\nb\nc\nd");
    let hunk = unstaged(&repo).remove(0);

    stage_hunk(&repo.repo, Path::new(PATH), &hunk).unwrap();
    assert_eq!(index_text(&repo, PATH), "a\nb\nc\nd");
    assert!(unstaged(&repo).is_empty());
}

#[test]
fn stage_hunk_keeps_windows_line_endings() {
    let crlf = |text: String| text.replace('\n', "\r\n");
    let repo = TestRepo::init();
    repo.commit_file(PATH, &crlf(build(NONE, NONE)), "base");
    repo.write(PATH, &crlf(build(Modify, Modify)));
    let first = unstaged(&repo).remove(0);

    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();
    assert_eq!(index_text(&repo, PATH), crlf(build(Modify, NONE)));
}

#[test]
fn stage_hunk_in_a_subdirectory() {
    let repo = TestRepo::init();
    repo.commit_file("src/deep/f.txt", &build(NONE, NONE), "base");
    repo.write("src/deep/f.txt", &build(Modify, Modify));
    let first = hunks(&repo, DiffTarget::Unstaged, "src/deep/f.txt").remove(0);

    stage_hunk(&repo.repo, Path::new("src/deep/f.txt"), &first).unwrap();
    assert_eq!(index_text(&repo, "src/deep/f.txt"), build(Modify, NONE));
}

#[test]
fn stage_hunk_matches_the_path_literally_not_as_a_glob() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", &build(NONE, NONE), "a");
    repo.commit_file("a1.txt", &build(NONE, NONE), "a1");
    repo.write("a[1].txt", &build(Modify, NONE));
    repo.write("a1.txt", &build(Modify, NONE));
    let hunk = hunks(&repo, DiffTarget::Unstaged, "a[1].txt").remove(0);

    stage_hunk(&repo.repo, Path::new("a[1].txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "a[1].txt"), (MODIFIED, None));
    assert_eq!(state(&repo, "a1.txt"), (None, MODIFIED), "a1.txt must not be touched");
}

#[test]
fn stage_hunk_rejects_bad_paths() {
    let repo = setup(Modify, Modify);
    let hunk = unstaged(&repo).remove(0);
    repo.write("dir/x.txt", "x\n");

    let err = stage_hunk(&repo.repo, Path::new("nope.txt"), &hunk).unwrap_err();
    assert!(matches!(err, GitError::PathNotFound(_)), "{err:?}");
    let err = stage_hunk(&repo.repo, Path::new("dir"), &hunk).unwrap_err();
    assert!(matches!(err, GitError::InvalidPath(_)), "{err:?}");
    let err = stage_hunk(&repo.repo, &repo.path().join(PATH), &hunk).unwrap_err();
    assert!(matches!(err, GitError::InvalidPath(_)), "{err:?}");
    assert_eq!(index_text(&repo, PATH), build(NONE, NONE));
}

#[test]
fn a_staged_hunk_is_what_gets_committed() {
    let repo = setup(Modify, Modify);
    let first = unstaged(&repo).remove(0);
    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();

    let id = commit(&repo.repo, "only the first change").unwrap();
    let tree = repo.repo.find_commit(id).unwrap().tree().unwrap();
    let blob = repo.repo.find_blob(tree.get_path(Path::new(PATH)).unwrap().id()).unwrap();
    assert_eq!(String::from_utf8_lossy(blob.content()), build(Modify, NONE));

    // the second change is still pending
    assert_eq!(state(&repo, PATH), (None, MODIFIED));
    assert_eq!(unstaged(&repo).len(), 1);
}

// ============================================================================== unstage_hunk

#[test]
fn unstage_hunk_unstages_only_the_first_hunk() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);
    let all = staged(&repo);
    assert_eq!(all.len(), 2);

    unstage_hunk(&repo.repo, Path::new(PATH), &all[0]).unwrap();

    assert_eq!(index_text(&repo, PATH), build(NONE, Modify));
    assert_eq!(staged(&repo), [all[1].clone()]);
    assert_eq!(unstaged(&repo), [all[0].clone()]);
    assert_eq!(state(&repo, PATH), (MODIFIED, MODIFIED));
    assert_eq!(disk_text(&repo, PATH), build(Modify, Modify), "working tree untouched");
}

#[test]
fn unstage_hunk_unstages_only_the_second_hunk() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);
    let all = staged(&repo);

    unstage_hunk(&repo.repo, Path::new(PATH), &all[1]).unwrap();

    assert_eq!(index_text(&repo, PATH), build(Modify, NONE));
    assert_eq!(staged(&repo), [all[0].clone()]);
    assert_eq!(unstaged(&repo), [all[1].clone()]);
}

#[test]
fn unstaging_every_hunk_equals_unstaging_the_whole_file() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);

    while let Some(hunk) = staged(&repo).first().cloned() {
        unstage_hunk(&repo.repo, Path::new(PATH), &hunk).unwrap();
    }

    assert_eq!(index_text(&repo, PATH), build(NONE, NONE));
    assert_eq!(state(&repo, PATH), (None, MODIFIED));
}

#[test]
fn stage_hunk_then_unstage_hunk_restores_the_original_state() {
    let repo = setup(Modify, Modify);
    let original = unstaged(&repo);
    let first = original[0].clone();

    stage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();
    let just_staged = staged(&repo).remove(0);
    unstage_hunk(&repo.repo, Path::new(PATH), &just_staged).unwrap();

    assert_eq!(index_text(&repo, PATH), build(NONE, NONE));
    assert_eq!(unstaged(&repo), original);
    assert!(staged(&repo).is_empty());
}

#[test]
fn unstage_hunk_handles_insertions_and_deletions() {
    let repo = setup(Insert, Delete);
    repo.stage(PATH);
    let all = staged(&repo);
    assert_eq!(all.len(), 2);

    unstage_hunk(&repo.repo, Path::new(PATH), &all[1]).unwrap(); // the deletion
    assert_eq!(index_text(&repo, PATH), build(Insert, NONE));

    let insertion = staged(&repo).remove(0);
    unstage_hunk(&repo.repo, Path::new(PATH), &insertion).unwrap();
    assert_eq!(index_text(&repo, PATH), build(NONE, NONE));
}

#[test]
fn unstaging_an_early_hunk_keeps_a_later_staged_hunk_applicable() {
    let repo = setup(Insert, Modify);
    repo.stage(PATH);
    let all = staged(&repo);

    unstage_hunk(&repo.repo, Path::new(PATH), &all[0]).unwrap(); // the insertion
    assert_eq!(index_text(&repo, PATH), build(NONE, Modify));

    let rest = staged(&repo).remove(0);
    unstage_hunk(&repo.repo, Path::new(PATH), &rest).unwrap();
    assert_eq!(index_text(&repo, PATH), build(NONE, NONE));
}

#[test]
fn unstage_hunk_of_a_staged_new_file_unstages_the_whole_file() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "one\ntwo\n");
    repo.stage("new.txt");
    let hunk = hunks(&repo, DiffTarget::Staged, "new.txt").remove(0);

    unstage_hunk(&repo.repo, Path::new("new.txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "new.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_hunk_of_a_new_file_before_the_first_commit() {
    let repo = TestRepo::init();
    repo.write("first.txt", "x\n");
    repo.stage("first.txt");
    let hunk = hunks(&repo, DiffTarget::Staged, "first.txt").remove(0);

    unstage_hunk(&repo.repo, Path::new("first.txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "first.txt"), (None, UNTRACKED));
}

#[test]
fn unstage_hunk_of_a_staged_deletion_keeps_the_file_deleted() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    repo.stage_all();
    let hunk = hunks(&repo, DiffTarget::Staged, "README.md").remove(0);

    unstage_hunk(&repo.repo, Path::new("README.md"), &hunk).unwrap();
    assert_eq!(state(&repo, "README.md"), (None, DELETED));
}

#[test]
fn a_staged_hunk_that_changed_since_the_diff_is_rejected() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);
    let first = staged(&repo).remove(0);
    let edited = build(Modify, Modify).replace("CHANGED 3\n", "SOMETHING ELSE 3\n");
    repo.write(PATH, &edited);
    repo.stage(PATH); // the staged hunk now has other content

    let err = unstage_hunk(&repo.repo, Path::new(PATH), &first).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
    assert_eq!(index_text(&repo, PATH), edited, "nothing was unstaged");
}

#[test]
fn a_hunk_that_is_not_staged_cannot_be_unstaged() {
    let repo = setup(Modify, Modify);
    let not_staged = unstaged(&repo).remove(0);

    let err = unstage_hunk(&repo.repo, Path::new(PATH), &not_staged).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
}

#[test]
fn unstaging_the_same_hunk_twice_is_rejected() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);
    let first = staged(&repo).remove(0);
    unstage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();

    let err = unstage_hunk(&repo.repo, Path::new(PATH), &first).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
}

#[test]
fn hunks_of_a_renamed_file_are_rejected_without_changing_the_index() {
    let repo = TestRepo::init();
    repo.commit_file("old.txt", &build(NONE, NONE), "base");
    repo.remove("old.txt");
    repo.write("new.txt", &build(Modify, NONE));
    repo.stage_all();
    let renamed = crate::diff::diff_files(&repo.repo, DiffTarget::Staged, None).unwrap();
    assert_eq!(renamed[0].status, FileStatus::Renamed);
    let hunk = renamed[0].hunks[0].clone();

    let err = unstage_hunk(&repo.repo, Path::new("new.txt"), &hunk).unwrap_err();
    assert!(is_hunk_not_found(&err), "{err:?}");
    assert_eq!(index_text(&repo, "new.txt"), build(Modify, NONE));
}

#[test]
fn unstage_hunk_works_without_a_final_newline() {
    let repo = TestRepo::init();
    repo.commit_file(PATH, "a\nb\nc", "base");
    repo.write(PATH, "a\nb\nc\nd");
    repo.stage(PATH);
    let hunk = staged(&repo).remove(0);

    unstage_hunk(&repo.repo, Path::new(PATH), &hunk).unwrap();
    assert_eq!(index_text(&repo, PATH), "a\nb\nc");
    assert!(staged(&repo).is_empty());
}

#[test]
fn unstage_hunk_keeps_windows_line_endings() {
    let crlf = |text: String| text.replace('\n', "\r\n");
    let repo = TestRepo::init();
    repo.commit_file(PATH, &crlf(build(NONE, NONE)), "base");
    repo.write(PATH, &crlf(build(Modify, Modify)));
    repo.stage(PATH);
    let first = staged(&repo).remove(0);

    unstage_hunk(&repo.repo, Path::new(PATH), &first).unwrap();
    assert_eq!(index_text(&repo, PATH), crlf(build(NONE, Modify)));
}

#[test]
fn unstage_hunk_matches_the_path_literally_not_as_a_glob() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", &build(NONE, NONE), "a");
    repo.commit_file("a1.txt", &build(NONE, NONE), "a1");
    repo.write("a[1].txt", &build(Modify, NONE));
    repo.write("a1.txt", &build(Modify, NONE));
    repo.stage_all();
    let hunk = hunks(&repo, DiffTarget::Staged, "a[1].txt").remove(0);

    unstage_hunk(&repo.repo, Path::new("a[1].txt"), &hunk).unwrap();
    assert_eq!(state(&repo, "a[1].txt"), (None, MODIFIED));
    assert_eq!(state(&repo, "a1.txt"), (MODIFIED, None), "a1.txt must stay staged");
}

#[test]
fn unstage_hunk_rejects_bad_paths() {
    let repo = setup(Modify, Modify);
    repo.stage(PATH);
    let hunk = staged(&repo).remove(0);
    repo.write("dir/x.txt", "x\n");

    let err = unstage_hunk(&repo.repo, Path::new("nope.txt"), &hunk).unwrap_err();
    assert!(matches!(err, GitError::PathNotFound(_)), "{err:?}");
    let err = unstage_hunk(&repo.repo, Path::new("dir"), &hunk).unwrap_err();
    assert!(matches!(err, GitError::InvalidPath(_)), "{err:?}");
    let err = unstage_hunk(&repo.repo, &repo.path().join(PATH), &hunk).unwrap_err();
    assert!(matches!(err, GitError::InvalidPath(_)), "{err:?}");
    assert_eq!(index_text(&repo, PATH), build(Modify, Modify));
}

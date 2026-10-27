//! Tests for `status` (written first; the implementation lives in `status/mod.rs`).
//!
//! # API these tests expect
//!
//! ```ignore
//! /// Working tree status: one entry per changed file, sorted by path. Clean repo => empty vec.
//! pub fn status(repo: &git2::Repository) -> Result<Vec<StatusEntry>, GitError>;
//!
//! #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
//! pub struct StatusEntry {
//!     pub path: PathBuf,             // relative to the working directory
//!     pub old_path: Option<PathBuf>, // only for staged renames
//!     pub staged: Option<ChangeKind>,   // HEAD -> index
//!     pub unstaged: Option<ChangeKind>, // index -> working tree
//!     pub conflicted: bool,          // unresolved merge conflict
//! }
//!
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
//! pub enum ChangeKind { Added, Modified, Deleted, Renamed, Untracked }
//! //   Untracked is only ever used for `unstaged`.
//!
//! ```
//!
//! Wire it up with `pub mod status;` in `lib.rs` and `#[cfg(test)] mod tests;` in `status/mod.rs`.
use super::*;
use crate::test_utils::TestRepo;
use std::path::{Path, PathBuf};

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn entries(repo: &TestRepo) -> Vec<StatusEntry> {
    match status(&repo.repo) {
        Ok(val) => val,
        Err(err) => panic!("{err}"),
    }
}

fn entry_paths(repo: &TestRepo) -> Vec<PathBuf> {
    entries(repo)
        .into_iter()
        .map(|e| e.path.to_path_buf())
        .collect()
}

/// The single entry for `path`; panics (with all entries) if missing.
fn entry(repo: &TestRepo, path: &str) -> StatusEntry {
    let all = entries(repo);
    all.iter()
        .find(|e| *e.path == *Path::new(path))
        .unwrap_or_else(|| panic!("no entry for {path} in {all:#?}"))
        .clone()
}

// ---------------------------------------------------------------- clean / empty

#[test]
fn clean_repo_has_empty_status() {
    let repo = TestRepo::with_initial_commit();
    assert!(entries(&repo).is_empty());
}

#[test]
fn fresh_repo_without_commits_has_empty_status() {
    let repo = TestRepo::init();
    assert!(entries(&repo).is_empty());
}

// ---------------------------------------------------------------- untracked

#[test]
fn untracked_file_is_unstaged_untracked() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");

    let e = entry(&repo, "new.txt");
    assert_eq!(e.staged, None);
    assert_eq!(e.unstaged, Some(ChangeKind::Untracked));
    assert!(!e.conflicted);
    assert_eq!(e.old_path, None);
}

#[test]
fn untracked_files_in_new_directories_are_listed_individually() {
    let repo = TestRepo::with_initial_commit();
    repo.write("dir/a.txt", "a\n");
    repo.write("dir/deep/b.txt", "b\n");

    let paths: Vec<_> = entry_paths(&repo);

    assert_eq!(paths, [Path::new("dir/a.txt"), Path::new("dir/deep/b.txt")]);
}

#[test]
fn empty_directories_are_not_listed() {
    let repo = TestRepo::with_initial_commit();
    std::fs::create_dir_all(repo.path().join("empty/dir")).unwrap();
    assert!(entries(&repo).is_empty());
}

#[test]
fn ignored_files_are_not_listed() {
    let repo = TestRepo::init();
    repo.commit_file(".gitignore", "*.log\nbuild/\n", "ignore");
    repo.write("debug.log", "noise\n");
    repo.write("build/out.bin", "noise\n");
    repo.write("kept.txt", "x\n");

    let paths: Vec<_> = entry_paths(&repo);

    assert_eq!(paths, [Path::new("kept.txt")]);
}

// ---------------------------------------------------------------- unstaged changes

#[test]
fn modified_tracked_file_is_unstaged_modified() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");

    let e = entry(&repo, "README.md");
    assert_eq!(e.staged, None);
    assert_eq!(e.unstaged, Some(ChangeKind::Modified));
}

#[test]
fn deleted_tracked_file_is_unstaged_deleted() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");

    let e = entry(&repo, "README.md");
    assert_eq!(e.staged, None);
    assert_eq!(e.unstaged, Some(ChangeKind::Deleted));
}

// ---------------------------------------------------------------- staged changes

#[test]
fn newly_staged_file_is_staged_added() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "x\n");
    repo.stage("new.txt");

    let e = entry(&repo, "new.txt");
    assert_eq!(e.staged, Some(ChangeKind::Added));
    assert_eq!(e.unstaged, None);
}

#[test]
fn staged_file_before_first_commit_is_staged_added() {
    let repo = TestRepo::init();
    repo.write("first.txt", "x\n");
    repo.stage("first.txt");

    let e = entry(&repo, "first.txt");
    assert_eq!(e.staged, Some(ChangeKind::Added));
    assert_eq!(e.unstaged, None);
}

#[test]
fn staged_modification_is_staged_modified() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.stage("README.md");

    let e = entry(&repo, "README.md");
    assert_eq!(e.staged, Some(ChangeKind::Modified));
    assert_eq!(e.unstaged, None);
}

#[test]
fn staged_deletion_is_staged_deleted() {
    let repo = TestRepo::with_initial_commit();
    repo.remove("README.md");
    repo.stage_all();

    let e = entry(&repo, "README.md");
    assert_eq!(e.staged, Some(ChangeKind::Deleted));
    assert_eq!(e.unstaged, None);
}

#[test]
fn staged_rename_reports_old_path() {
    let repo = TestRepo::init();
    repo.commit_file("old.txt", &numbered(20), "add");
    repo.remove("old.txt");
    repo.write("new.txt", &numbered(20));
    repo.stage_all();

    let all = entries(&repo);
    assert_eq!(
        all.len(),
        1,
        "a rename is one entry, not delete + add: {all:#?}"
    );
    assert_eq!(*all[0].path, *Path::new("new.txt"));
    assert_eq!(all[0].old_path.as_deref(), Some(Path::new("old.txt")));
    assert_eq!(all[0].staged, Some(ChangeKind::Renamed));
    assert_eq!(all[0].unstaged, None);
}

// ---------------------------------------------------------------- both sides at once

#[test]
fn partially_staged_file_has_both_staged_and_unstaged() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "staged version\n");
    repo.stage("README.md");
    repo.write("README.md", "staged version\nplus more\n");

    let e = entry(&repo, "README.md");
    assert_eq!(e.staged, Some(ChangeKind::Modified));
    assert_eq!(e.unstaged, Some(ChangeKind::Modified));
}

#[test]
fn staged_new_file_modified_afterwards_is_added_and_modified() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "v1\n");
    repo.stage("new.txt");
    repo.write("new.txt", "v2\n");

    let e = entry(&repo, "new.txt");
    assert_eq!(e.staged, Some(ChangeKind::Added));
    assert_eq!(e.unstaged, Some(ChangeKind::Modified));
}

#[test]
fn staged_new_file_deleted_afterwards_is_added_and_deleted() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "v1\n");
    repo.stage("new.txt");
    repo.remove("new.txt");

    let e = entry(&repo, "new.txt");
    assert_eq!(e.staged, Some(ChangeKind::Added));
    assert_eq!(e.unstaged, Some(ChangeKind::Deleted));
}

// ---------------------------------------------------------------- conflicts

#[test]
fn merge_conflict_is_flagged_as_conflicted() {
    let repo = TestRepo::init();
    repo.make_conflict();

    let e = entry(&repo, "c.txt");
    assert!(e.conflicted, "{e:#?}");
}

#[test]
fn non_conflicted_files_are_not_flagged_during_a_conflict() {
    let repo = TestRepo::init();
    repo.commit_file("other.txt", "o\n", "other");
    repo.make_conflict();
    repo.write("untouched.txt", "u\n");

    assert!(!entry(&repo, "untouched.txt").conflicted);
    assert_eq!(entries(&repo).iter().filter(|e| e.conflicted).count(), 1);
}

// ---------------------------------------------------------------- listing behaviour

#[test]
fn entries_are_sorted_by_path_and_unique() {
    let repo = TestRepo::with_initial_commit();
    repo.write("b.txt", "b\n");
    repo.write("a\\z.txt", "z\n");
    repo.write("a.txt", "a\n");
    repo.write("README.md", "changed\n");
    repo.stage("README.md");
    repo.write("README.md", "changed again\n"); // staged + unstaged must still be ONE entry

    // `PathBuf` ordering compares component by component, so `a/z.txt` sorts before `a.txt`
    // (plain `sort_by_key(|e| e.path.clone())` gives exactly this).
    let paths: Vec<_> = entry_paths(&repo);
    let mut expect = [
        Path::new("README.md"),
        Path::new("a.txt"),
        Path::new("a\\z.txt"),
        Path::new("b.txt"),
    ];
    expect.sort();
    assert_eq!(paths, expect);
}

#[test]
fn paths_with_spaces_and_unicode_are_reported_verbatim() {
    let repo = TestRepo::with_initial_commit();
    repo.write("my file.txt", "x\n");
    repo.write("café/naïve.txt", "x\n");

    let paths: Vec<_> = entry_paths(&repo);
    assert_eq!(
        paths,
        [Path::new("café/naïve.txt"), Path::new("my file.txt")]
    );
}

/// The entry's path as a `&Path`, whatever pointer type the field uses (`PathBuf`, `Arc<Path>`, ...).
fn path_of(entry: &StatusEntry) -> &Path {
    &entry.path
}

/// A file name that is not valid UTF-8 (git allows any bytes except `/` and NUL on Unix).
#[cfg(unix)]
fn non_utf8_name() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(b"bad-\xff\xfe-name.txt".to_vec())
}

#[cfg(unix)]
#[test]
fn non_utf8_file_name_does_not_break_status() {
    let repo = TestRepo::with_initial_commit();
    std::fs::write(repo.path().join(non_utf8_name()), "x\n").unwrap();
    repo.write("normal.txt", "x\n");
    assert!(std::str::from_utf8(non_utf8_name().as_encoded_bytes()).is_err());

    // one odd name must not make the whole listing fail, and the other files still show up
    let all = entries(&repo);
    assert_eq!(all.len(), 2, "{all:#?}");
    assert!(all.iter().any(|e| path_of(e) == Path::new("normal.txt")));
}

#[cfg(unix)]
#[test]
fn non_utf8_file_name_is_reported_byte_for_byte() {
    let repo = TestRepo::with_initial_commit();
    let name = non_utf8_name();
    std::fs::write(repo.path().join(&name), "x\n").unwrap();

    let all = entries(&repo);
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(path_of(&all[0]), Path::new(&name));
    assert_eq!(all[0].unstaged, Some(ChangeKind::Untracked));
}

#[cfg(unix)]
#[test]
fn staged_non_utf8_file_name_is_reported() {
    let repo = TestRepo::with_initial_commit();
    let name = non_utf8_name();
    std::fs::write(repo.path().join(&name), "x\n").unwrap();
    let mut index = repo.repo.index().unwrap();
    index.add_path(Path::new(&name)).unwrap();
    index.write().unwrap();

    let all = entries(&repo);
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(path_of(&all[0]), Path::new(&name));
    assert_eq!(all[0].staged, Some(ChangeKind::Added));
}

#[test]
fn committing_everything_makes_status_clean_again() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.write("new.txt", "x\n");
    assert_eq!(entries(&repo).len(), 2);

    repo.stage_all();
    repo.commit("save work");
    assert!(entries(&repo).is_empty());
}

#[test]
fn status_survives_json_round_trip() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.write("new.txt", "x\n");
    let all = entries(&repo);

    let json = serde_json::to_string(&all).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<StatusEntry>>(&json).unwrap(),
        all
    );
}

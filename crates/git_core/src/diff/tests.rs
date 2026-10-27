use super::*;
use crate::test_utils::TestRepo;

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn clean_repo_has_no_diff() {
    let repo = TestRepo::with_initial_commit();
    assert!(
        diff_files(&repo.repo, DiffTarget::Unstaged, None)
            .unwrap()
            .is_empty()
    );
    assert!(
        diff_files(&repo.repo, DiffTarget::Staged, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn modified_file_has_structured_hunk() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "one\ntwo\nthree\n", "add a");
    repo.write("a.txt", "one\nTWO\nthree\n");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .expect("changed");
    assert_eq!(diff.status, FileStatus::Modified);
    assert_eq!(diff.hunks.len(), 1);

    let hunk = &diff.hunks[0];
    assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
    assert_eq!((hunk.new_start, hunk.new_lines), (1, 3));
    assert!(hunk.header.starts_with("@@ -1,3 +1,3 @@"));

    let kinds: Vec<_> = hunk.lines.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [
            LineKind::Context,
            LineKind::Removed,
            LineKind::Added,
            LineKind::Context
        ]
    );

    let removed = &hunk.lines[1];
    assert_eq!(
        (
            removed.content.as_str(),
            removed.old_lineno,
            removed.new_lineno
        ),
        ("two", Some(2), None)
    );
    let added = &hunk.lines[2];
    assert_eq!(
        (added.content.as_str(), added.old_lineno, added.new_lineno),
        ("TWO", None, Some(2))
    );
}

#[test]
fn patch_text_is_unified_diff() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "one\ntwo\n", "add a");
    repo.write("a.txt", "one\n2\n");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .unwrap();
    assert!(diff.patch.starts_with("diff --git a/a.txt b/a.txt"));
    assert!(diff.patch.contains("\n-two\n"));
    assert!(diff.patch.contains("\n+2\n"));
}

#[test]
fn distant_changes_make_separate_hunks() {
    let repo = TestRepo::init();
    repo.commit_file("big.txt", &numbered(100), "add");
    let changed = numbered(100)
        .replace("line 2\n", "CHANGED 2\n")
        .replace("line 99\n", "CHANGED 99\n");
    repo.write("big.txt", &changed);

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("big.txt"))
        .unwrap()
        .unwrap();
    assert_eq!(diff.hunks.len(), 2);
    assert!(diff.hunks[0].new_start < diff.hunks[1].new_start);
}

#[test]
fn untracked_file_is_added_when_unstaged() {
    let repo = TestRepo::with_initial_commit();
    repo.write("new.txt", "hello\nworld\n");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("new.txt"))
        .unwrap()
        .unwrap();
    assert_eq!(diff.status, FileStatus::Added);
    assert_eq!(diff.hunks[0].lines.len(), 2);
    assert!(
        diff.hunks[0]
            .lines
            .iter()
            .all(|l| l.kind == LineKind::Added)
    );
}

#[test]
fn untracked_file_in_new_directory_is_listed() {
    let repo = TestRepo::with_initial_commit();
    repo.write("sub/deep/new.txt", "x\n");

    let diffs = diff_files(&repo.repo, DiffTarget::Unstaged, None).unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path.as_ref(), Path::new("sub/deep/new.txt"));
}

#[test]
fn staging_moves_change_from_unstaged_to_staged() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    let readme = Path::new("README.md");

    assert!(
        file_diff(&repo.repo, DiffTarget::Unstaged, readme)
            .unwrap()
            .is_some()
    );
    assert!(
        file_diff(&repo.repo, DiffTarget::Staged, readme)
            .unwrap()
            .is_none()
    );

    repo.stage("README.md");
    assert!(
        file_diff(&repo.repo, DiffTarget::Unstaged, readme)
            .unwrap()
            .is_none()
    );
    assert!(
        file_diff(&repo.repo, DiffTarget::Staged, readme)
            .unwrap()
            .is_some()
    );
}

#[test]
fn partially_staged_file_shows_each_side() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "1\n2\n3\n", "add");
    repo.write("a.txt", "1\nstaged\n3\n");
    repo.stage("a.txt");
    repo.write("a.txt", "1\nstaged\nunstaged\n");

    let staged = file_diff(&repo.repo, DiffTarget::Staged, Path::new("a.txt"))
        .unwrap()
        .unwrap();
    let unstaged = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .unwrap();
    assert!(staged.patch.contains("+staged"));
    assert!(!staged.patch.contains("unstaged"));
    assert!(unstaged.patch.contains("+unstaged"));
    assert!(unstaged.patch.contains("-3"));
}

#[test]
fn staged_diff_works_before_first_commit() {
    let repo = TestRepo::init();
    repo.write("first.txt", "hi\n");
    repo.stage("first.txt");

    let diff = file_diff(&repo.repo, DiffTarget::Staged, Path::new("first.txt"))
        .unwrap()
        .unwrap();
    assert_eq!(diff.status, FileStatus::Added);
}

#[test]
fn deleted_file_reports_old_path_and_removed_lines() {
    let repo = TestRepo::init();
    repo.commit_file("gone.txt", "bye\n", "add");
    repo.remove("gone.txt");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("gone.txt"))
        .unwrap()
        .unwrap();
    assert_eq!(diff.status, FileStatus::Deleted);
    assert_eq!(diff.path.as_ref(), Path::new("gone.txt"));
    assert_eq!(diff.hunks[0].lines[0].kind, LineKind::Removed);
}

#[test]
fn staged_rename_is_detected() {
    let repo = TestRepo::init();
    repo.commit_file("old.txt", &numbered(20), "add");
    repo.remove("old.txt");
    repo.write("new.txt", &numbered(20));
    repo.stage_all();

    let diffs = diff_files(&repo.repo, DiffTarget::Staged, None).unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].status, FileStatus::Renamed);
    assert_eq!(diffs[0].path.as_ref(), Path::new("new.txt"));
    assert_eq!(diffs[0].old_path.as_deref(), Some(Path::new("old.txt")));
}

#[test]
fn binary_file_has_no_hunks() {
    let repo = TestRepo::init();
    std::fs::write(repo.path().join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
    repo.stage("blob.bin");
    repo.commit("add blob");
    std::fs::write(repo.path().join("blob.bin"), [0u8, 1, 2, 3, 0, 0, 9]).unwrap();

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("blob.bin"))
        .unwrap()
        .unwrap();
    assert!(diff.is_binary);
    assert!(diff.hunks.is_empty());
}

#[test]
fn missing_newline_at_eof_is_flagged() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "one\ntwo", "add");
    repo.write("a.txt", "one\ntwo\nthree");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .unwrap();
    let lines = &diff.hunks[0].lines;
    assert!(lines.iter().any(|l| l.missing_newline), "{lines:#?}");
    assert!(lines.iter().all(|l| !l.content.ends_with('\n')));
}

#[test]
fn only_the_requested_file_is_returned() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a\n", "a");
    repo.commit_file("b.txt", "b\n", "b");
    repo.write("a.txt", "A\n");
    repo.write("b.txt", "B\n");

    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .unwrap();
    assert_eq!(diff.path.as_ref(), Path::new("a.txt"));
    assert_eq!(
        diff_files(&repo.repo, DiffTarget::Unstaged, None)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn unknown_path_returns_none() {
    let repo = TestRepo::with_initial_commit();
    assert!(
        file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("nope.txt"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn path_with_glob_characters_matches_literally() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", "x\n", "add");
    repo.commit_file("a1.txt", "x\n", "add");
    repo.write("a[1].txt", "y\n");
    repo.write("a1.txt", "y\n");

    let diffs = diff_files(
        &repo.repo,
        DiffTarget::Unstaged,
        Some(Path::new("a[1].txt")),
    )
    .unwrap();
    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].path.as_ref(), Path::new("a[1].txt"));
}

#[test]
fn diff_survives_json_round_trip() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a\n", "a");
    repo.write("a.txt", "b\n");
    let diff = file_diff(&repo.repo, DiffTarget::Unstaged, Path::new("a.txt"))
        .unwrap()
        .unwrap();

    let json = serde_json::to_string(&diff).unwrap();
    assert_eq!(serde_json::from_str::<FileDiff>(&json).unwrap(), diff);
}

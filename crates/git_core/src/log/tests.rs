//! Tests for `log` and `commit_detail`.
//!
//! # Behaviour the tests pin down
//! - Newest first, children before parents. A repository without commits has an empty log.
//! - `start` is any revision (default `HEAD`); `exclude` hides everything reachable from it
//!   (`exclude..start`). Unknown revisions are `GitError::RevisionNotFound`.
//! - `skip` / `limit` paginate over the *matching* commits and `has_more` says whether another
//!   page follows. The default limit is 50.
//! - `path` keeps the commits that changed that file or directory, `author` the commits whose
//!   author name or email contains the text (case-insensitive).
//! - `commit_detail` is the commit plus its diff against the first parent.
use super::*;
use crate::{diff::FileStatus, tests::TestRepo};
use git2::Oid;

fn id(oid: Oid) -> CommitId {
    CommitId::from_oid(oid)
}

fn page(repo: &TestRepo, query: &LogQuery) -> LogPage {
    log(&repo.repo, query).unwrap()
}

fn messages(page: &LogPage) -> Vec<&str> {
    page.commits.iter().map(|c| c.summary.as_str()).collect()
}

fn all(repo: &TestRepo) -> Vec<String> {
    let query = LogQuery { limit: usize::MAX, ..LogQuery::default() };
    page(repo, &query).commits.iter().map(|c| c.summary.clone()).collect()
}

/// `n` commits "commit 1" .. "commit n", each changing `f.txt`.
fn history(n: usize) -> TestRepo {
    let repo = TestRepo::init();
    for i in 1..=n {
        repo.commit_file("f.txt", &format!("{i}\n"), &format!("commit {i}"));
    }
    repo
}

/// A merge commit of `HEAD` and `other`, keeping `HEAD`'s tree.
fn merge(repo: &TestRepo, other: Oid, message: &str) -> Oid {
    let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
    let other = repo.repo.find_commit(other).unwrap();
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    repo.repo
        .commit(Some("HEAD"), &sig, &sig, message, &head.tree().unwrap(), &[&head, &other])
        .unwrap()
}

// ============================================================================== the list itself

#[test]
fn log_of_a_repository_without_commits_is_empty() {
    let repo = TestRepo::init();
    let result = page(&repo, &LogQuery::default());
    assert!(result.commits.is_empty());
    assert!(!result.has_more);
}

#[test]
fn log_lists_the_newest_commit_first() {
    let repo = history(4);
    assert_eq!(all(&repo), ["commit 4", "commit 3", "commit 2", "commit 1"]);
}

#[test]
fn log_info_describes_the_commit() {
    let repo = TestRepo::init();
    repo.write("f.txt", "x\n");
    repo.stage("f.txt");
    let oid = repo.commit_at("Add f\n\nA longer explanation.\nSecond line.\n\n", 1_700_000_000);

    let info = &page(&repo, &LogQuery::default()).commits[0];
    assert_eq!(info.id, id(oid));
    assert!(info.parents.is_empty(), "root commit");
    assert_eq!(info.summary, "Add f");
    assert_eq!(info.message, "Add f\n\nA longer explanation.\nSecond line.");
    assert_eq!(info.author.name, "Test User");
    assert_eq!(info.author.email, "test@example.com");
    assert_eq!(info.author.time.timestamp(), 1_700_000_000);
    assert_eq!(info.committer, info.author);
}

#[test]
fn parents_are_listed_for_normal_and_merge_commits() {
    let repo = history(2);
    let first = repo.repo.head().unwrap().peel_to_commit().unwrap().parent_id(0).unwrap();
    let second = repo.repo.head().unwrap().target().unwrap();
    repo.repo.branch("side", &repo.repo.find_commit(first).unwrap(), false).unwrap();
    let side = repo.repo.find_branch("side", git2::BranchType::Local).unwrap().get().target().unwrap();
    let merged = merge(&repo, side, "merge side");

    let commits = page(&repo, &LogQuery::default()).commits;
    assert_eq!(commits[0].id, id(merged));
    assert_eq!(commits[0].parents, [id(second), id(side)]);
    assert_eq!(commits[1].parents, [id(first)]);
}

#[test]
fn children_always_come_before_their_parents() {
    let repo = history(2);
    let base = repo.repo.head().unwrap().peel_to_commit().unwrap();
    // a commit on a side branch that is created without checking it out
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    let side = repo
        .repo
        .commit(Some("refs/heads/side"), &sig, &sig, "side work", &base.tree().unwrap(), &[&base])
        .unwrap();
    repo.commit_file("g.txt", "g\n", "main work");
    merge(&repo, side, "merge");

    let commits = page(&repo, &LogQuery { limit: 100, ..LogQuery::default() }).commits;
    assert_eq!(commits.len(), 5);
    let position = |oid: CommitId| commits.iter().position(|c| c.id == oid).unwrap();
    for commit in &commits {
        for parent in &commit.parents {
            assert!(position(commit.id) < position(*parent), "{} before its parent", commit.summary);
        }
    }
}

// ================================================================================== pagination

#[test]
fn pages_follow_each_other_without_gaps() {
    let repo = history(25);
    let full = all(&repo);

    let mut collected = Vec::new();
    let mut skip = 0;
    // bounded, so a pagination bug fails the test instead of looping forever
    for _ in 0..10 {
        let result = page(&repo, &LogQuery { skip, limit: 10, ..LogQuery::default() });
        collected.extend(result.commits.iter().map(|c| c.summary.clone()));
        if !result.has_more {
            break;
        }
        skip += 10;
    }
    assert_eq!(collected, full);
    assert_eq!(collected.len(), 25);
}

#[test]
fn has_more_is_false_exactly_when_the_last_commit_is_included() {
    let repo = history(25);
    assert!(page(&repo, &LogQuery { limit: 24, ..LogQuery::default() }).has_more);
    assert!(!page(&repo, &LogQuery { limit: 25, ..LogQuery::default() }).has_more);
    assert!(!page(&repo, &LogQuery { limit: 26, ..LogQuery::default() }).has_more);
    assert!(!page(&repo, &LogQuery { skip: 25, limit: 10, ..LogQuery::default() }).has_more);
    assert!(page(&repo, &LogQuery { skip: 25, limit: 10, ..LogQuery::default() }).commits.is_empty());
}

#[test]
fn skipping_past_the_end_gives_an_empty_page() {
    let repo = history(3);
    let result = page(&repo, &LogQuery { skip: 100, ..LogQuery::default() });
    assert!(result.commits.is_empty() && !result.has_more);
}

#[test]
fn the_default_limit_is_fifty() {
    let repo = history(60);
    let result = page(&repo, &LogQuery::default());
    assert_eq!(result.commits.len(), 50);
    assert!(result.has_more);
    assert_eq!(result.commits[0].summary, "commit 60");
}

#[test]
fn a_limit_of_zero_returns_nothing_but_reports_more() {
    let repo = history(3);
    let result = page(&repo, &LogQuery { limit: 0, ..LogQuery::default() });
    assert!(result.commits.is_empty());
    assert!(result.has_more);
}

// =========================================================================== start and exclude

#[test]
fn start_can_be_a_branch_a_tag_or_a_commit_id() {
    let repo = history(4);
    let second = repo.repo.revparse_single("HEAD~2").unwrap().peel_to_commit().unwrap();
    repo.repo.branch("old", &second, false).unwrap();
    repo.repo.tag_lightweight("v0.1", second.as_object(), false).unwrap();

    for start in ["old", "v0.1", &second.id().to_string(), "HEAD~2"] {
        let query = LogQuery { start: Some(start.to_owned()), limit: 10, ..LogQuery::default() };
        let result = page(&repo, &query);
        assert_eq!(messages(&result), ["commit 2", "commit 1"], "start {start}");
    }
}

#[test]
fn unknown_start_is_revision_not_found() {
    let repo = history(2);
    for start in ["nope", "HEAD~50", "", "a..b", &"0".repeat(40)] {
        let query = LogQuery { start: Some(start.to_owned()), ..LogQuery::default() };
        let err = log(&repo.repo, &query).unwrap_err();
        assert!(matches!(err, GitError::RevisionNotFound(ref s) if s == start), "{start:?}: {err:?}");
    }
}

#[test]
fn exclude_hides_everything_reachable_from_it() {
    let repo = history(5);
    let tagged = repo.repo.revparse_single("HEAD~2").unwrap();
    repo.repo.tag_lightweight("v1", &tagged, false).unwrap();

    let query = LogQuery { exclude: Some("v1".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["commit 5", "commit 4"]);
}

#[test]
fn excluding_the_start_gives_an_empty_log() {
    let repo = history(3);
    let query = LogQuery { exclude: Some("HEAD".into()), ..LogQuery::default() };
    assert!(page(&repo, &query).commits.is_empty());
}

#[test]
fn unknown_exclude_is_revision_not_found() {
    let repo = history(2);
    let query = LogQuery { exclude: Some("nope".into()), ..LogQuery::default() };
    assert!(matches!(log(&repo.repo, &query).unwrap_err(), GitError::RevisionNotFound(_)));
}

// ================================================================================ path filter

/// a.txt: commits 1, 3; b.txt: commits 2, 4; dir/c.txt: commit 5; a.txt deleted in commit 6.
fn files_history() -> TestRepo {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a1\n", "1 add a");
    repo.commit_file("b.txt", "b1\n", "2 add b");
    repo.commit_file("a.txt", "a2\n", "3 change a");
    repo.commit_file("b.txt", "b2\n", "4 change b");
    repo.commit_file("dir/c.txt", "c\n", "5 add c");
    repo.remove("a.txt");
    repo.stage_all();
    repo.commit("6 delete a");
    repo
}

#[test]
fn path_filter_keeps_only_commits_that_touched_the_file() {
    let repo = files_history();
    let query = LogQuery { path: Some("b.txt".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["4 change b", "2 add b"]);
}

#[test]
fn path_filter_includes_the_commit_that_deleted_the_file() {
    let repo = files_history();
    let query = LogQuery { path: Some("a.txt".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["6 delete a", "3 change a", "1 add a"]);
}

#[test]
fn path_filter_works_for_directories() {
    let repo = files_history();
    let query = LogQuery { path: Some("dir".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["5 add c"]);
}

#[test]
fn path_filter_for_an_unknown_path_is_empty() {
    let repo = files_history();
    let query = LogQuery { path: Some("nope.txt".into()), ..LogQuery::default() };
    assert!(page(&repo, &query).commits.is_empty());
}

#[test]
fn path_filter_matches_literally_not_as_a_glob() {
    let repo = TestRepo::init();
    repo.commit_file("a[1].txt", "x\n", "bracket");
    repo.commit_file("a1.txt", "x\n", "plain");

    let query = LogQuery { path: Some("a[1].txt".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["bracket"]);
}

#[test]
fn bad_paths_are_rejected() {
    let repo = files_history();
    for path in ["", "../x", "/abs/x", "a/../../x"] {
        let query = LogQuery { path: Some(path.into()), ..LogQuery::default() };
        let err = log(&repo.repo, &query).unwrap_err();
        assert!(matches!(err, GitError::InvalidPath(_)), "{path:?}: {err:?}");
    }
}

#[test]
fn merge_commits_that_equal_a_parent_at_the_path_are_left_out() {
    // main changes a.txt, side changes b.txt, then side is merged into main
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "a0\n", "base a");
    repo.commit_file("b.txt", "b0\n", "base b");
    let base = repo.repo.head().unwrap().peel_to_commit().unwrap();
    repo.repo.branch("side", &base, false).unwrap();
    let main_ref = repo.repo.head().unwrap().name().unwrap().to_owned();
    repo.commit_file("a.txt", "a1\n", "main changes a");

    repo.repo.set_head("refs/heads/side").unwrap();
    repo.repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    let side = repo.commit_file("b.txt", "b1\n", "side changes b");

    repo.repo.set_head(&main_ref).unwrap();
    repo.repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).unwrap();
    // the merge result has both changes
    repo.write("b.txt", "b1\n");
    repo.stage("b.txt");
    let tree = repo.repo.find_tree(repo.repo.index().unwrap().write_tree().unwrap()).unwrap();
    let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
    let side_commit = repo.repo.find_commit(side).unwrap();
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    repo.repo
        .commit(Some("HEAD"), &sig, &sig, "merge side", &tree, &[&head, &side_commit])
        .unwrap();

    let for_path = |path: &str| {
        let query = LogQuery { path: Some(path.into()), ..LogQuery::default() };
        messages(&page(&repo, &query)).into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(for_path("a.txt"), ["main changes a", "base a"]);
    assert_eq!(for_path("b.txt"), ["side changes b", "base b"]);
}

// ================================================================================ author filter

fn authors_history() -> TestRepo {
    let repo = TestRepo::init();
    repo.commit_file_as("a.txt", "1\n", "alice 1", "Alice Smith", "alice@example.com");
    repo.commit_file_as("b.txt", "1\n", "bob 1", "Bob Jones", "bob@work.org");
    repo.commit_file_as("a.txt", "2\n", "alice 2", "Alice Smith", "alice@example.com");
    repo.commit_file_as("b.txt", "2\n", "bob 2", "Bob Jones", "bob@work.org");
    repo
}

#[test]
fn author_filter_matches_name_or_email_ignoring_case() {
    let repo = authors_history();
    for text in ["alice", "ALICE", "Smith", "alice@example", "@example.com"] {
        let query = LogQuery { author: Some(text.into()), ..LogQuery::default() };
        assert_eq!(messages(&page(&repo, &query)), ["alice 2", "alice 1"], "{text}");
    }
    let query = LogQuery { author: Some("work.org".into()), ..LogQuery::default() };
    assert_eq!(messages(&page(&repo, &query)), ["bob 2", "bob 1"]);
}

#[test]
fn author_filter_without_a_match_is_empty() {
    let repo = authors_history();
    let query = LogQuery { author: Some("carol".into()), ..LogQuery::default() };
    assert!(page(&repo, &query).commits.is_empty());
}

#[test]
fn filters_combine_and_pagination_counts_only_matches() {
    let repo = authors_history();
    let query = LogQuery {
        author: Some("alice".into()),
        path: Some("a.txt".into()),
        ..LogQuery::default()
    };
    assert_eq!(messages(&page(&repo, &query)), ["alice 2", "alice 1"]);

    let second_page = LogQuery { author: Some("bob".into()), skip: 1, limit: 1, ..LogQuery::default() };
    let result = page(&repo, &second_page);
    assert_eq!(messages(&result), ["bob 1"]);
    assert!(!result.has_more);

    let first_page = LogQuery { author: Some("bob".into()), limit: 1, ..LogQuery::default() };
    assert!(page(&repo, &first_page).has_more);
}

#[test]
fn start_exclude_and_filters_work_together() {
    let repo = authors_history();
    let query = LogQuery {
        exclude: Some("HEAD~2".into()),
        author: Some("alice".into()),
        ..LogQuery::default()
    };
    assert_eq!(messages(&page(&repo, &query)), ["alice 2"]);
}

// ================================================================================ commit detail

#[test]
fn detail_lists_the_changed_files_with_their_hunks() {
    let repo = TestRepo::init();
    repo.commit_file("a.txt", "one\ntwo\n", "add a");
    repo.write("a.txt", "one\nTWO\n");
    repo.write("b.txt", "new\n");
    repo.stage_all();
    let oid = repo.commit("change a, add b");

    let detail = commit_detail(&repo.repo, id(oid)).unwrap();
    assert_eq!(detail.info.summary, "change a, add b");
    let paths: Vec<_> = detail.files.iter().map(|f| (f.path.to_str().unwrap(), f.status)).collect();
    assert_eq!(paths, [("a.txt", FileStatus::Modified), ("b.txt", FileStatus::Added)]);

    let a = &detail.files[0];
    assert_eq!(a.hunks.len(), 1);
    assert!(a.patch.contains("-two") && a.patch.contains("+TWO"), "{}", a.patch);
}

#[test]
fn detail_of_the_root_commit_shows_everything_as_added() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a\n");
    repo.write("dir/b.txt", "b\n");
    repo.stage_all();
    let oid = repo.commit("root");

    let detail = commit_detail(&repo.repo, id(oid)).unwrap();
    assert_eq!(detail.files.len(), 2);
    assert!(detail.files.iter().all(|f| f.status == FileStatus::Added));
}

#[test]
fn detail_shows_deletions_and_renames() {
    let repo = TestRepo::init();
    let content: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    repo.commit_file("old.txt", &content, "add");
    repo.commit_file("gone.txt", "bye\n", "add gone");
    repo.remove("old.txt");
    repo.remove("gone.txt");
    repo.write("new.txt", &content);
    repo.stage_all();
    let oid = repo.commit("rename and delete");

    let files = commit_detail(&repo.repo, id(oid)).unwrap().files;
    let status_of = |path: &str| files.iter().find(|f| &*f.path == std::path::Path::new(path)).map(|f| f.status);
    assert_eq!(status_of("new.txt"), Some(FileStatus::Renamed));
    assert_eq!(status_of("gone.txt"), Some(FileStatus::Deleted));
    let renamed = files.iter().find(|f| f.status == FileStatus::Renamed).unwrap();
    assert_eq!(renamed.old_path.as_deref(), Some(std::path::Path::new("old.txt")));
}

#[test]
fn detail_of_a_merge_commit_diffs_against_the_first_parent() {
    let repo = history(2);
    let first = repo.repo.head().unwrap().peel_to_commit().unwrap().parent_id(0).unwrap();
    let merged = merge(&repo, first, "merge older commit");

    let detail = commit_detail(&repo.repo, id(merged)).unwrap();
    assert_eq!(detail.info.parents.len(), 2);
    assert!(detail.files.is_empty(), "same tree as the first parent");
}

#[test]
fn detail_of_an_empty_commit_has_no_files() {
    let repo = history(1);
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    let head = repo.repo.head().unwrap().peel_to_commit().unwrap();
    let empty = repo
        .repo
        .commit(Some("HEAD"), &sig, &sig, "empty", &head.tree().unwrap(), &[&head])
        .unwrap();

    assert!(commit_detail(&repo.repo, id(empty)).unwrap().files.is_empty());
}

#[test]
fn detail_of_an_unknown_commit_is_revision_not_found() {
    let repo = history(1);
    let missing: CommitId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();

    let err = commit_detail(&repo.repo, missing).unwrap_err();
    assert!(matches!(err, GitError::RevisionNotFound(ref s) if *s == missing.to_string()), "{err:?}");
}

#[test]
fn detail_info_equals_the_log_entry() {
    let repo = history(3);
    let entry = page(&repo, &LogQuery::default()).commits.remove(1);
    assert_eq!(commit_detail(&repo.repo, entry.id).unwrap().info, entry);
}

#[test]
fn log_results_survive_a_json_round_trip() {
    let repo = history(3);
    let result = page(&repo, &LogQuery::default());
    let json = serde_json::to_string(&result).unwrap();
    assert_eq!(serde_json::from_str::<LogPage>(&json).unwrap(), result);

    let query = LogQuery { start: Some("v1".into()), path: Some("a".into()), ..LogQuery::default() };
    let json = serde_json::to_string(&query).unwrap();
    assert_eq!(serde_json::from_str::<LogQuery>(&json).unwrap(), query);

    let detail = commit_detail(&repo.repo, result.commits[0].id).unwrap();
    let json = serde_json::to_string(&detail).unwrap();
    assert_eq!(serde_json::from_str::<CommitDetail>(&json).unwrap(), detail);
}

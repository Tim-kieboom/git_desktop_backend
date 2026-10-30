//! Tests for listing, creating, checking out and deleting branches.
//!
//! # Behaviour the tests pin down
//! - `list_branches`: local branches first, then remote ones, each sorted by name; exactly the
//!   checked-out local branch has `is_head` (none on a detached `HEAD`). A repository without
//!   commits has no branches.
//! - `create_branch` makes a local branch at a revision (default `HEAD`) and does not check it
//!   out. Bad names are `InvalidName`, existing ones `BranchAlreadyExists`.
//! - `checkout_branch` moves `HEAD` and the working tree. Uncommitted changes that don't clash
//!   with the switch come along; clashing ones give `WouldOverwriteChanges` and change nothing.
//! - `delete_branch` refuses the checked-out branch, and unmerged ones unless forced.
//! - `ahead_behind` counts the commits each of two revisions has that the other lacks;
//!   `BranchInfo::ahead_behind` is that comparison between a branch and its upstream.
use super::*;
use crate::tests::TestRepo;
use git2::Oid;

fn id(oid: Oid) -> CommitId {
    CommitId::from_oid(oid)
}

fn head(repo: &TestRepo) -> Oid {
    repo.repo.head().unwrap().target().unwrap()
}

/// The checked-out branch's short name.
fn current(repo: &TestRepo) -> String {
    repo.repo.head().unwrap().shorthand().unwrap().to_owned()
}

fn names(repo: &TestRepo) -> Vec<String> {
    list_branches(&repo.repo).unwrap().into_iter().map(|b| b.name).collect()
}

fn disk(repo: &TestRepo, path: &str) -> Option<String> {
    std::fs::read_to_string(repo.path().join(path)).ok()
}

/// A repo on its default branch with a commit, plus `other` which has one extra commit that adds
/// `only-on-other.txt` and changes `shared.txt`.
fn repo_with_other_branch() -> TestRepo {
    let repo = TestRepo::init();
    repo.commit_file("shared.txt", "base\n", "base");
    repo.commit_file("untouched.txt", "same\n", "second");
    create_branch(&repo.repo, "other", None).unwrap();
    checkout_branch(&repo.repo, "other").unwrap();
    repo.write("only-on-other.txt", "other\n");
    repo.write("shared.txt", "changed on other\n");
    repo.stage_all();
    repo.commit("work on other");
    let default = list_branches(&repo.repo).unwrap().into_iter().find(|b| b.name != "other").unwrap();
    checkout_branch(&repo.repo, &default.name).unwrap();
    repo
}

// ====================================================================================== list

#[test]
fn a_repository_without_commits_has_no_branches() {
    assert!(list_branches(&TestRepo::init().repo).unwrap().is_empty());
}

#[test]
fn a_fresh_repository_has_one_checked_out_branch() {
    let repo = TestRepo::with_initial_commit();

    let branches = list_branches(&repo.repo).unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].name, current(&repo));
    assert_eq!(branches[0].kind, BranchKind::Local);
    assert!(branches[0].is_head);
    assert_eq!(branches[0].target, id(head(&repo)));
    assert_eq!(branches[0].upstream, None);
}

#[test]
fn branches_are_sorted_and_only_one_is_checked_out() {
    let repo = TestRepo::with_initial_commit();
    for name in ["zeta", "alpha", "feature/login", "beta"] {
        create_branch(&repo.repo, name, None).unwrap();
    }

    let branches = list_branches(&repo.repo).unwrap();
    let listed: Vec<_> = branches.iter().map(|b| b.name.clone()).collect();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);
    assert_eq!(listed.len(), 5);
    assert_eq!(branches.iter().filter(|b| b.is_head).count(), 1);
    assert!(branches.iter().find(|b| b.is_head).unwrap().name == current(&repo));
}

#[test]
fn remote_branches_come_after_local_ones() {
    let repo = TestRepo::with_initial_commit();
    let oid = head(&repo);
    create_branch(&repo.repo, "zzz-local", None).unwrap();
    repo.repo.reference("refs/remotes/origin/main", oid, true, "test").unwrap();
    repo.repo.reference("refs/remotes/origin/HEAD", oid, true, "test").ok();

    let branches = list_branches(&repo.repo).unwrap();
    let kinds: Vec<_> = branches.iter().map(|b| (b.name.as_str(), b.kind, b.is_head)).collect();
    let local = current(&repo);
    assert_eq!(
        kinds,
        [
            (local.as_str(), BranchKind::Local, true),
            ("zzz-local", BranchKind::Local, false),
            ("origin/main", BranchKind::Remote, false),
        ]
    );
}

#[test]
fn no_branch_is_checked_out_on_a_detached_head() {
    let repo = TestRepo::with_initial_commit();
    repo.repo.set_head_detached(head(&repo)).unwrap();

    let branches = list_branches(&repo.repo).unwrap();
    assert_eq!(branches.len(), 1);
    assert!(!branches[0].is_head);
}

#[test]
fn upstream_is_reported_for_tracking_branches() {
    let repo = TestRepo::with_initial_commit();
    let main = current(&repo);
    create_branch(&repo.repo, "feature", None).unwrap();
    repo.repo
        .find_branch("feature", git2::BranchType::Local)
        .unwrap()
        .set_upstream(Some(&main))
        .unwrap();

    let branches = list_branches(&repo.repo).unwrap();
    let upstream_of = |name: &str| branches.iter().find(|b| b.name == name).unwrap().upstream.clone();
    assert_eq!(upstream_of("feature"), Some(main.clone()));
    assert_eq!(upstream_of(&main), None);
}

// ==================================================================================== create

#[test]
fn create_branch_points_at_head_by_default_and_is_not_checked_out() {
    let repo = TestRepo::with_initial_commit();
    let before = current(&repo);

    let info = create_branch(&repo.repo, "feature", None).unwrap();
    assert_eq!(info.name, "feature");
    assert_eq!(info.kind, BranchKind::Local);
    assert_eq!(info.target, id(head(&repo)));
    assert!(!info.is_head);

    assert_eq!(current(&repo), before, "HEAD did not move");
    assert!(names(&repo).contains(&"feature".to_owned()));
}

#[test]
fn create_branch_at_another_revision() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    let first_object = repo.repo.find_object(first, None).unwrap();
    repo.repo.tag_lightweight("v1", &first_object, false).unwrap();
    repo.commit_file("a.txt", "a\n", "second");
    create_branch(&repo.repo, "from-id", Some(&first.to_string())).unwrap();
    create_branch(&repo.repo, "from-tag", Some("v1")).unwrap();
    create_branch(&repo.repo, "from-parent", Some("HEAD~1")).unwrap();
    create_branch(&repo.repo, "from-branch", Some(&current(&repo))).unwrap();

    let target = |name: &str| list_branches(&repo.repo).unwrap().into_iter().find(|b| b.name == name).unwrap().target;
    assert_eq!(target("from-id"), id(first));
    assert_eq!(target("from-tag"), id(first));
    assert_eq!(target("from-parent"), id(first));
    assert_eq!(target("from-branch"), id(head(&repo)));
}

#[test]
fn branch_names_may_contain_slashes() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "feature/login/oauth", None).unwrap();
    assert!(names(&repo).contains(&"feature/login/oauth".to_owned()));
}

#[test]
fn creating_an_existing_branch_fails_and_keeps_it() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "feature", None).unwrap();
    repo.commit_file("a.txt", "a\n", "second");

    let err = create_branch(&repo.repo, "feature", None).unwrap_err();
    assert!(matches!(err, GitError::BranchAlreadyExists(ref n) if n == "feature"), "{err:?}");
    let existing = create_branch(&repo.repo, &current(&repo), None).unwrap_err();
    assert!(matches!(existing, GitError::BranchAlreadyExists(_)), "{existing:?}");

    let feature = list_branches(&repo.repo).unwrap().into_iter().find(|b| b.name == "feature").unwrap();
    assert_ne!(feature.target, id(head(&repo)), "the old branch was not moved");
}

#[test]
fn invalid_branch_names_are_rejected() {
    let repo = TestRepo::with_initial_commit();
    for name in ["", "a b", "a..b", "-x", "feature/", "x.lock", "a~b", "a:b", "a^b", "a?b", "a*b", "a[b", "HEAD", "a//b", "a\\b"] {
        let err = create_branch(&repo.repo, name, None).unwrap_err();
        assert!(matches!(err, GitError::InvalidName(ref n) if n == name), "{name:?}: {err:?}");
    }
    assert_eq!(names(&repo).len(), 1, "nothing was created");
}

#[test]
fn create_branch_at_an_unknown_revision_fails() {
    let repo = TestRepo::with_initial_commit();
    for start in ["nope", "HEAD~9", &"0".repeat(40)] {
        let err = create_branch(&repo.repo, "feature", Some(start)).unwrap_err();
        assert!(matches!(err, GitError::RevisionNotFound(ref s) if s == start), "{start}: {err:?}");
    }
    assert_eq!(names(&repo).len(), 1);
}

#[test]
fn create_branch_without_any_commits_fails() {
    let repo = TestRepo::init();
    let err = create_branch(&repo.repo, "feature", None).unwrap_err();
    assert!(matches!(err, GitError::RevisionNotFound(_)), "{err:?}");
}

// ================================================================================== checkout

#[test]
fn checkout_switches_head_and_the_working_tree() {
    let repo = repo_with_other_branch();
    let main = current(&repo);
    assert_eq!(disk(&repo, "only-on-other.txt"), None);
    assert_eq!(disk(&repo, "shared.txt").as_deref(), Some("base\n"));

    checkout_branch(&repo.repo, "other").unwrap();
    assert_eq!(current(&repo), "other");
    assert_eq!(disk(&repo, "only-on-other.txt").as_deref(), Some("other\n"));
    assert_eq!(disk(&repo, "shared.txt").as_deref(), Some("changed on other\n"));
    assert!(list_branches(&repo.repo).unwrap().iter().find(|b| b.name == "other").unwrap().is_head);

    checkout_branch(&repo.repo, &main).unwrap();
    assert_eq!(current(&repo), main);
    assert_eq!(disk(&repo, "only-on-other.txt"), None, "files of the other branch are removed");
    assert_eq!(disk(&repo, "shared.txt").as_deref(), Some("base\n"));
}

#[test]
fn checkout_leaves_a_clean_status() {
    let repo = repo_with_other_branch();
    checkout_branch(&repo.repo, "other").unwrap();
    assert!(crate::status::status(&repo.repo).unwrap().is_empty());
}

#[test]
fn checking_out_the_current_branch_does_nothing() {
    let repo = repo_with_other_branch();
    let main = current(&repo);
    repo.write("untouched.txt", "edited\n");

    checkout_branch(&repo.repo, &main).unwrap();
    assert_eq!(current(&repo), main);
    assert_eq!(disk(&repo, "untouched.txt").as_deref(), Some("edited\n"));
}

#[test]
fn checkout_of_an_unknown_branch_fails_without_changes() {
    let repo = repo_with_other_branch();
    let main = current(&repo);

    let err = checkout_branch(&repo.repo, "nope").unwrap_err();
    assert!(matches!(err, GitError::BranchNotFound(ref n) if n == "nope"), "{err:?}");
    assert_eq!(current(&repo), main);
}

#[test]
fn checkout_of_a_remote_branch_name_is_not_found() {
    let repo = TestRepo::with_initial_commit();
    repo.repo.reference("refs/remotes/origin/main", head(&repo), true, "test").unwrap();

    let err = checkout_branch(&repo.repo, "origin/main").unwrap_err();
    assert!(matches!(err, GitError::BranchNotFound(_)), "{err:?}");
}

#[test]
fn unrelated_uncommitted_changes_come_along() {
    let repo = repo_with_other_branch();
    repo.write("untouched.txt", "edited locally\n"); // same on both branches
    repo.write("brand-new.txt", "untracked\n");
    repo.write("staged.txt", "staged\n");
    repo.stage("staged.txt");

    checkout_branch(&repo.repo, "other").unwrap();
    assert_eq!(current(&repo), "other");
    assert_eq!(disk(&repo, "untouched.txt").as_deref(), Some("edited locally\n"));
    assert_eq!(disk(&repo, "brand-new.txt").as_deref(), Some("untracked\n"));
    let state: Vec<_> = crate::status::status(&repo.repo)
        .unwrap()
        .into_iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(state, ["brand-new.txt", "staged.txt", "untouched.txt"]);
}

#[test]
fn changes_that_clash_with_the_switch_block_it() {
    let repo = repo_with_other_branch();
    let main = current(&repo);
    repo.write("shared.txt", "my local edit\n"); // `other` changes this file too

    let err = checkout_branch(&repo.repo, "other").unwrap_err();
    assert!(matches!(err, GitError::WouldOverwriteChanges), "{err:?}");
    assert_eq!(current(&repo), main, "HEAD did not move");
    assert_eq!(disk(&repo, "shared.txt").as_deref(), Some("my local edit\n"), "edit is kept");
    assert_eq!(disk(&repo, "only-on-other.txt"), None, "nothing was half-applied");
}

#[test]
fn an_untracked_file_in_the_way_blocks_the_switch() {
    let repo = repo_with_other_branch();
    repo.write("only-on-other.txt", "my own file\n");

    let err = checkout_branch(&repo.repo, "other").unwrap_err();
    assert!(matches!(err, GitError::WouldOverwriteChanges), "{err:?}");
    assert_eq!(disk(&repo, "only-on-other.txt").as_deref(), Some("my own file\n"));
}

#[test]
fn checkout_from_a_detached_head_attaches_it() {
    let repo = repo_with_other_branch();
    repo.repo.set_head_detached(head(&repo)).unwrap();
    assert!(repo.repo.head_detached().unwrap());

    checkout_branch(&repo.repo, "other").unwrap();
    assert!(!repo.repo.head_detached().unwrap());
    assert_eq!(current(&repo), "other");
}

#[test]
fn new_commits_after_checkout_land_on_that_branch() {
    let repo = repo_with_other_branch();
    let main_tip = head(&repo);
    checkout_branch(&repo.repo, "other").unwrap();
    let new = repo.commit_file("x.txt", "x\n", "on other");

    let branches = list_branches(&repo.repo).unwrap();
    assert_eq!(branches.iter().find(|b| b.name == "other").unwrap().target, id(new));
    let main = branches.iter().find(|b| b.name != "other").unwrap();
    assert_eq!(main.target, id(main_tip), "the other branch did not move");
}

// ===================================================================================== delete

#[test]
fn delete_removes_a_merged_branch_but_keeps_its_commits() {
    let repo = TestRepo::with_initial_commit();
    let tip = create_branch(&repo.repo, "done", None).unwrap().target;

    delete_branch(&repo.repo, "done", false).unwrap();
    assert!(!names(&repo).contains(&"done".to_owned()));
    assert!(repo.repo.find_commit(tip.to_oid()).is_ok(), "the commit is still there");
}

#[test]
fn delete_accepts_a_branch_that_was_merged_into_head() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "old", None).unwrap();
    repo.commit_file("a.txt", "a\n", "moves HEAD ahead of `old`");

    delete_branch(&repo.repo, "old", false).unwrap();
    assert!(!names(&repo).contains(&"old".to_owned()));
}

#[test]
fn delete_refuses_the_checked_out_branch_even_when_forced() {
    let repo = TestRepo::with_initial_commit();
    let main = current(&repo);

    for force in [false, true] {
        let err = delete_branch(&repo.repo, &main, force).unwrap_err();
        assert!(matches!(err, GitError::CurrentBranch(ref n) if *n == main), "{err:?}");
    }
    assert!(names(&repo).contains(&main));
}

#[test]
fn delete_refuses_an_unmerged_branch_unless_forced() {
    let repo = repo_with_other_branch();

    let err = delete_branch(&repo.repo, "other", false).unwrap_err();
    assert!(matches!(err, GitError::BranchNotMerged(ref n) if n == "other"), "{err:?}");
    assert!(names(&repo).contains(&"other".to_owned()), "still there");

    delete_branch(&repo.repo, "other", true).unwrap();
    assert!(!names(&repo).contains(&"other".to_owned()));
}

#[test]
fn delete_of_an_unknown_branch_fails() {
    let repo = TestRepo::with_initial_commit();
    let err = delete_branch(&repo.repo, "nope", false).unwrap_err();
    assert!(matches!(err, GitError::BranchNotFound(ref n) if n == "nope"), "{err:?}");
}

#[test]
fn remote_branches_cannot_be_deleted_here() {
    let repo = TestRepo::with_initial_commit();
    repo.repo.reference("refs/remotes/origin/main", head(&repo), true, "test").unwrap();

    let err = delete_branch(&repo.repo, "origin/main", true).unwrap_err();
    assert!(matches!(err, GitError::BranchNotFound(_)), "{err:?}");
    assert!(names(&repo).contains(&"origin/main".to_owned()));
}

#[test]
fn json_round_trip() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "feature", None).unwrap();
    let branches = list_branches(&repo.repo).unwrap();

    let json = serde_json::to_string(&branches).unwrap();
    assert_eq!(serde_json::from_str::<Vec<BranchInfo>>(&json).unwrap(), branches);
    assert!(json.contains("\"kind\":\"local\""), "{json}");
}

// ============================================================================ ahead / behind

fn counts(repo: &TestRepo, local: &str, upstream: &str) -> (usize, usize) {
    let result = ahead_behind(&repo.repo, local, upstream).unwrap();
    (result.ahead, result.behind)
}

/// `main` (the default branch) has 2 commits that `up` lacks, `up` has 3 that `main` lacks.
fn diverged() -> (TestRepo, String) {
    let repo = TestRepo::with_initial_commit();
    let main = current(&repo);
    create_branch(&repo.repo, "up", None).unwrap();

    checkout_branch(&repo.repo, "up").unwrap();
    for i in 1..=3 {
        repo.commit_file(&format!("up{i}.txt"), "x\n", &format!("up {i}"));
    }
    checkout_branch(&repo.repo, &main).unwrap();
    for i in 1..=2 {
        repo.commit_file(&format!("main{i}.txt"), "x\n", &format!("main {i}"));
    }
    (repo, main)
}

#[test]
fn the_same_commit_is_neither_ahead_nor_behind() {
    let repo = TestRepo::with_initial_commit();
    assert_eq!(counts(&repo, "HEAD", "HEAD"), (0, 0));
    create_branch(&repo.repo, "same", None).unwrap();
    assert_eq!(counts(&repo, "same", "HEAD"), (0, 0));
}

#[test]
fn commits_on_top_make_a_branch_ahead() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "old", None).unwrap();
    for i in 1..=3 {
        repo.commit_file(&format!("f{i}.txt"), "x\n", &format!("c{i}"));
    }

    assert_eq!(counts(&repo, "HEAD", "old"), (3, 0));
    assert_eq!(counts(&repo, "old", "HEAD"), (0, 3), "and the other way around: behind");
}

#[test]
fn diverged_branches_are_both_ahead_and_behind() {
    let (repo, main) = diverged();
    assert_eq!(counts(&repo, &main, "up"), (2, 3));
    assert_eq!(counts(&repo, "up", &main), (3, 2));
}

#[test]
fn revisions_can_be_branches_tags_ids_and_relative_names() {
    let repo = TestRepo::with_initial_commit();
    let first = head(&repo);
    repo.tag("v1");
    repo.commit_file("a.txt", "a\n", "second");
    repo.commit_file("b.txt", "b\n", "third");

    for old in ["v1", &first.to_string(), "HEAD~2", "HEAD~2^{commit}"] {
        assert_eq!(counts(&repo, "HEAD", old), (2, 0), "{old}");
    }
    assert_eq!(counts(&repo, "HEAD~1", "HEAD"), (0, 1));
}

#[test]
fn merged_branches_count_their_commits_and_the_merge() {
    let repo = TestRepo::with_initial_commit();
    let base = head(&repo);
    let main = current(&repo);
    create_branch(&repo.repo, "side", None).unwrap();
    checkout_branch(&repo.repo, "side").unwrap();
    repo.commit_file("s1.txt", "x\n", "side 1");
    let side = repo.commit_file("s2.txt", "x\n", "side 2");
    checkout_branch(&repo.repo, &main).unwrap();
    repo.commit_file("m1.txt", "x\n", "main 1");
    // merge commit: parents are HEAD and the side branch, tree keeps HEAD's
    let head_commit = repo.repo.head().unwrap().peel_to_commit().unwrap();
    let side_commit = repo.repo.find_commit(side).unwrap();
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    repo.repo
        .commit(Some("HEAD"), &sig, &sig, "merge", &head_commit.tree().unwrap(), &[&head_commit, &side_commit])
        .unwrap();

    // main 1, side 1, side 2 and the merge commit itself
    assert_eq!(counts(&repo, "HEAD", &base.to_string()), (4, 0));
    assert_eq!(counts(&repo, "HEAD", "side"), (2, 0), "main 1 and the merge");
    assert_eq!(counts(&repo, "side", "HEAD"), (0, 2));
}

#[test]
fn histories_without_a_common_ancestor_count_everything() {
    let repo = TestRepo::with_initial_commit();
    repo.commit_file("a.txt", "a\n", "second"); // 2 commits on HEAD
    let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
    let empty = repo.repo.find_tree(repo.repo.treebuilder(None).unwrap().write().unwrap()).unwrap();
    repo.repo.commit(Some("refs/heads/orphan"), &sig, &sig, "unrelated root", &empty, &[]).unwrap();

    assert_eq!(counts(&repo, "HEAD", "orphan"), (2, 1));
    assert_eq!(counts(&repo, "orphan", "HEAD"), (1, 2));
}

#[test]
fn ahead_behind_with_an_unknown_revision_fails() {
    let repo = TestRepo::with_initial_commit();
    for (local, upstream) in [("nope", "HEAD"), ("HEAD", "nope"), ("nope", "nope"), ("HEAD", "")] {
        let err = ahead_behind(&repo.repo, local, upstream).unwrap_err();
        assert!(matches!(err, GitError::RevisionNotFound(_)), "{local} {upstream}: {err:?}");
    }
}

#[test]
fn ahead_behind_in_a_repository_without_commits_fails() {
    let repo = TestRepo::init();
    let err = ahead_behind(&repo.repo, "HEAD", "HEAD").unwrap_err();
    assert!(matches!(err, GitError::RevisionNotFound(_)), "{err:?}");
}

#[test]
fn ahead_behind_survives_a_json_round_trip() {
    let value = AheadBehind { ahead: 2, behind: 5 };
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(json, r#"{"ahead":2,"behind":5}"#);
    assert_eq!(serde_json::from_str::<AheadBehind>(&json).unwrap(), value);
}

// ------------------------------------------------------------------ BranchInfo::ahead_behind

fn info(repo: &TestRepo, name: &str) -> BranchInfo {
    list_branches(&repo.repo).unwrap().into_iter().find(|b| b.name == name).expect(name)
}

fn track(repo: &TestRepo, branch: &str, upstream: &str) {
    repo.repo.find_branch(branch, git2::BranchType::Local).unwrap().set_upstream(Some(upstream)).unwrap();
}

#[test]
fn a_branch_without_an_upstream_has_no_comparison() {
    let repo = TestRepo::with_initial_commit();
    assert_eq!(info(&repo, &current(&repo)).ahead_behind, None);
}

#[test]
fn a_branch_level_with_its_upstream_is_zero_zero() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "feature", None).unwrap();
    track(&repo, "feature", &current(&repo));

    assert_eq!(info(&repo, "feature").ahead_behind, Some(AheadBehind { ahead: 0, behind: 0 }));
}

#[test]
fn branch_info_compares_with_a_local_upstream() {
    let (repo, main) = diverged();
    track(&repo, &main, "up");

    let branch = info(&repo, &main);
    assert_eq!(branch.upstream.as_deref(), Some("up"));
    assert_eq!(branch.ahead_behind, Some(AheadBehind { ahead: 2, behind: 3 }));
    assert_eq!(info(&repo, "up").ahead_behind, None, "`up` has no upstream of its own");
}

#[test]
fn branch_info_compares_with_a_remote_tracking_branch() {
    let repo = TestRepo::with_initial_commit();
    let main = current(&repo);
    let published = head(&repo);
    repo.repo.remote("origin", "https://example.invalid/repo.git").unwrap();
    repo.repo.reference("refs/remotes/origin/main", published, true, "fetch").unwrap();
    track(&repo, &main, "origin/main");
    repo.commit_file("a.txt", "a\n", "local 1");
    repo.commit_file("b.txt", "b\n", "local 2");

    let branch = info(&repo, &main);
    assert_eq!(branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!(branch.ahead_behind, Some(AheadBehind { ahead: 2, behind: 0 }));

    // someone else pushes: the remote moves on and we are behind as well
    let theirs = repo.repo.find_commit(published).unwrap();
    let sig = git2::Signature::now("Other", "other@example.com").unwrap();
    let pushed = repo.repo.commit(None, &sig, &sig, "remote work", &theirs.tree().unwrap(), &[&theirs]).unwrap();
    repo.repo.reference("refs/remotes/origin/main", pushed, true, "fetch").unwrap();
    assert_eq!(info(&repo, &main).ahead_behind, Some(AheadBehind { ahead: 2, behind: 1 }));

    // the remote-tracking branch itself is not compared with anything
    assert_eq!(info(&repo, "origin/main").ahead_behind, None);
}

#[test]
fn a_vanished_upstream_means_no_comparison() {
    let repo = TestRepo::with_initial_commit();
    create_branch(&repo.repo, "feature", None).unwrap();
    create_branch(&repo.repo, "gone-soon", None).unwrap();
    track(&repo, "feature", "gone-soon");
    repo.repo.find_branch("gone-soon", git2::BranchType::Local).unwrap().delete().unwrap();

    let branch = info(&repo, "feature");
    assert_eq!(branch.ahead_behind, None);
}

#[test]
fn created_branches_report_their_comparison_too() {
    let repo = TestRepo::with_initial_commit();
    let created = create_branch(&repo.repo, "feature", None).unwrap();
    assert_eq!(created.ahead_behind, None, "a new branch has no upstream yet");
}

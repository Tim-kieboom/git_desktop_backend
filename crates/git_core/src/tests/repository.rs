//! Tests for the public `Repository` wrapper (needs the private `raw` field, so it can't be shared
//! with the integration tests).

use crate::tests::TestRepo;
use crate::{CommitId, CoreError, DiffTarget, GitError, LogQuery};

fn open(repo: &TestRepo) -> crate::Repository {
    crate::Repository {
        raw: git2::Repository::open(repo.path()).unwrap(),
    }
}

#[test]
fn commit_returns_the_id_of_the_new_head() {
    let repo = TestRepo::with_initial_commit();
    repo.write("a.txt", "x\n");
    repo.stage("a.txt");

    let id = open(&repo).commit("add a").unwrap();
    let head = repo.repo.head().unwrap().target().unwrap();
    assert_eq!(id.to_string(), head.to_string());
    assert_eq!(id.to_string().parse::<CommitId>().unwrap(), id);
}

#[test]
fn commit_errors_are_core_errors() {
    let repo = TestRepo::with_initial_commit();

    let err = open(&repo).commit("nothing staged").unwrap_err();
    assert!(
        matches!(err, CoreError::Git(GitError::NothingToCommit)),
        "{err:?}"
    );
}

fn numbered(changed: &[u32]) -> String {
    (1..=30)
        .map(|i| match changed.contains(&i) {
            true => format!("CHANGED {i}\n"),
            false => format!("line {i}\n"),
        })
        .collect()
}

#[test]
fn stage_hunk_then_commit_only_commits_that_hunk() {
    let repo = TestRepo::init();
    repo.commit_file("f.txt", &numbered(&[]), "base");
    repo.write("f.txt", &numbered(&[3, 28]));
    let wrapper = open(&repo);
    let path = std::path::Path::new("f.txt");

    let hunks = wrapper.file_diff(DiffTarget::Unstaged, path).unwrap().unwrap().hunks;
    assert_eq!(hunks.len(), 2);
    wrapper.stage_hunk(path, &hunks[0]).unwrap();
    wrapper.commit("first change only").unwrap();

    let rest = wrapper.file_diff(DiffTarget::Unstaged, path).unwrap().unwrap().hunks;
    assert_eq!(rest.len(), 1, "the second change is still pending");
    assert!(rest[0].lines.iter().any(|l| l.content == "CHANGED 28"));
}

#[test]
fn unstage_hunk_through_the_wrapper() {
    let repo = TestRepo::init();
    repo.commit_file("f.txt", &numbered(&[]), "base");
    repo.write("f.txt", &numbered(&[3, 28]));
    repo.stage("f.txt");
    let wrapper = open(&repo);
    let path = std::path::Path::new("f.txt");

    let staged = wrapper.file_diff(DiffTarget::Staged, path).unwrap().unwrap().hunks;
    wrapper.unstage_hunk(path, &staged[1]).unwrap();

    let staged = wrapper.file_diff(DiffTarget::Staged, path).unwrap().unwrap().hunks;
    assert_eq!(staged.len(), 1);
}

#[test]
fn stale_hunk_is_a_core_error() {
    let repo = TestRepo::init();
    repo.commit_file("f.txt", &numbered(&[]), "base");
    repo.write("f.txt", &numbered(&[3]));
    let wrapper = open(&repo);
    let path = std::path::Path::new("f.txt");
    let hunk = wrapper.file_diff(DiffTarget::Unstaged, path).unwrap().unwrap().hunks.remove(0);
    wrapper.stage_hunk(path, &hunk).unwrap();

    let err = wrapper.stage_hunk(path, &hunk).unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::HunkNotFound(_))), "{err:?}");
}

#[test]
fn release_workflow_through_the_wrapper() {
    let repo = TestRepo::init();
    let wrapper = open(&repo);
    let edit_and_commit = |content: &str, message: &str| {
        repo.write("f.txt", content);
        wrapper.stage(std::path::Path::new("f.txt")).unwrap();
        wrapper.commit(message).unwrap()
    };

    let first = edit_and_commit("1\n", "feat: first");
    wrapper.create_tag("v1.0.0", None, Some("Release 1.0.0")).unwrap();
    edit_and_commit("2\n", "fix: second");
    edit_and_commit("3\n", "feat: third");

    // what changed since the release
    let since = LogQuery { exclude: Some("v1.0.0".into()), ..LogQuery::default() };
    let page = wrapper.log(&since).unwrap();
    let summaries: Vec<_> = page.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(summaries, ["feat: third", "fix: second"]);

    let tags = wrapper.tags().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].target, first);
    assert!(tags[0].annotated);

    let detail = wrapper.commit_detail(page.commits[0].id).unwrap();
    assert_eq!(detail.files.len(), 1);
    assert_eq!(&*detail.files[0].path, std::path::Path::new("f.txt"));
}

#[test]
fn branch_workflow_through_the_wrapper() {
    let repo = TestRepo::with_initial_commit();
    let wrapper = open(&repo);

    wrapper.create_branch("feature", None).unwrap();
    wrapper.checkout_branch("feature").unwrap();
    repo.write("new.txt", "x\n");
    wrapper.stage(std::path::Path::new("new.txt")).unwrap();
    wrapper.commit("work on feature").unwrap();

    let branches = wrapper.branches().unwrap();
    assert!(branches.iter().find(|b| b.name == "feature").unwrap().is_head);

    let err = wrapper.delete_branch("feature", false).unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::CurrentBranch(_))), "{err:?}");

    let other = branches.iter().find(|b| b.name != "feature").unwrap().name.clone();
    wrapper.checkout_branch(&other).unwrap();
    let err = wrapper.delete_branch("feature", false).unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::BranchNotMerged(_))), "{err:?}");
    wrapper.delete_branch("feature", true).unwrap();
    assert_eq!(wrapper.branches().unwrap().len(), 1);
}

#[test]
fn tag_errors_are_core_errors() {
    let repo = TestRepo::with_initial_commit();
    let wrapper = open(&repo);
    wrapper.create_tag("v1", None, None).unwrap();

    let err = wrapper.create_tag("v1", None, None).unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::TagAlreadyExists(_))), "{err:?}");
    wrapper.delete_tag("v1").unwrap();
    let err = wrapper.delete_tag("v1").unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::TagNotFound(_))), "{err:?}");
}

#[test]
fn ahead_behind_through_the_wrapper() {
    let repo = TestRepo::with_initial_commit();
    let wrapper = open(&repo);
    wrapper.create_tag("v1", None, None).unwrap();
    repo.commit_file("a.txt", "a\n", "one");
    repo.commit_file("b.txt", "b\n", "two");

    let result = wrapper.ahead_behind("HEAD", "v1").unwrap();
    assert_eq!((result.ahead, result.behind), (2, 0));
    let err = wrapper.ahead_behind("HEAD", "nope").unwrap_err();
    assert!(matches!(err, CoreError::Git(GitError::RevisionNotFound(_))), "{err:?}");
}

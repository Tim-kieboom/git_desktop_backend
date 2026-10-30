//! Tests for the public `Repository` wrapper (needs the private `raw` field, so it can't be shared
//! with the integration tests).

use crate::tests::TestRepo;
use crate::{CommitId, CoreError, DiffTarget, GitError};

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

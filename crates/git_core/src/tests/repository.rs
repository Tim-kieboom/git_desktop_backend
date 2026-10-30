//! Tests for the public `Repository` wrapper (needs the private `raw` field, so it can't be shared
//! with the integration tests).

use crate::tests::TestRepo;
use crate::{CommitId, CoreError, GitError};

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

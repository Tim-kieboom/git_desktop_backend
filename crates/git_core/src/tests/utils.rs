//! Helpers for tests that need real git repositories on disk.
//!
//! Shared by the unit tests (`#[cfg(test)] mod test_utils`) and the integration tests
//! (`#[path = "../src/test_utils.rs"] mod test_utils`), so only depend on dev-dependencies here.
#![allow(dead_code)]

use git2::Repository;
use std::path::Path;

use crate::tests::{TestRepo, plain_dir};

#[test]
fn init_is_an_empty_repo() {
    let repo = TestRepo::init();
    assert!(repo.repo.head().is_err(), "no commits yet");
    assert!(repo.path().join(".git").is_dir());
}

#[test]
fn commit_file_creates_history_and_parents() {
    let repo = TestRepo::with_initial_commit();
    let second = repo.commit_file("src/a.txt", "a", "add a");

    let commit = repo.repo.find_commit(second).unwrap();
    assert_eq!(commit.parent_count(), 1);
    assert_eq!(commit.message().unwrap(), "add a");
    assert!(repo.path().join("src/a.txt").is_file());
}

#[test]
fn stage_all_picks_up_new_and_modified_files() {
    let repo = TestRepo::with_initial_commit();
    repo.write("README.md", "changed\n");
    repo.write("new.txt", "new\n");
    repo.stage_all();

    let index = repo.repo.index().unwrap();
    assert!(index.get_path(Path::new("new.txt"), 0).is_some());
}

#[test]
fn tag_points_at_head() {
    let repo = TestRepo::with_initial_commit();
    repo.tag("v1.0.0");
    let tagged = repo.repo.revparse_single("v1.0.0").unwrap().id();
    assert_eq!(tagged, repo.repo.head().unwrap().target().unwrap());
}

#[test]
fn plain_dir_is_not_a_repo() {
    let dir = plain_dir();
    assert!(Repository::open(dir.path()).is_err());
}

//! Integration tests: the public registry API on top of the JSON backend.
use git_core::repo_registry::{
    RepoError, RepoRegistry,
    storage::{StoreError, json_storage::JsonStorage},
};
use std::{path::Path, sync::Arc};

#[path = "../src/test_utils.rs"]
mod test_utils;
use test_utils::{TestRepo, plain_dir};

/// Registry whose storage lives in `data` (JsonStorage::new() would use the real user dir).
fn registry(data: &Path) -> RepoRegistry<JsonStorage> {
    RepoRegistry::with_storage(JsonStorage::with_dir(data).unwrap())
}

#[test]
fn entries_survive_reopening_the_storage() {
    let data = plain_dir();
    let repo = TestRepo::init();

    registry(data.path()).add(repo.path(), "repo").unwrap();

    let reopened = registry(data.path());
    let found = reopened.find(repo.path()).unwrap().expect("persisted");
    assert_eq!(&*found.name, "repo");
}

#[test]
fn removal_survives_reopening_the_storage() {
    let data = plain_dir();
    let repo = TestRepo::init();

    let reg = registry(data.path());
    reg.add(repo.path(), "repo").unwrap();
    let id = reg.find(repo.path()).unwrap().unwrap().id;
    assert!(reg.remove(id).unwrap());

    assert!(registry(data.path()).get(id).unwrap().is_none());
}

#[test]
fn duplicate_is_rejected_across_reopen() {
    let data = plain_dir();
    let repo = TestRepo::init();

    registry(data.path()).add(repo.path(), "a").unwrap();
    let err = registry(data.path()).add(repo.path(), "b").unwrap_err();
    assert!(matches!(
        err,
        RepoError::Store(StoreError::DuplicatePath(_))
    ));
}

#[test]
fn non_git_directory_is_rejected_and_not_persisted() {
    let data = plain_dir();
    let not_a_repo = plain_dir();

    let err = registry(data.path()).add(not_a_repo.path(), "x").unwrap_err();
    assert!(matches!(err, RepoError::Git2(_)));
    assert_eq!(registry(data.path()).list().unwrap().entries.values().count(), 0);
}

#[test]
fn concurrent_adds_of_same_path_yield_one_entry() {
    let data = plain_dir();
    let repo = TestRepo::init();
    let reg = Arc::new(registry(data.path()));

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let (reg, path) = (reg.clone(), repo.path().to_path_buf());
            std::thread::spawn(move || reg.add(&path, format!("n{i}")).is_ok())
        })
        .collect();
    let ok = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|b| *b)
        .count();

    assert_eq!(ok, 1);
    assert_eq!(reg.list().unwrap().entries.values().count(), 1);
}

#[test]
fn separate_storage_instances_do_not_lose_updates() {
    // Each thread opens its own registry on the same directory, like separate processes would
    // (the OS lock, not a shared in-memory lock, has to keep them apart).
    let data = plain_dir();
    let repos: Vec<_> = (0..8).map(|_| TestRepo::init()).collect();

    let handles: Vec<_> = repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let (data, path) = (data.path().to_path_buf(), repo.path().to_path_buf());
            std::thread::spawn(move || registry(&data).add(&path, format!("n{i}")).unwrap())
        })
        .collect();
    handles.into_iter().for_each(|h| h.join().unwrap());

    assert_eq!(
        registry(data.path()).list().unwrap().entries.values().count(),
        8
    );
}

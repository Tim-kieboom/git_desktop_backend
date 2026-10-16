//! Integration tests: the public registry API on top of the JSON backend.
use git_core::repo_registry::{
    RepoError, RepoRegistry,
    storage::{StoreError, json_storage::JsonStorage},
};
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("git_core_it_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Registry whose storage lives in `data` (JsonStorage::new() would use the real user dir).
fn registry(data: &PathBuf) -> RepoRegistry<JsonStorage> {
    RepoRegistry::with_storage(JsonStorage::with_dir(data).unwrap())
}

#[test]
fn entries_survive_reopening_the_storage() {
    let data = temp_dir("persist_data");
    let repo = temp_dir("persist_repo");

    registry(&data).add(&repo, "repo").unwrap();

    let reopened = registry(&data);
    let found = reopened.find(&repo).unwrap().expect("persisted");
    assert_eq!(&*found.name, "repo");
}

#[test]
fn removal_survives_reopening_the_storage() {
    let data = temp_dir("rm_data");
    let repo = temp_dir("rm_repo");

    let reg = registry(&data);
    reg.add(&repo, "repo").unwrap();
    let id = reg.find(&repo).unwrap().unwrap().id;
    assert!(reg.remove(id).unwrap());

    assert!(registry(&data).get(id).unwrap().is_none());
}

#[test]
fn duplicate_is_rejected_across_reopen() {
    let data = temp_dir("dup_data");
    let repo = temp_dir("dup_repo");

    registry(&data).add(&repo, "a").unwrap();
    let err = registry(&data).add(&repo, "b").unwrap_err();
    assert!(matches!(err, RepoError::Store(StoreError::DuplicatePath(_))));
}

#[test]
fn concurrent_adds_of_same_path_yield_one_entry() {
    let data = temp_dir("race_data");
    let repo = temp_dir("race_repo");
    let reg = std::sync::Arc::new(registry(&data));

    let handles: Vec<_> = (0..8)
        .map(|i| {
            let (reg, repo) = (reg.clone(), repo.clone());
            std::thread::spawn(move || reg.add(&repo, format!("n{i}")).is_ok())
        })
        .collect();
    let ok = handles.into_iter().map(|h| h.join().unwrap()).filter(|b| *b).count();

    assert_eq!(ok, 1);
    assert_eq!(reg.list().unwrap().entries.values().count(), 1);
}

#[test]
fn separate_storage_instances_do_not_lose_updates() {
    // Each thread opens its own registry on the same directory, like separate processes would
    // (the OS lock, not a shared in-memory lock, has to keep them apart).
    let data = temp_dir("multi_data");
    let repos: Vec<_> = (0..8).map(|i| temp_dir(&format!("multi_repo{i}"))).collect();

    let handles: Vec<_> = repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let (data, repo) = (data.clone(), repo.clone());
            std::thread::spawn(move || registry(&data).add(&repo, format!("n{i}")).unwrap())
        })
        .collect();
    handles.into_iter().for_each(|h| h.join().unwrap());

    assert_eq!(registry(&data).list().unwrap().entries.values().count(), 8);
}

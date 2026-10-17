use super::{empty_store, temp_dir};
use crate::repo_registry::storage::{RepoStorage, StoreError, json_storage::JsonStorage};

#[test]
fn load_without_file_is_empty() {
    let dir = temp_dir("empty");
    let s = JsonStorage::with_dir(&dir).unwrap();
    assert_eq!(s.load().unwrap().entries.values().count(), 0);
}

#[test]
fn with_dir_creates_directory_not_file_named_repos_json() {
    let dir = temp_dir("mkdir");
    let _s = JsonStorage::with_dir(&dir).unwrap();
    assert!(dir.is_dir());
    assert!(!dir.join("repos.json").is_dir());
}

#[test]
fn save_then_load_round_trips_and_leaves_no_temp_file() {
    let dir = temp_dir("roundtrip");
    let s = JsonStorage::with_dir(&dir).unwrap();
    let mut store = s.load().unwrap();
    store.alloc.alloc();
    s.save(store).unwrap();

    // a fresh instance reads what the first one wrote
    let again = JsonStorage::with_dir(&dir).unwrap();
    assert!(again.load().is_ok());
    assert!(!dir.join("repos.json.tmp").exists());
}

#[test]
fn save_shorter_data_over_longer_is_valid_json() {
    let dir = temp_dir("shrink");
    let s = JsonStorage::with_dir(&dir).unwrap();
    std::fs::write(dir.join("repos.json"), "x".repeat(10_000)).unwrap();
    s.save(empty_store()).unwrap();
    assert!(s.load().is_ok());
}

#[test]
fn load_corrupt_file_is_error() {
    let dir = temp_dir("corrupt");
    let s = JsonStorage::with_dir(&dir).unwrap();
    std::fs::write(dir.join("repos.json"), "not json").unwrap();
    assert!(matches!(s.load(), Err(StoreError::Json(_))));
}

#[test]
fn update_persists_changes() {
    let dir = temp_dir("update");
    let s = JsonStorage::with_dir(&dir).unwrap();
    s.update(|store| {
        store.alloc.alloc();
        Ok(())
    })
    .unwrap();

    let mut loaded = s.load().unwrap();
    let next = loaded.alloc.alloc();
    // first alloc on a fresh store would be the lowest id; update must have advanced it
    let mut fresh = empty_store();
    assert_ne!(next, fresh.alloc.alloc());
}

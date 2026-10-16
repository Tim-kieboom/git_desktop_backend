use super::*;
use crate::repo_registry::storage::memory_storage::MemoryStorage;

fn registry() -> RepoRegistry<MemoryStorage> {
    RepoRegistry::<MemoryStorage>::new().unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("git_core_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn add_then_list_contains_entry() {
    let reg = registry();
    let dir = temp_dir("add_list");
    reg.add(&dir, "one").unwrap();

    let store = reg.list().unwrap();
    let entries: Vec<_> = store.entries.values().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(&*entries[0].name, "one");
    assert_eq!(entries[0].path, dir.canonicalize().unwrap());
}

#[test]
fn add_duplicate_path_fails() {
    let reg = registry();
    let dir = temp_dir("dup_path");
    reg.add(&dir, "a").unwrap();

    let err = reg.add(&dir, "b").unwrap_err();
    assert!(matches!(err, RepoError::Store(StoreError::DuplicatePath(_))));
    assert_eq!(reg.list().unwrap().entries.values().count(), 1);
}

#[test]
fn add_duplicate_name_fails() {
    let reg = registry();
    let a = temp_dir("dup_name_a");
    let b = temp_dir("dup_name_b");
    reg.add(&a, "same").unwrap();

    let err = reg.add(&b, "same").unwrap_err();
    assert!(matches!(err, RepoError::Store(StoreError::DuplicateName(_))));
}

#[test]
fn add_equivalent_path_spellings_are_duplicates() {
    let reg = registry();
    let dir = temp_dir("spelling");
    reg.add(&dir, "a").unwrap();

    let with_dot = dir.join(".");
    let err = reg.add(with_dot, "b").unwrap_err();
    assert!(matches!(err, RepoError::Store(StoreError::DuplicatePath(_))));
}

#[test]
fn add_missing_path_fails() {
    let reg = registry();
    let err = reg.add("/definitely/not/a/real/path", "x").unwrap_err();
    assert!(matches!(err, RepoError::Store(StoreError::Io(_))));
}

#[test]
fn get_and_find_return_added_entry() {
    let reg = registry();
    let dir = temp_dir("get_find");
    reg.add(&dir, "a").unwrap();

    let found = reg.find(&dir).unwrap().expect("found by path");
    let got = reg.get(found.id).unwrap().expect("found by id");
    assert_eq!(got.path, found.path);
}

#[test]
fn find_unknown_path_returns_none() {
    let reg = registry();
    let known = temp_dir("find_known");
    let other = temp_dir("find_other");
    reg.add(&known, "a").unwrap();
    assert!(reg.find(&other).unwrap().is_none());
}

#[test]
fn remove_deletes_entry_and_reports_result() {
    let reg = registry();
    let dir = temp_dir("remove");
    reg.add(&dir, "a").unwrap();
    let id = reg.find(&dir).unwrap().unwrap().id;

    assert!(reg.remove(id).unwrap());
    assert!(reg.get(id).unwrap().is_none());
    assert!(!reg.remove(id).unwrap(), "second removal is a no-op");
}

#[test]
fn ids_are_unique_across_removals() {
    let reg = registry();
    let a = temp_dir("ids_a");
    let b = temp_dir("ids_b");
    let c = temp_dir("ids_c");
    reg.add(&a, "a").unwrap();
    let first = reg.find(&a).unwrap().unwrap().id;
    reg.remove(first).unwrap();
    reg.add(&b, "b").unwrap();
    reg.add(&c, "c").unwrap();

    let ids: Vec<_> = reg.list().unwrap().entries.values().map(|e| e.id).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

use super::*;
use crate::repo_registry::storage::memory_storage::MemoryStorage;
use crate::test_utils::{TestRepo, plain_dir};

fn registry() -> RepoRegistry<MemoryStorage> {
    RepoRegistry::<MemoryStorage>::new().unwrap()
}

#[test]
fn add_then_list_contains_entry() {
    let reg = registry();
    let repo = TestRepo::init();
    reg.add(repo.path(), "one").unwrap();

    let store = reg.list().unwrap();
    let entries: Vec<_> = store.entries.values().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(&*entries[0].name, "one");
    assert_eq!(entries[0].path, repo.path().canonicalize().unwrap());
}

#[test]
fn add_works_for_repo_with_history() {
    let reg = registry();
    let repo = TestRepo::with_initial_commit();
    reg.add(repo.path(), "one").unwrap();
    assert_eq!(reg.list().unwrap().entries.values().count(), 1);
}

#[test]
fn add_non_git_directory_fails() {
    let reg = registry();
    let dir = plain_dir();

    let err = reg.add(dir.path(), "x").unwrap_err();
    assert!(matches!(err, CoreError::Repo(RepoError::Git(_))));
    assert_eq!(reg.list().unwrap().entries.values().count(), 0);
}

#[test]
fn add_duplicate_path_fails() {
    let reg = registry();
    let repo = TestRepo::init();
    reg.add(repo.path(), "a").unwrap();

    let err = reg.add(repo.path(), "b").unwrap_err();
    assert!(matches!(
        err,
        CoreError::Repo(RepoError::Store(StoreError::DuplicatePath(_)))
    ));
    assert_eq!(reg.list().unwrap().entries.values().count(), 1);
}

#[test]
fn add_duplicate_name_fails() {
    let reg = registry();
    let (a, b) = (TestRepo::init(), TestRepo::init());
    reg.add(a.path(), "same").unwrap();

    let err = reg.add(b.path(), "same").unwrap_err();
    assert!(matches!(
        err,
        CoreError::Repo(RepoError::Store(StoreError::DuplicateName(_)))
    ));
}

#[test]
fn add_equivalent_path_spellings_are_duplicates() {
    let reg = registry();
    let repo = TestRepo::init();
    reg.add(repo.path(), "a").unwrap();

    let err = reg.add(repo.path().join("."), "b").unwrap_err();
    assert!(matches!(
        err,
        CoreError::Repo(RepoError::Store(StoreError::DuplicatePath(_)))
    ));
}

#[test]
fn add_missing_path_fails() {
    let reg = registry();
    let err = reg.add("/definitely/not/a/real/path", "x").unwrap_err();
    assert!(matches!(
        err,
        CoreError::Repo(RepoError::Store(StoreError::Io(_)))
    ));
}

#[test]
fn get_and_find_return_added_entry() {
    let reg = registry();
    let repo = TestRepo::init();
    reg.add(repo.path(), "a").unwrap();

    let found = reg.find(repo.path()).unwrap().expect("found by path");
    let got = reg.get(found.id).unwrap().expect("found by id");
    assert_eq!(got.path, found.path);
}

#[test]
fn find_unknown_path_returns_none() {
    let reg = registry();
    let (known, other) = (TestRepo::init(), TestRepo::init());
    reg.add(known.path(), "a").unwrap();
    assert!(reg.find(other.path()).unwrap().is_none());
}

#[test]
fn remove_deletes_entry_and_reports_result() {
    let reg = registry();
    let repo = TestRepo::init();
    reg.add(repo.path(), "a").unwrap();
    let id = reg.find(repo.path()).unwrap().unwrap().id;

    assert!(reg.remove(id).unwrap());
    assert!(reg.get(id).unwrap().is_none());
    assert!(!reg.remove(id).unwrap(), "second removal is a no-op");
}

#[test]
fn ids_are_unique_across_removals() {
    let reg = registry();
    let (a, b, c) = (TestRepo::init(), TestRepo::init(), TestRepo::init());
    reg.add(a.path(), "a").unwrap();
    let first = reg.find(a.path()).unwrap().unwrap().id;
    reg.remove(first).unwrap();
    reg.add(b.path(), "b").unwrap();
    reg.add(c.path(), "c").unwrap();

    let ids: Vec<_> = reg.list().unwrap().entries.values().map(|e| e.id).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

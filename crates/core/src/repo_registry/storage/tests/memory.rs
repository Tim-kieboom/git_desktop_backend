use crate::repo_registry::storage::{RepoStorage, StoreError, memory_storage::MemoryStorage};

#[test]
fn starts_empty() {
    let s = MemoryStorage::new().unwrap();
    assert_eq!(s.load().unwrap().entries.values().count(), 0);
}

#[test]
fn save_then_load_round_trips() {
    let s = MemoryStorage::new().unwrap();
    let mut store = s.load().unwrap();
    let id = store.alloc.alloc();
    store.alloc.alloc();
    s.save(store).unwrap();

    let mut loaded = s.load().unwrap();
    assert_ne!(loaded.alloc.alloc(), id, "allocator state is kept");
}

#[test]
fn update_error_is_propagated() {
    let s = MemoryStorage::new().unwrap();
    let err = s.update(|_| Err(StoreError::Unknown("boom".into()))).unwrap_err();
    assert!(matches!(err, StoreError::Unknown(_)));
}

#[test]
fn update_is_atomic_across_threads() {
    use std::sync::Arc;
    let s = Arc::new(MemoryStorage::new().unwrap());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let s = s.clone();
            std::thread::spawn(move || {
                for _ in 0..50 {
                    s.update(|store| {
                        store.alloc.alloc();
                        Ok(())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    handles.into_iter().for_each(|h| h.join().unwrap());

    let mut store = s.load().unwrap();
    let next = store.alloc.alloc();
    assert_eq!(format!("{next:?}"), "RepoId(401)");
}

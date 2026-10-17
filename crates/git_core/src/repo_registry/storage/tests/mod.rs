use crate::repo_registry::storage::RepoStore;

mod json;
mod memory;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("git_core_storage_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn empty_store() -> RepoStore {
    RepoStore::new()
}

use git_core::repo_registry::{RepoError, RepoRegistry, storage::memory_storage::MemoryStorage};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}")
    }
}

fn run() -> Result<(), RepoError> {
    let register = RepoRegistry::<MemoryStorage>::new()?;
    register.add("/mnt/windows", "test")?;
    register.add("/mnt", "test")?;
    let repos = register.list()?;
    let values = repos.entries.values().collect::<Vec<_>>();
    println!("{:#?}", values);
    Ok(())
}

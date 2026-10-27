use git_core::repo_registry::{RepoError, RepoRegistry, storage::memory_storage::MemoryStorage};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}")
    }
}

fn run() -> Result<(), RepoError> {
    let register = RepoRegistry::<MemoryStorage>::new()?;
    register.add("./", "this")?;
    register.add("../../TerminalCode", "trcode")?;
    for repo in &register.list()? {
        println!("{:#?}\n", repo)
    }

    Ok(())
}

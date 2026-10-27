//! Helpers for tests that need real git repositories on disk.
//!
//! Shared by the unit tests (`#[cfg(test)] mod test_utils`) and the integration tests
//! (`#[path = "../src/test_utils.rs"] mod test_utils`), so only depend on dev-dependencies here.
#![allow(dead_code)]

use git2::{IndexAddOption, Oid, Repository, Signature};
use std::path::Path;
use tempfile::TempDir;

/// A freshly initialised git repository in a temp directory that is removed on drop.
pub struct TestRepo {
    dir: TempDir,
    pub repo: Repository,
}

impl TestRepo {
    /// Empty repository (no commits) with a committer identity configured.
    pub fn init() -> Self {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();

        Self { dir, repo }
    }

    /// Repository with one commit (`README.md`), for tests that need a valid `HEAD`.
    pub fn with_initial_commit() -> Self {
        let repo = Self::init();
        repo.commit_file("README.md", "# test\n", "initial commit");
        repo
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Writes (or overwrites) a working-tree file, creating parent directories.
    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path().join(rel)).unwrap();
    }

    /// Stages a single file, like `git add <rel>`.
    pub fn stage(&self, rel: &str) {
        let mut index = self.repo.index().unwrap();
        index.add_path(Path::new(rel)).unwrap();
        index.write().unwrap();
    }

    /// Stages everything, like `git add -A`.
    pub fn stage_all(&self) {
        let mut index = self.repo.index().unwrap();
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .unwrap();
        // add_all skips deletions; update_all stages them
        index.update_all(["*"], None).unwrap();
        index.write().unwrap();
    }

    /// Commits whatever is currently staged on top of `HEAD` (or as a root commit).
    pub fn commit(&self, message: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = Signature::now("Test User", "test@example.com").unwrap();

        let parent = self.repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let parents: Vec<_> = parent.iter().collect();
        self.repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
    }

    /// Writes, stages and commits one file.
    pub fn commit_file(&self, rel: &str, contents: &str, message: &str) -> Oid {
        self.write(rel, contents);
        self.stage(rel);
        self.commit(message)
    }

    /// Leaves `c.txt` in an unresolved merge conflict: a `feature` branch is merged into the
    /// default branch, and both changed the same line. Needs a repo without a `c.txt`.
    pub fn make_conflict(&self) {
        self.commit_file("c.txt", "base\n", "base");
        let head = self.repo.head().unwrap();
        let default_branch = head.shorthand().unwrap().to_owned();
        let base = head.peel_to_commit().unwrap();

        self.repo.branch("feature", &base, false).unwrap();
        self.commit_file("c.txt", "default branch change\n", "change on default");

        self.repo.set_head("refs/heads/feature").unwrap();
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        self.commit_file("c.txt", "feature change\n", "change on feature");

        let theirs = self
            .repo
            .find_branch(&default_branch, git2::BranchType::Local)
            .unwrap()
            .get()
            .peel_to_commit()
            .unwrap();
        let annotated = self.repo.find_annotated_commit(theirs.id()).unwrap();
        self.repo.merge(&[&annotated], None, None).unwrap();
    }

    /// Lightweight tag on `HEAD`.
    pub fn tag(&self, name: &str) {
        let head = self.repo.head().unwrap().peel_to_commit().unwrap();
        self.repo.tag_lightweight(name, head.as_object(), false).unwrap();
    }
}

/// A plain directory that is *not* a git repository.
pub fn plain_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

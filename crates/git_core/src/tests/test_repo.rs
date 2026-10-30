//! Helpers for tests that need real git repositories on disk.
//!
//! This file is shared with the integration tests (`#[path = "../src/tests/test_repo.rs"]`), where
//! `crate` is the test crate: it must only use dev-dependencies and never refer to `crate::`.
#![allow(dead_code)]

use std::path::Path;

use git2::{IndexAddOption, Oid, Signature};
use tempfile::TempDir;

/// A freshly initialised git repository in a temp directory that is removed on drop.
pub struct TestRepo {
    dir: TempDir,
    pub repo: git2::Repository,
}

impl TestRepo {
    /// Empty repository (no commits) with a committer identity configured.
    pub fn init() -> Self {
        let dir = TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();

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
        let mut index = self.as_raw().index().unwrap();
        index.add_path(Path::new(rel)).unwrap();
        index.write().unwrap();
    }

    /// Stages everything, like `git add -A`.
    pub fn stage_all(&self) {
        let mut index = self.as_raw().index().unwrap();
        index.add_all(["*"], IndexAddOption::DEFAULT, None).unwrap();
        // add_all skips deletions; update_all stages them
        index.update_all(["*"], None).unwrap();
        index.write().unwrap();
    }

    /// Commits whatever is currently staged on top of `HEAD` (or as a root commit).
    pub fn commit(&self, message: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        let tree = self
            .as_raw()
            .find_tree(index.write_tree().unwrap())
            .unwrap();
        let sig = Signature::now("Test User", "test@example.com").unwrap();

        let parent = self.repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let parents: Vec<_> = parent.iter().collect();
        self.as_raw()
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .unwrap()
    }

    /// Like [`TestRepo::commit`] but with another author and committer.
    pub fn commit_as(&self, name: &str, email: &str, message: &str) -> Oid {
        self.commit_signed(&Signature::now(name, email).unwrap(), message)
    }

    /// Like [`TestRepo::commit`] but at a fixed time (seconds since the unix epoch).
    pub fn commit_at(&self, message: &str, unix_seconds: i64) -> Oid {
        let sig =
            Signature::new("Test User", "test@example.com", &git2::Time::new(unix_seconds, 0))
                .unwrap();
        self.commit_signed(&sig, message)
    }

    fn commit_signed(&self, sig: &Signature<'_>, message: &str) -> Oid {
        let mut index = self.repo.index().unwrap();
        let tree = self.as_raw().find_tree(index.write_tree().unwrap()).unwrap();
        let parent = self.repo.head().ok().map(|h| h.peel_to_commit().unwrap());
        let parents: Vec<_> = parent.iter().collect();
        self.as_raw()
            .commit(Some("HEAD"), sig, sig, message, &tree, &parents)
            .unwrap()
    }

    /// Writes, stages and commits one file as `name <email>`.
    pub fn commit_file_as(&self, rel: &str, contents: &str, message: &str, name: &str, email: &str) -> Oid {
        self.write(rel, contents);
        self.stage(rel);
        self.commit_as(name, email, message)
    }

    /// Replaces the repository's config with one that has only `entries`, so a test doesn't depend
    /// on a `user.name` / `user.email` in the global git config of whoever runs it.
    pub fn isolate_config(&self, entries: &[(&str, &str)]) {
        let file = self.path().join(".git").join("isolated-config");
        std::fs::write(&file, "").unwrap();
        let mut config = git2::Config::new().unwrap();
        config.add_file(&file, git2::ConfigLevel::Local, false).unwrap();
        for (key, value) in entries {
            config.set_str(key, value).unwrap();
        }
        self.repo.set_config(&mut config).unwrap();
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

        self.as_raw().branch("feature", &base, false).unwrap();
        self.commit_file("c.txt", "default branch change\n", "change on default");

        self.as_raw().set_head("refs/heads/feature").unwrap();
        self.as_raw()
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        self.commit_file("c.txt", "feature change\n", "change on feature");

        let theirs = self
            .as_raw()
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
        self.as_raw()
            .tag_lightweight(name, head.as_object(), false)
            .unwrap();
    }

    pub fn as_raw(&self) -> &git2::Repository {
        &self.repo
    }
}

/// A plain directory that is *not* a git repository.
pub fn plain_dir() -> TempDir {
    TempDir::new().unwrap()
}

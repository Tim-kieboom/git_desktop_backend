//! Tests for the async facade (`RepoHub` / `AsyncRepository`).
//!
//! # What the facade has to guarantee
//! - **Off the runtime:** git work runs on tokio's blocking pool, never on the async worker
//!   threads, so a slow git call can't stall other tasks.
//! - **One writer *or* many readers per repository** (a `RwLock` per repository): reads (status,
//!   diff, log, branches, tags, ...) run in parallel, writes (stage, commit, checkout, ...) run
//!   alone. Different repositories never block each other.
//! - **The lock follows the repository, not the handle:** every `AsyncRepository` for the same
//!   directory from the same `RepoHub` (clones, another `open`, another spelling of the path)
//!   shares one lock.
//! - **The lock is held until the git work is really done**, even if the awaiting future is
//!   dropped (a cancelled HTTP request). So the guard has to move *into* the blocking task.
//! - **Waiting is free:** tasks that wait for the lock don't occupy blocking threads (take the
//!   async lock *before* `spawn_blocking`), so a busy repository can't starve the others.
//! - **A panic in git work** is an error (`CoreError::Task`), not a poisoned or stuck lock.
//!
//! Most tests use the crate-private hooks `run_read` / `run_write` (closure gets the sync
//! `Repository`) to observe the locking with sleeps and an event log, and the rest drive the
//! public async API the way the daemon will.
use crate::{
    AsyncRepository, CoreError, CoreResult, DiffTarget, GitError, LogQuery, RepositoryHub,
    repository::Repository,
    tests::{TestRepo, plain_dir},
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinSet, time::timeout};

/// Everything async test code may wait for, with a limit so a locking bug fails instead of hanging.
async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    timeout(Duration::from_secs(20), fut)
        .await
        .expect("timed out: deadlock?")
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn open(hub: &RepositoryHub, repo: &TestRepo) -> AsyncRepository {
    hub.open(repo.path()).unwrap()
}

/// A hub with one freshly created repository (with an initial commit) opened in it.
fn setup() -> (RepositoryHub, TestRepo, AsyncRepository) {
    let hub = RepositoryHub::new();
    let repo = TestRepo::with_initial_commit();
    let handle = open(&hub, &repo);
    (hub, repo, handle)
}

// ----------------------------------------------------------------------------- observation tools

/// Counts how many closures are running at once and the highest that count ever got.
#[derive(Default)]
struct Probe {
    active: AtomicUsize,
    max: AtomicUsize,
}
impl Probe {
    /// Call from inside a git closure: occupies the probe for `millis`.
    fn busy(&self, millis: u64) {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(ms(millis));
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    fn max(&self) -> usize {
        self.max.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);
impl Log {
    fn push(&self, event: impl Into<String>) {
        self.0.lock().unwrap().push(event.into());
    }
    fn events(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    fn contains(&self, event: &str) -> bool {
        self.events().iter().any(|e| e == event)
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Read,
    Write,
}

/// Starts a job `name` on `repo` that logs "<name> start", sleeps `millis`, logs "<name> end".
/// Returns once the job is *running* (so it holds the lock), together with its task handle.
async fn start_job(
    repo: &AsyncRepository,
    kind: Kind,
    log: &Log,
    name: &str,
    millis: u64,
) -> tokio::task::JoinHandle<CoreResult<()>> {
    let (started_tx, started_rx) = oneshot::channel();
    let (repo, log, name) = (repo.clone(), log.clone(), name.to_owned());
    let handle = tokio::spawn(async move {
        let work = move |_: &Repository| -> CoreResult<()> {
            log.push(format!("{name} start"));
            let _ = started_tx.send(());
            std::thread::sleep(ms(millis));
            log.push(format!("{name} end"));
            Ok(())
        };
        match kind {
            Kind::Read => repo.run_read(work).await,
            Kind::Write => repo.run_write(work).await,
        }
    });
    within(started_rx).await.expect("the job never started");
    handle
}

// ==================================================================================== the basics

#[test]
fn the_facade_types_can_be_shared_between_tasks() {
    fn assert_shareable<T: Send + Sync + Clone + 'static>() {}
    assert_shareable::<AsyncRepository>();
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<RepositoryHub>();
}

#[tokio::test]
async fn git_work_runs_on_another_thread() {
    let (_hub, _repo, handle) = setup();
    let here = std::thread::current().id();

    let in_read = within(handle.run_read(|_| Ok(std::thread::current().id())))
        .await
        .unwrap();
    let in_write = within(handle.run_write(|_| Ok(std::thread::current().id())))
        .await
        .unwrap();
    assert_ne!(in_read, here);
    assert_ne!(in_write, here);
}

#[tokio::test]
async fn the_closure_gets_a_working_repository() {
    let (_hub, repo, handle) = setup();
    repo.write("new.txt", "x\n");

    let changed = within(handle.run_read(|r| r.status().map(|s| s.len())))
        .await
        .unwrap();
    assert_eq!(changed, 1);
}

#[tokio::test]
async fn results_and_errors_come_back_unchanged() {
    let (_hub, _repo, handle) = setup();

    assert_eq!(within(handle.run_write(|_| Ok(41 + 1))).await.unwrap(), 42);
    let err =
        within(handle.run_write(|_| -> CoreResult<()> { Err(GitError::NothingToCommit.into()) }))
            .await
            .unwrap_err();
    assert!(
        matches!(err, CoreError::Git(GitError::NothingToCommit)),
        "{err:?}"
    );
}

#[tokio::test]
async fn the_runtime_keeps_running_while_git_works() {
    let (_hub, _repo, handle) = setup();
    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker = {
        let ticks = ticks.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(ms(10)).await;
                ticks.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    // a single-threaded runtime: if the git work ran on it, the ticker would stand still
    within(handle.run_write(|_| {
        std::thread::sleep(ms(300));
        Ok(())
    }))
    .await
    .unwrap();
    ticker.abort();
    assert!(
        ticks.load(Ordering::SeqCst) >= 8,
        "only {} ticks",
        ticks.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn a_panicking_task_is_an_error_and_the_repository_stays_usable() {
    let (_hub, _repo, handle) = setup();

    let err = within(handle.run_write(|_| -> CoreResult<()> { panic!("boom") }))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Task(_)), "{err:?}");
    let err = within(handle.run_read(|_| -> CoreResult<()> { panic!("boom") }))
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Task(_)), "{err:?}");

    // the lock was released and is not poisoned
    assert_eq!(within(handle.run_write(|_| Ok(7))).await.unwrap(), 7);
    assert_eq!(within(handle.run_read(|_| Ok(8))).await.unwrap(), 8);
}

// ================================================================================== opening

#[test]
fn opening_something_that_is_not_a_repository_fails() {
    let hub = RepositoryHub::new();
    let plain = plain_dir();
    assert!(matches!(hub.open(plain.path()), Err(CoreError::Git(_))));
    assert!(hub.open(Path::new("/definitely/not/a/real/path")).is_err());
}

#[test]
fn a_repository_can_be_opened_from_its_registry_entry() {
    use crate::{RepoRegistry, storage::memory_storage::MemoryStorage};
    let hub = RepositoryHub::new();
    let repo = TestRepo::with_initial_commit();
    let registry = RepoRegistry::<MemoryStorage>::new().unwrap();
    registry.add(repo.path(), "demo").unwrap();
    let entry = registry.find(repo.path()).unwrap().unwrap();

    assert!(hub.open_entry(&entry).is_ok());
}

// =============================================================================== the locking

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_to_one_repository_never_overlap() {
    let (_hub, _repo, handle) = setup();
    let probe = Arc::new(Probe::default());

    let mut tasks = JoinSet::new();
    for _ in 0..8 {
        let (handle, probe) = (handle.clone(), probe.clone());
        tasks.spawn(async move {
            handle
                .run_write(move |_| {
                    probe.busy(20);
                    Ok(())
                })
                .await
        });
    }
    while let Some(done) = within(tasks.join_next()).await {
        done.unwrap().unwrap();
    }
    assert_eq!(probe.max(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_of_one_repository_run_in_parallel() {
    let (_hub, _repo, handle) = setup();
    let probe = Arc::new(Probe::default());

    let mut tasks = JoinSet::new();
    for _ in 0..4 {
        let (handle, probe) = (handle.clone(), probe.clone());
        tasks.spawn(async move {
            handle
                .run_read(move |_| {
                    probe.busy(150);
                    Ok(())
                })
                .await
        });
    }
    while let Some(done) = within(tasks.join_next()).await {
        done.unwrap().unwrap();
    }
    assert!(
        probe.max() >= 2,
        "reads were serialised (max {})",
        probe.max()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_waits_for_a_running_read() {
    let (_hub, _repo, handle) = setup();
    let log = Log::default();

    let read = start_job(&handle, Kind::Read, &log, "read", 150).await;
    let write = start_job(&handle, Kind::Write, &log, "write", 0).await; // returns once it runs
    within(read).await.unwrap().unwrap();
    within(write).await.unwrap().unwrap();

    assert_eq!(
        log.events(),
        ["read start", "read end", "write start", "write end"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_waits_for_a_running_write() {
    let (_hub, _repo, handle) = setup();
    let log = Log::default();

    let write = start_job(&handle, Kind::Write, &log, "write", 150).await;
    let read = start_job(&handle, Kind::Read, &log, "read", 0).await;
    within(write).await.unwrap().unwrap();
    within(read).await.unwrap().unwrap();

    assert_eq!(
        log.events(),
        ["write start", "write end", "read start", "read end"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn different_repositories_do_not_block_each_other() {
    let hub = RepositoryHub::new();
    let (repo_a, repo_b) = (
        TestRepo::with_initial_commit(),
        TestRepo::with_initial_commit(),
    );
    let (a, b) = (open(&hub, &repo_a), open(&hub, &repo_b));
    let log = Log::default();

    let slow = start_job(&a, Kind::Write, &log, "A", 200).await;
    let quick = start_job(&b, Kind::Write, &log, "B", 0).await;
    within(quick).await.unwrap().unwrap();
    assert!(
        !log.contains("A end"),
        "B had to wait for A: {:?}",
        log.events()
    );
    within(slow).await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_handle_of_the_same_repository_shares_one_lock() {
    let hub = RepositoryHub::new();
    let repo = TestRepo::with_initial_commit();
    let handles = [
        hub.open(repo.path()).unwrap(),
        hub.open(repo.path()).unwrap(),            // opened again
        hub.open(&repo.path().join(".")).unwrap(), // another spelling of the same directory
        hub.open(repo.path()).unwrap().clone(),    // a clone
    ];
    let probe = Arc::new(Probe::default());

    let mut tasks = JoinSet::new();
    for handle in handles {
        let probe = probe.clone();
        tasks.spawn(async move {
            handle
                .run_write(move |_| {
                    probe.busy(60);
                    Ok(())
                })
                .await
        });
    }
    while let Some(done) = within(tasks.join_next()).await {
        done.unwrap().unwrap();
    }
    assert_eq!(probe.max(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_the_waiting_future_does_not_release_the_lock_early() {
    let (_hub, _repo, handle) = setup();
    let log = Log::default();

    // the request is cancelled while its git work is still running ...
    let cancelled = start_job(&handle, Kind::Write, &log, "A", 200).await;
    cancelled.abort();

    // ... the next write must still wait for that work to finish
    let next = start_job(&handle, Kind::Write, &log, "B", 0).await;
    within(next).await.unwrap().unwrap();
    assert_eq!(log.events(), ["A start", "A end", "B start", "B end"]);
}

#[test]
fn waiting_for_the_lock_does_not_use_up_blocking_threads() {
    // only two blocking threads: if waiting writers sat on them, other repositories would starve
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let hub = RepositoryHub::new();
        let (repo_a, repo_b) = (
            TestRepo::with_initial_commit(),
            TestRepo::with_initial_commit(),
        );
        let (a, b) = (open(&hub, &repo_a), open(&hub, &repo_b));
        let log = Log::default();

        let holder = start_job(&a, Kind::Write, &log, "A1", 400).await;
        let waiters: Vec<_> = (2..=4)
            .map(|i| {
                let (a, log) = (a.clone(), log.clone());
                tokio::spawn(async move {
                    a.run_write(move |_| {
                        log.push(format!("A{i}"));
                        Ok(())
                    })
                    .await
                })
            })
            .collect();
        tokio::time::sleep(ms(100)).await; // let the waiters queue up behind A1

        let other = start_job(&b, Kind::Write, &log, "B", 0).await;
        within(other).await.unwrap().unwrap();
        assert!(
            !log.contains("A1 end"),
            "B waited for A's queue: {:?}",
            log.events()
        );

        within(holder).await.unwrap().unwrap();
        for waiter in waiters {
            within(waiter).await.unwrap().unwrap();
        }
    });
}

// ====================================================================== the async git API

#[tokio::test]
async fn status_and_diff_through_the_facade() {
    let (_hub, repo, handle) = setup();
    repo.write("README.md", "changed\n");
    repo.write("new.txt", "x\n");

    let status = within(handle.status()).await.unwrap();
    let paths: Vec<_> = status
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(paths, ["README.md", "new.txt"]);

    let diff = within(handle.file_diff(DiffTarget::Unstaged, "README.md"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(diff.hunks.len(), 1);
    let all = within(handle.diff_files(DiffTarget::Unstaged, None))
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn a_whole_workflow_through_the_facade() {
    let (_hub, repo, handle) = setup();
    repo.write("a.txt", "one\ntwo\n");

    within(handle.stage("a.txt")).await.unwrap();
    let id = within(handle.commit("add a")).await.unwrap();
    within(handle.create_tag("v1", None, Some("release")))
        .await
        .unwrap();
    within(handle.create_branch("feature", None)).await.unwrap();
    within(handle.checkout_branch("feature")).await.unwrap();

    let log = within(handle.log(LogQuery::default())).await.unwrap();
    assert_eq!(log.commits[0].id, id);
    assert_eq!(
        within(handle.commit_detail(id)).await.unwrap().files.len(),
        1
    );
    assert_eq!(within(handle.tags()).await.unwrap()[0].name, "v1");
    let branches = within(handle.branches()).await.unwrap();
    assert!(
        branches
            .iter()
            .find(|b| b.name == "feature")
            .unwrap()
            .is_head
    );
    let compared = within(handle.ahead_behind("feature", "v1")).await.unwrap();
    assert_eq!((compared.ahead, compared.behind), (0, 0));

    let other = branches
        .iter()
        .find(|b| b.name != "feature")
        .unwrap()
        .name
        .clone();
    within(handle.checkout_branch(&other)).await.unwrap();
    within(handle.delete_branch("feature", false))
        .await
        .unwrap();
    within(handle.delete_tag("v1")).await.unwrap();
    assert!(within(handle.tags()).await.unwrap().is_empty());
}

#[tokio::test]
async fn hunks_through_the_facade() {
    let (_hub, repo, handle) = setup();
    let base: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    repo.commit_file("f.txt", &base, "base");
    repo.write(
        "f.txt",
        &base
            .replace("line 3\n", "CHANGED 3\n")
            .replace("line 28\n", "CHANGED 28\n"),
    );

    let hunks = within(handle.file_diff(DiffTarget::Unstaged, "f.txt"))
        .await
        .unwrap()
        .unwrap()
        .hunks;
    assert_eq!(hunks.len(), 2);
    within(handle.stage_hunk("f.txt", &hunks[0])).await.unwrap();
    let staged = within(handle.file_diff(DiffTarget::Staged, "f.txt"))
        .await
        .unwrap()
        .unwrap()
        .hunks;
    assert_eq!(staged.len(), 1);
    within(handle.unstage_hunk("f.txt", &staged[0]))
        .await
        .unwrap();
    assert!(
        within(handle.file_diff(DiffTarget::Staged, "f.txt"))
            .await
            .unwrap()
            .is_none()
    );
    within(handle.stage("f.txt")).await.unwrap();
    within(handle.unstage("f.txt")).await.unwrap();
}

#[tokio::test]
async fn git_errors_arrive_as_core_errors() {
    let (_hub, _repo, handle) = setup();

    let err = within(handle.checkout_branch("nope")).await.unwrap_err();
    assert!(
        matches!(err, CoreError::Git(GitError::BranchNotFound(_))),
        "{err:?}"
    );
    let err = within(handle.commit("nothing staged")).await.unwrap_err();
    assert!(
        matches!(err, CoreError::Git(GitError::NothingToCommit)),
        "{err:?}"
    );
}

// ======================================================== the reason the lock exists: real races

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn staging_many_files_at_once_loses_nothing() {
    let (_hub, repo, handle) = setup();
    for i in 0..20 {
        repo.write(&format!("file{i:02}.txt"), "x\n");
    }

    // every stage rewrites the index: without the write lock these collide on index.lock
    let mut tasks = JoinSet::new();
    for i in 0..20 {
        let handle = handle.clone();
        tasks.spawn(async move { handle.stage(format!("file{i:02}.txt")).await });
    }
    while let Some(done) = within(tasks.join_next()).await {
        done.unwrap().unwrap();
    }

    let status = within(handle.status()).await.unwrap();
    assert_eq!(status.len(), 20);
    assert!(
        status
            .iter()
            .all(|e| e.staged.is_some() && e.unstaged.is_none()),
        "{status:#?}"
    );
    within(handle.commit("all of them")).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_and_writes_mixed_together_never_fail() {
    let (_hub, repo, handle) = setup();
    for i in 0..10 {
        repo.write(&format!("w{i}.txt"), "x\n");
    }

    let mut tasks = JoinSet::new();
    for i in 0..10 {
        let writer = handle.clone();
        tasks.spawn(async move { writer.stage(format!("w{i}.txt")).await.map(|_| ()) });
        let reader = handle.clone();
        tasks.spawn(async move { reader.status().await.map(|_| ()) });
        let reader = handle.clone();
        tasks.spawn(async move { reader.log(LogQuery::default()).await.map(|_| ()) });
    }
    while let Some(done) = within(tasks.join_next()).await {
        done.unwrap().unwrap();
    }
    assert_eq!(within(handle.status()).await.unwrap().len(), 10);
}

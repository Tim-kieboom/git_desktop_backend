# gitdesk — Generic Git Backend Daemon in Rust

> Working name. A host-agnostic git engine exposed as a local daemon (HTTP + WebSocket), with release-notes generation as the flagship feature and optional provider integrations (GitHub, Gitea, ...).

## 1. Goals

- Build a "GitHub Desktop backend": a long-running service that manages local repos and exposes git operations over an API.
- Work with **any** git remote (GitHub, Gitea, Forgejo, GitLab, Codeberg, bare SSH repos). Core never depends on a specific host.
- Ship a demoable flagship feature: **release notes / changelog generation** between two tags.
- Showcase Rust skills: async (tokio/axum), FFI-backed libraries (`git2`), trait-based abstraction, file watching, error handling, testing against real repos.

### Non-goals (for now)

- Full merge-conflict resolution UI.
- Hosting/serving git repos (this is a client engine, not a git server).
- Multi-user auth; the daemon is local, single user.

## 2. Architecture

```
┌──────────────────────────────────────────────┐
│ Clients: CLI (clap)  ·  Web UI (optional)    │
└───────────────┬──────────────────────────────┘
                │ HTTP (REST) + WebSocket events
┌───────────────▼──────────────────────────────┐
│ Layer 4: API server (axum)                   │
│  routes · DTOs · error mapping · event hub   │
└───────────────┬──────────────────────────────┘
┌───────────────▼──────────────────────────────┐
│ Layer 3: Features                            │
│  changelog generator · conventional commits  │
└───────────────┬──────────────────────────────┘
┌───────────────▼──────────────┬───────────────┐
│ Layer 1: Git engine          │ Layer 2:      │
│  (host-agnostic, git2)       │ Provider trait│
│  status · diff · stage ·     │  GitHub       │
│  commit · branch · history · │  Gitea        │
│  tags · fetch/push/pull      │  Generic(noop)│
└──────────────────────────────┴───────────────┘
```

### Layer 1 — Git engine (host-agnostic)

Everything local and protocol-level. Knows nothing about GitHub/Gitea.

- Repo registry: add/remove/list repos, persisted to disk (SQLite or TOML).
- Status, diff (file + hunk level), staging/unstaging (including individual hunks), commit.
- Branches, tags, history/log with pagination, commit details.
- Fetch / push / pull over SSH and HTTPS, with a pluggable credential provider.
- Library choice: `git2` (libgit2) for completeness; revisit `gitoxide` later for read paths.
- All blocking `git2` calls wrapped in `tokio::task::spawn_blocking` behind an async facade.

### Layer 2 — Provider abstraction

Optional host-specific features behind a trait. The core still works if no provider matches.

```rust
trait GitProvider {
    fn name(&self) -> &str;
    fn matches(&self, remote_url: &Url) -> bool;
    fn commit_url(&self, repo: &RemoteRef, sha: &str) -> Url;
    async fn list_pull_requests(&self, repo: &RemoteRef) -> Result<Vec<PullRequest>>;
    async fn resolve_references(&self, repo: &RemoteRef, refs: &[Ref]) -> Result<Vec<LinkedItem>>;
}
```

- Implementations: `GitHubProvider`, `GiteaProvider` (also covers Forgejo), `GenericProvider` (no-op fallback).
- Provider auto-detected from remote URL; overridable in config for self-hosted instances (e.g. `git.mycompany.nl` → Gitea).

### Layer 3 — Features

- **Changelog generator:** walk commits between two refs (tags/SHAs), parse conventional commits (`feat:`, `fix:`, `BREAKING CHANGE`), group by type, render Markdown/JSON.
- Uses provider (if any) to turn `#123` into PR/issue links and commit SHAs into URLs.
- Configurable via a `gitdesk.toml` (group names, ordering, ignored types, template).

### Layer 4 — API & events

- REST endpoints under `/api/v1/` (see section 4).
- WebSocket `/api/v1/events` pushes repo-change events (file watcher → debounced status updates).
- Consistent error model (`thiserror` in libs, `anyhow` at edges, problem-JSON style responses).
- OpenAPI spec generated (e.g. `utoipa`).

## 3. Workspace layout

```
gitdesk/
├── Cargo.toml                 # workspace
├── crates/
│   ├── gitdesk-core/          # Layer 1: git engine
│   ├── gitdesk-provider/      # Layer 2: trait + GitHub/Gitea/Generic
│   ├── gitdesk-changelog/     # Layer 3: parsing + rendering
│   ├── gitdesk-daemon/        # Layer 4: axum server, event hub, config
│   └── gitdesk-cli/           # CLI client
├── tests/                     # integration tests (temp repos, Gitea container)
├── docker/                    # docker-compose with Gitea for testing
├── docs/                      # design notes, ADRs, OpenAPI
├── plan.md
└── README.md
```

## 4. Draft API surface

| Method | Path                                     | Purpose                         |
|--------|------------------------------------------|---------------------------------|
| GET    | `/repos`                                 | List registered repos           |
| POST   | `/repos`                                 | Register a repo (path or clone) |
| GET    | `/repos/{id}/status`                     | Working tree status             |
| GET    | `/repos/{id}/diff`                       | Diff (staged/unstaged, per file)|
| POST   | `/repos/{id}/stage`                      | Stage files/hunks               |
| POST   | `/repos/{id}/unstage`                    | Unstage files/hunks             |
| POST   | `/repos/{id}/commit`                     | Create commit                   |
| GET    | `/repos/{id}/log`                        | Paginated history               |
| GET    | `/repos/{id}/branches`                   | List branches                   |
| POST   | `/repos/{id}/branches`                   | Create/checkout/delete branch   |
| GET    | `/repos/{id}/tags`                       | List tags                       |
| POST   | `/repos/{id}/fetch` `/pull` `/push`      | Remote operations               |
| GET    | `/repos/{id}/changelog?from=..&to=..`    | Generate release notes          |
| WS     | `/events`                                | Live repo change events         |

## 5. Key design decisions (to document in README)

1. **Host-agnostic core, optional providers** — why the trait boundary exists and what happens with unknown hosts.
2. **Async over blocking libgit2** — `spawn_blocking`, per-repo locking to avoid concurrent writes corrupting state.
3. **Debounced file watching** — `notify` events coalesced before emitting to WebSocket clients.
4. **Credential handling** — SSH agent first, then token/keychain, never store plaintext secrets.
5. **Testing strategy** — real temp repos via `tempfile`, golden-file tests for the changelog, Gitea in Docker for provider tests.

## 6. Tech stack

| Concern          | Choice                                   |
|------------------|------------------------------------------|
| Async runtime    | `tokio`                                  |
| HTTP / WS        | `axum`                                   |
| Git              | `git2` (revisit `gitoxide`)              |
| File watching    | `notify` + debouncer                     |
| Serialization    | `serde`, `serde_json`, `toml`            |
| Persistence      | `rusqlite` or `sqlx` (SQLite)            |
| Errors           | `thiserror`, `anyhow`                    |
| Logging          | `tracing`, `tracing-subscriber`          |
| CLI              | `clap`                                   |
| HTTP client      | `reqwest`                                |
| OpenAPI          | `utoipa`                                 |
| Testing          | `tempfile`, `insta`, `testcontainers`    |

---

# TODO

## Phase 0 — Setup
- [ ] Pick final project name
- [ ] Create Cargo workspace with the five crates
- [ ] Set up CI (fmt, clippy, test) on GitHub Actions
- [ ] Add `tracing` + basic config loading
- [ ] Write initial README skeleton and `docs/` folder

## Phase 1 — Git engine basics (`gitdesk-core`)
- [ ] Repo registry (add/remove/list, persisted)
- [ ] Open repo + `status` (staged/unstaged/untracked)
- [ ] Diff for a file (unified + structured hunks)
- [ ] Stage / unstage whole files
- [ ] Stage / unstage individual hunks
- [ ] Create commit (author from git config)
- [ ] Async facade with `spawn_blocking` + per-repo lock
- [ ] Integration tests against temp repos

## Phase 2 — History, branches, tags
- [ ] Paginated commit log with filters (path, author, range)
- [ ] Commit detail view (diff against parent)
- [ ] List/create/checkout/delete branches
- [ ] List/create tags
- [ ] Ahead/behind computation vs upstream

## Phase 3 — API daemon (`gitdesk-daemon`)
- [ ] axum server skeleton, config, graceful shutdown
- [ ] Routes for repos, status, diff, stage, commit, log, branches, tags
- [ ] Unified error model + HTTP mapping
- [ ] OpenAPI spec generation
- [ ] Minimal CLI client (`gitdesk status`, `gitdesk log`, ...)

## Phase 4 — Changelog generator (`gitdesk-changelog`)
- [ ] Conventional commit parser (types, scopes, breaking changes)
- [ ] Commit range walker between two refs/tags
- [ ] Grouping and ordering rules
- [ ] Markdown renderer
- [ ] JSON renderer
- [ ] `gitdesk.toml` config (groups, ignore rules, template)
- [ ] Golden-file tests with `insta`
- [ ] `GET /changelog` endpoint + `gitdesk changelog v1.0.0..v1.1.0` CLI command
- [ ] **Milestone: shippable v0.1** (Phases 1–4)

## Phase 5 — Live events
- [ ] File watcher with debouncing per repo
- [ ] Event hub (broadcast channel) in daemon
- [ ] WebSocket `/events` endpoint
- [ ] CLI `gitdesk watch` that streams status changes

## Phase 6 — Remote operations
- [ ] Credential provider abstraction (SSH agent, token, keychain)
- [ ] Fetch with progress reporting
- [ ] Pull (fast-forward + merge, clean errors on conflict)
- [ ] Push with progress and rejection handling
- [ ] Clone into a registered repo
- [ ] Progress streamed over WebSocket

## Phase 7 — Providers (`gitdesk-provider`)
- [ ] Define `GitProvider` trait + remote URL parsing (ssh/https/scp-style)
- [ ] `GenericProvider` fallback
- [ ] `GitHubProvider` (commit URLs, PRs, issue refs)
- [ ] `GiteaProvider` (also works for Forgejo)
- [ ] Config override for self-hosted instances
- [ ] Docker-compose Gitea for integration tests (`testcontainers`)
- [ ] Changelog: enrich entries with PR/issue/commit links via provider

## Phase 8 — Polish & portfolio
- [ ] Architecture diagram in README
- [ ] Documented design decisions / ADRs (section 5)
- [ ] Demo GIF or asciinema recording
- [ ] Docker image + `docker-compose.yml` for the daemon
- [ ] Benchmarks on a large repo (status, log, changelog)
- [ ] Release binaries via GitHub Actions
- [ ] Optional: small web UI (status, diff, commit, changelog view)

## Stretch ideas
- [ ] Merge conflict detection and reporting
- [ ] Interactive rebase / amend / cherry-pick
- [ ] Stash support
- [ ] Switch read paths to `gitoxide` and benchmark against `git2`
- [ ] GitLab provider
- [ ] Changelog served as RSS feed / static site export

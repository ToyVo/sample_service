# Sample xitdb backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** xitdb becomes a third selectable database backend (samples + auth) in `packages/api`, chosen via `DATABASE_URL`.

**Architecture:** First generalize both storage traits from `diesel::result::Error` to a local `StoreError` (diesel query bodies stay, converting via `From`). Then add one new module with an `XitdbConnection` implementing both traits against xitdb collections, wire `xitdb://` routing behind a new cargo feature, and extend the test suites with a third backend leg.

**Tech Stack:** Rust edition 2024 (nightly toolchain), Diesel 2.2 (existing backends), xitdb 0.2 (crates.io, new), axum handlers unchanged.

**Spec:** `docs/superpowers/specs/2026-11-05-sample-xitdb-backend-design.md`

## Global Constraints

- Repo gates per task: `cargo nextest run -p api --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`.
- Workspace clippy lints are deny-by-default (pedantic, nursery, `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `as_conversions`, arithmetic); new `tests/*.rs` files carry the same `#![allow(clippy::expect_used)] #![allow(clippy::panic)] #![allow(clippy::unwrap_used)]` header the existing test files use.
- No behavior change for sqlite/postgres: their suites must pass unmodified in expectation after every task.
- No new runtime surface beyond the feature: `xitdb-backend = ["dep:xitdb", "server"]`, xitdb 0.2 in workspace.dependencies.
- Do not publish anything; no version bumps (app is unreleased 0.1.0).

## Review Focus

- Double insert of the same username id returns Conflict, never a silent overwrite (pinned by Task 2's duplicate-username test).
- `fetch_samples` offset/limit order matches created_at ascending exactly like SQL `ORDER BY` (pinned by Task 2's pagination-order test).
- Passkey listing is oldest-first on xitdb (pinned by Task 2's passkey-order test).
- Bare `./dev.xitdb` paths route to xitdb, never sqlite (pinned by Task 3's routing test asserting `backend_name() == "xitdb"`).
- A garbage file opened as xitdb fails `establish_connection` cleanly, never panics (pinned by Task 3's corrupt-file test).

## Review Focus

- Double insert of the same username id returns Conflict, never a silent overwrite (pinned by Task 2's duplicate-username test).
- `fetch_samples` offset/limit order matches created_at ascending on xitdb exactly like SQL `ORDER BY` (pinned by Task 2's pagination-order test with explicit distinct timestamps).
- Passkey listing is oldest-first on xitdb (pinned by Task 2's passkey-order test).
- Bare `./dev.xitdb` paths route to xitdb, never sqlite (pinned by Task 3's routing test asserting `!is_sqlite_url` is unnecessary — it asserts `backend_name() == "xitdb"` for the bare path).
- A garbage file opened as xitdb fails `establish_connection` cleanly, never panics (pinned by Task 3's corrupt-file test).

## File structure

- Modify: `packages/api/src/store.rs` (StoreError + trait + free-fn mapping).
- Modify: `packages/api/src/auth/store.rs` (trait + macro error types; diesel query bodies unchanged).
- Modify: `packages/api/src/auth/mod.rs` (`From<DieselError>` → `From<StoreError>`, match arms at UniqueViolation/NotFound sites).
- Create: `packages/api/src/xitdb_backend.rs` (connection, both impls, model mapping, error conversion).
- Modify: `packages/api/src/lib.rs` (gate the module decl), `packages/api/src/state.rs` (detection, path helper, `new_xitdb`, routing arm), `packages/api/Cargo.toml` + workspace `Cargo.toml` (dep + feature).
- Tests: new `packages/api/tests/xitdb_backend.rs`; extend `packages/api/tests/dual_backend.rs`, the auth suite, `state.rs` unit tests.

## Task 1: Backend-agnostic error type

**Files:**
- Modify: `packages/api/src/store.rs` (add `StoreError`, swap trait + free-fn error types)
- Modify: `packages/api/src/auth/store.rs` (swap `QueryResult` → `Result<_, StoreError>` in trait + macro; bodies unchanged — diesel errors convert via `?`)
- Modify: `packages/api/src/auth/mod.rs` (`From<DieselError> for AuthError` → `From<StoreError> for AuthError`; match arms `DieselError::DatabaseError(UniqueViolation, _)` → `StoreError::Conflict(_)`, `DieselError::NotFound` → `StoreError::NotFound`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum StoreError { NotFound, Conflict(String), Backend(String) }` with `Display`, `std::error::Error`, and `From<diesel::result::Error>` (`NotFound`→`NotFound`, `DatabaseError(UniqueViolation,_)`→`Conflict(message)`, else→`Backend(details)`); traits speaking only `StoreError`.

- [ ] **Step 1: Write failing mapping tests** in `packages/api/src/store.rs` test module (create it if absent):

```rust
#[test]
fn diesel_not_found_maps_to_store_not_found() {
    let err = StoreError::from(diesel::result::Error::NotFound);
    assert!(matches!(err, StoreError::NotFound));
}
```

plus UniqueViolation→Conflict and a generic error→Backend case.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p api --features server store::tests`
Expected: FAIL, `StoreError` does not exist.

- [ ] **Step 3: Add `StoreError`, its impls, and swap all trait/free-fn/macro/auth-flow error types per Interfaces**

Diesel query bodies stay byte-identical; only error positions change. Survey `auth/` for any other `DieselError` match sites beyond `mod.rs` and convert them too.

- [ ] **Step 4: Run gates**

Run: `cargo nextest run -p api --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
Expected: all green (sqlite always runs; postgres runs-or-skips per environment).

- [ ] **Step 5: Commit**

```bash
git add packages/api/src/store.rs packages/api/src/auth/
git commit -m "refactor: backend-agnostic StoreError for storage traits"
```

## Task 2: xitdb backend module

**Files:**
- Modify: `packages/api/Cargo.toml` (`xitdb = {optional = true, workspace = true}`), workspace `Cargo.toml` (`xitdb = "0.2"` in workspace.dependencies), `packages/api/src/lib.rs` (`#[cfg(feature = "xitdb-backend")] mod xitdb_backend;`)
- Modify: `packages/api/Cargo.toml` features (`xitdb-backend = ["dep:xitdb", "server"]`)
- Create: `packages/api/src/xitdb_backend.rs`
- Create: `packages/api/tests/xitdb_backend.rs` (header allows like `dual_backend.rs`, `required-features = ["xitdb-backend"]`)

**Interfaces:**
- Consumes: `StoreError` + converted traits from Task 1.
- Produces: `pub struct XitdbConnection { db: Database<BufferedFileCore, Sha1Hasher> }` with `impl SampleConnection` + `impl AuthConnection`; `From<xitdb::error::Error>` mapping `NotFound`/`KeyNotFound`→`NotFound`, rest→`Backend`.

Data model (fixed rules, field lists per `models/common.rs` row types): one top-level `HashMap`, one key per table (`samples`, `samples_by_created`, `users`, `usernames`, `passkeys`, `passkeys_by_credential`, `passkey_lists`); rows are nested `HashMap<field, value>` with strings as `Bytes`, datetimes as micros `Int`, absent key meaning `None`; `samples_by_created` is a `SortedMap` keyed by big-endian micros + `/` + id; `usernames` maps username→id; `passkey_lists`/user-id holds a per-user `LinkedArrayList` of passkey ids. Uniqueness via check-then-insert (single-`Mutex` discipline makes it race-free). `run_migrations` returns `Ok(())` with a doc comment (collections materialize on write). First survey `token.rs`: if sessions are stored, map them the same way; stateless JWTs need no mapping — record the finding in the code comment.

- [ ] **Step 1: Write failing backend round-trip tests** in `packages/api/tests/xitdb_backend.rs` (temp-file DBs, never `./dev.db`): insert→fetch sample, missing id → `NotFound`, duplicate username → `Conflict`, paginated fetch order with explicit distinct timestamps, passkey oldest-first.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p api --features xitdb-backend --test xitdb_backend`
Expected: FAIL, `xitdb_backend` module does not exist.

- [ ] **Step 3: Implement the module** per Interfaces (samples CRUD + users/passkeys mapping; `backend_name()` returns `"xitdb"`).

- [ ] **Step 4: Run gates**

Run: `cargo nextest run -p api --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
Expected: all green, new tests pass.

- [ ] **Step 5: Commit**

```bash
git add packages/api/src/xitdb_backend.rs packages/api/src/lib.rs packages/api/tests/xitdb_backend.rs packages/api/Cargo.toml Cargo.toml Cargo.lock
git commit -m "feat: xitdb storage backend"
```

## Task 3: Routing, state, and URL handling

**Files:**
- Modify: `packages/api/src/state.rs` (`is_xitdb_url`, `xitdb_path`, `AppState::new_xitdb`, routing arm checked after postgres and before the sqlite fallback, `backend_name` passthrough which already works via the trait)

**Interfaces:**
- Consumes: `XitdbConnection` from Task 2.
- Produces: `xitdb://…`, `xitdb:…`, and bare `*.xitdb` paths route to xitdb; open failures map to `ConnectionError::BadConnection`; `fsync` on, truncate off.

- [ ] **Step 1: Write failing URL tests** in `state.rs` test module: `is_xitdb_url("xitdb://./dev.xitdb")`, `xitdb:./dev.xitdb`, bare `./dev.xitdb` routes to `"xitdb"` via `establish_connection_from_url` (temp file, cleaned up), garbage file errors cleanly, `mysql://` still errors.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p api --features xitdb-backend state::tests`
Expected: FAIL, `is_xitdb_url` does not exist.

- [ ] **Step 3: Implement routing + `new_xitdb`** per Interfaces (gate the arm and constructor on the feature so `--features server` builds unchanged).

- [ ] **Step 4: Run gates**

Run: `cargo nextest run -p api --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add packages/api/src/state.rs
git commit -m "feat: xitdb DATABASE_URL routing"
```

## Task 4: Suite integration and full verification

**Files:**
- Modify: `packages/api/tests/dual_backend.rs` (third leg: temp-file xitdb, always runs; `backend_name() == "xitdb"` assertion mirroring the sqlite leg)
- Modify: the auth test suite the same way its harness parametrizes backends
- Modify: fresh-file init coverage equivalent to `migrations.rs` for xitdb (open → `run_migrations` → empty reads)

**Interfaces:**
- Consumes: everything above.
- Produces: releasable tree (no publish step).

- [ ] **Step 1: Add the third legs and init coverage**

- [ ] **Step 2: Run the backend suites**

Run: `cargo nextest run -p api --all-features --test dual_backend --test xitdb_backend`
Expected: all green (postgres legs run-or-skip per environment).

- [ ] **Step 3: Run full gates**

Run: `cargo nextest run --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
Expected: all green workspace-wide.

- [ ] **Step 4: Commit**

```bash
git add packages/api/tests/
git commit -m "test: xitdb third backend leg"
```

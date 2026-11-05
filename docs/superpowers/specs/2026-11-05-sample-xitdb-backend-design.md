# Design: xitdb as a third database backend

Date: 2026-11-05. Context: [xit-vcs/xitdb#6](https://github.com/xit-vcs/xitdb/issues/6)
tracks the Rust port; this makes xitdb (0.2.0, crates.io) a full-parity
backend in the sample API alongside SQLite and Postgres — samples and
auth, selectable via `DATABASE_URL`.

## Goal

`DATABASE_URL=xitdb://./dev.xitdb` boots the API on xitdb with zero
changes to routes, handlers, or business logic. All existing suites keep
passing unmodified in expectation; new suites prove the third backend.

Non-goals: changing the file format or xitdb crate; async xitdb API
(sync under the existing short lock matches current diesel behavior);
`no_std`; touching `establish_connection`'s `diesel::ConnectionError`
(it stays: startup code, not the operation abstraction).

## Error abstraction

New shared error in `packages/api/src/store.rs`, reused by auth:

```rust
pub enum StoreError {
    NotFound,
    Conflict(String),
    Backend(String),
}
```

`Display` renders the detail (`NotFound` → `"not found"`); implement
`std::error::Error`. Both traits swap their diesel error types for it:
`SampleConnection` methods return `Result<_, StoreError>`, auth methods
return `Result<_, StoreError>` instead of `QueryResult`. Conversions:
`From<diesel::result::Error>` maps `NotFound` → `NotFound`,
`DatabaseError(UniqueViolation, _)` → `Conflict(info)`, everything else
→ `Backend(details)`; `From<xitdb::error::Error>` (defined in the xitdb
module) maps `NotFound`/`KeyNotFound` → `NotFound`, rest →
`Backend(details)`. Free functions in `store.rs` match on `StoreError`;
auth flows that match diesel errors directly are converted at their
(match-site survey happens during implementation; the suite locks
behavior).

## xitdb backend module

New `packages/api/src/xitdb_backend.rs`, gated behind a new opt-in
`xitdb-backend` cargo feature (`dep:xitdb` 0.2.0; implies nothing, enabled
alongside `server`):

```rust
pub struct XitdbConnection {
    db: Database<BufferedFileCore, Sha1Hasher>,
}
```

`Database` is `Send` (interior mutability only), so the existing
`Arc<Mutex<dyn SampleConnection>>` pattern holds unchanged; handlers
keep locking briefly with no await across the guard. Hand-written
`impl SampleConnection` + `impl AuthConnection` (no macro: the mapping
logic differs per method).

Data model — one top-level `HashMap`, fixed keys per table, each row a
nested `HashMap<field, value>`:

- `samples`: key = id string; fields `name`/`description`/`status`/
  `tags`/`image_uri` as `Bytes`, datetimes as micros `Int`, absent key
  means `None`. Secondary index `samples_by_created`: `SortedMap` keyed
  by big-endian micros + `/` + id (lexicographic order is chronological;
  `created_at` never changes on update so the key is stable), value = id.
  `fetch_samples` seeks by rank for offset/limit.
- `users`: key = id string; `usernames`: `SortedMap` username → id (lookup
  plus uniqueness check; the surrounding `Mutex` makes check-then-insert
  race-free).
- `passkeys`: key = id string; `passkeys_by_credential`: `HashMap`
  credential-id → id (uniqueness check); per-user insertion order via
  `LinkedArrayList` under `passkey_lists`/user-id, appended on insert
  (oldest-first reads).
- Sessions/tokens: only if the implementation survey finds stored session
  state (JWTs are expected to be stateless; confirm, then either map or
  record as out of scope).

`backend_name()` returns `"xitdb"`. `run_migrations` is a documented
no-op returning `Ok(())`: collections materialize on first write and
missing keys already read as `NotFound`/empty, so a fresh file behaves
like a freshly migrated database.

## Routing and state

- `is_xitdb_url`: `xitdb://…`, `xitdb:…`, and bare paths ending `.xitdb`.
  Checked after postgres, before the sqlite fallback (which otherwise
  claims every schemeless path). `sqlite_path`-style helper strips the
  prefix; open via `BufferedFileCore` (truncate off — never destroy an
  existing file), `fsync` on.
- `AppState::new_xitdb(conn)`; routing errors map open failures to
  `ConnectionError::BadConnection`. Default `DATABASE_URL` unchanged.
- Unit tests for detection/stripping/routing in `state.rs` test-module
  style.

## Tests

- `dual_backend.rs` gains a third leg: temp-file xitdb, always runs
  (like sqlite, unlike conditional postgres).
- Auth suite extended the same way its harness already parametrizes
  backends (follow the existing file's pattern).
- Fresh-file init test: open → `run_migrations` → empty reads
  (`NotFound`/empty vec), mirroring `migrations.rs`.
- Full gates unchanged: `cargo test`, clippy pedantic+nursery deny,
  rustfmt; xitdb lockfile entries via crates.io (no path dependency).

## Ordering and versioning

1. `StoreError` + diesel `From` impl + trait/callers conversion (both
   diesel backends green throughout).
2. `xitdb_backend.rs` module + impls.
3. Routing, `AppState::new_xitdb`, feature flag wiring.
4. Tests (third legs + routing + init).
5. No version bump needed (app is 0.1.0, unreleased); no publish step.

## Risks

- Auth flows matching diesel errors outside the traits (survey at
  implementation; suite locks behavior).
- Passkey/token stored state (confirm during implementation; stateless
  JWTs need no mapping).
- Unique-violation fidelity: check-then-insert under one `Mutex` is
  equivalent given the app's single-connection discipline (documented in
  `state.rs`); no concurrent-writer path exists to break it.

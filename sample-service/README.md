# Development

Your new jumpstart project includes basic organization with an organized `assets` folder and a `components` folder.
If you chose to develop with the router feature, you will also have a `views` folder.

```
project/
├─ assets/ # Any assets that are used by the app should be placed here
├─ src/
│  ├─ main.rs # The entrypoint for the app. It also defines the routes for the app.
│  ├─ components/
│  │  ├─ mod.rs # Defines the components module
│  │  ├─ hero.rs # The Hero component for use in the home page
│  │  ├─ echo.rs # The echo component uses server functions to communicate with the server
│  ├─ views/ # The views each route will render in the app.
│  │  ├─ mod.rs # Defines the module for the views route and re-exports the components for each route
│  │  ├─ blog.rs # The component that will render at the /blog/:id route
│  │  ├─ home.rs # The component that will render at the / route
├─ Cargo.toml # The Cargo.toml file defines the dependencies and feature flags for your project
```

### Serving Your App

Run the following command in the root of your project to start developing with the default platform:

```bash
dx serve --platform web
```

To run for a different platform, use the `--platform platform` flag. E.g.

```bash
dx serve --platform desktop
```

## Databases (SQLite or Postgres)

The server picks a backend from `DATABASE_URL` (see `secretspec.toml`):

- SQLite (default): `sqlite://./dev.db` — a file alongside this README.
- Postgres: `postgresql://postgres@localhost:5433/sample` — a local server
  whose data lives in `./dev.postgresql/` (gitignored, next to `dev.db`).

Migrations run automatically on startup (embedded via `diesel_migrations`),
so a fresh `dev.db` or fresh `dev.postgresql/` just works.

### Local Postgres

Postgres binaries come from the nix dev shell (`postgresql` in `flake.nix`),
so run everything below inside `nix develop` from the repo root.

```bash
cd sample-service

# Start local Postgres (initdb on first run, port 5433, db `sample`)
./scripts/postgres.sh start
# or: cargo script postgres-start

# Serve against Postgres (instead of the default SQLite)
DATABASE_URL=postgresql://postgres@localhost:5433/sample dx serve
# or: cargo script dev-postgres

# Other helpers
./scripts/postgres.sh status   # pg_ctl status + pg_isready
./scripts/postgres.sh logs     # tail dev.postgresql/logfile
./scripts/postgres.sh stop     # stop the server (keeps data)
./scripts/postgres.sh clean    # stop + delete dev.postgresql/ (fresh next start)
```

### Migrations & schemas

- SQLite: `migrations/sqlite/` → `src/schemas/sqlite.rs` (`diesel.sqlite.toml`)
- Postgres: `migrations/postgresql/` → `src/schemas/postgresql.rs`
  (`diesel.postgresql.toml`). Postgres DDL uses `BYTEA` where SQLite uses
  `BLOB` for the `id` column; both map to Diesel `Binary` so the same `UUID`
  wrapper works for either backend. Keep the two `up.sql` files in sync
  (same columns/order).

```bash
# Regenerate API types only (no DB needed)
cargo script codegen

# Apply one backend + regenerate its schema file
cargo script db-migrate-sqlite    # needs nothing running
cargo script db-migrate-postgres  # needs postgres-start first

# Both backends + API types
cargo script codegen-all
```

Tests cover both backends (`tests/dual_backend.rs`): SQLite runs in-memory
always; Postgres runs when `postgresql://postgres@localhost:5433/sample`
(or `$TEST_POSTGRES_URL`) is reachable, otherwise it skips.

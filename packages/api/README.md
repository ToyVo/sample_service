# API

This crate contains the shared backend: storage, Axum routers, and
observability. It has no Dioxus dependency — the frontend lives in `ui`
plus the `web` / `desktop` / `mobile` shells.

## Backend modules (all `server`-gated)

- `state` — `AppState` (a single boxed storage abstraction) plus
  `DATABASE_URL` routing.
- `routes` — the `CodegenApi` implementation.
- `fallback` — transport 404s: JSON for `/api*`, HTML otherwise.
- `store` — backend-agnostic CRUD over the `SampleConnection` trait.
- `models` / `schemas` — Diesel row types and table definitions for SQLite
  and Postgres.
- `generated` — OpenAPI-generated types, client, and axum server
  (regenerate with `cgs regen` from the repo root).
- `docs` — `GET /api` serving the embedded spec. The interactive
  console and `/docs` reference page are Dioxus routes in the `web` crate
  (`views/console.rs`, `views/docs.rs`), rendered from that same spec.
- `telemetry` — OpenTelemetry traces, metrics, and logs (see below).

## Observability

`api::init_telemetry()` (called by the server binary) exports traces,
metrics, and logs over OTLP/HTTP. Every request gets an `http.server.request`
span (with `traceparent` propagation), an `http.server.request.duration`
histogram point, and a log event; storage calls add `store.*` child spans
with `db.system.name` (`sqlite`/`postgresql`).

Run the local stack from the repo root, then serve against it:

```sh
cgs run otel up   # collector, Tempo, Prometheus, Grafana
cgs dev-otel      # dx serve with telemetry on (defaults suffice)
```

- Grafana (Tempo + Prometheus provisioned): http://localhost:3000
- Prometheus: http://localhost:9090
- Tempo: http://localhost:3200
- OTLP: http://localhost:4318 (gRPC :4317)

Configuration is environmental: `OTEL_EXPORTER_OTLP_ENDPOINT`
(default `http://localhost:4318`), `OTEL_SERVICE_NAME` (default `sample`),
and `OTEL_SDK_DISABLED=true` to run without telemetry. A missing collector
only costs telemetry data, never requests.

This crate is `server`-gated: non-server builds only get the spec
constant and (generated) types, while all storage, routing, and
telemetry stay behind the `server` feature so WASM clients never compile
them.

## Dependencies

Most server dependencies (like sqlx and tokio) will not compile on client platforms like WASM. To avoid building server dependencies on the client, you should add platform specific dependencies under the `server` feature in the [Cargo.toml](../Cargo.toml) file. More details about managing server only dependencies can be found in the [Dioxus guide](https://dioxuslabs.com/learn/0.7/guides/fullstack/managing_dependencies#adding-server-only-dependencies).

# API

This crate contains all shared fullstack server functions. This is a great place to place any server-only logic you would like to expose in multiple platforms like a method that accesses your database or a method that sends an email.

## Backend modules (all `server`-gated)

- `state` — `AppState` (a single boxed storage abstraction) plus
  `DATABASE_URL` routing.
- `routes` — the `SampleApi` implementation and shared server functions.
- `store` — backend-agnostic CRUD over the `SampleConnection` trait.
- `models` / `schemas` — Diesel row types and table definitions for SQLite
  and Postgres.
- `generated` — OpenAPI-generated types, client, and axum server
  (regenerate with `cgs regen` from the repo root).
- `docs` — `GET /openapi.json` serving the embedded spec. The interactive
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

This crate will be built twice:

1. Once for the server build with the `dioxus/server` feature enabled
2. Once for the client build with the client feature disabled

During the server build, the server functions will be collected and hosted on a public API for the client to call. During the client build, the server functions will be compiled into the client build.

## Dependencies

Most server dependencies (like sqlx and tokio) will not compile on client platforms like WASM. To avoid building server dependencies on the client, you should add platform specific dependencies under the `server` feature in the [Cargo.toml](../Cargo.toml) file. More details about managing server only dependencies can be found in the [Dioxus guide](https://dioxuslabs.com/learn/0.7/guides/fullstack/managing_dependencies#adding-server-only-dependencies).

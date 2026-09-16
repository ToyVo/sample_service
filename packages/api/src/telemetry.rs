//! OpenTelemetry observability: traces, metrics, and logs over OTLP/HTTP.
//!
//! [`init_telemetry`] wires a `tracing` subscriber (human-readable stderr
//! plus OTLP trace/metric/log pipelines) and returns a guard that flushes on
//! drop. [`router_with_telemetry`] wraps the API router with per-request
//! spans and an HTTP duration histogram. Storage spans come from
//! `#[tracing::instrument]` on the [`crate::store`] operations.
//!
//! Configuration is entirely environmental (defaults also declared in the
//! root `secretspec.toml`, which exports them at server startup):
//!
//! - `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://localhost:4318`): the
//!   collector's OTLP/HTTP receiver. `cgs otel-up` starts one locally.
//! - `OTEL_SERVICE_NAME` (default `sample`).
//! - `OTEL_SDK_DISABLED=true`: skip telemetry entirely (returns `None`).
//!
//! Exporters work in the background: a missing collector degrades to
//! retried-and-dropped telemetry, never to failed requests. Only an invalid
//! setup (bad endpoint URL aside, builders accept anything) disables the
//! pipelines with a stderr notice.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::{
    Resource,
    logs::SdkLoggerProvider,
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Metric instruments are created lazily against the global meter provider,
/// which [`init_telemetry`] installs. Before that (notably in tests) they
/// resolve to no-ops.
static REQUEST_DURATION: std::sync::LazyLock<opentelemetry::metrics::Histogram<f64>> =
    std::sync::LazyLock::new(|| {
        global::meter("sample.http")
            .f64_histogram("http.server.request.duration")
            .with_unit("s")
            .with_description("HTTP request duration in seconds.")
            .build()
    });

/// Providers flushed by [`TelemetryGuard::drop`].
pub struct TelemetryGuard {
    tracer: SdkTracerProvider,
    meter: SdkMeterProvider,
    logger: SdkLoggerProvider,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let _ = self.tracer.shutdown();
        let _ = self.meter.shutdown();
        let _ = self.logger.shutdown();
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn resource(service_name: &str) -> Resource {
    Resource::builder()
        .with_service_name(service_name.to_owned())
        .with_attributes([KeyValue::new("service.version", env!("CARGO_PKG_VERSION"))])
        .build()
}

fn build_providers(endpoint: &str, service_name: &str) -> Result<TelemetryGuard, String> {
    // The HTTP exporter posts to exactly the configured URL, so each signal
    // gets its conventional `/v1/*` path appended here.
    let base = endpoint.trim_end_matches('/');
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(
            SpanExporter::builder()
                .with_http()
                .with_endpoint(format!("{base}/v1/traces"))
                .build()
                .map_err(|err| format!("span exporter: {err}"))?,
        )
        .with_resource(resource(service_name))
        .build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(
            PeriodicReader::builder(
                MetricExporter::builder()
                    .with_http()
                    .with_endpoint(format!("{base}/v1/metrics"))
                    .build()
                    .map_err(|err| format!("metric exporter: {err}"))?,
            )
            // Export every few seconds so local dashboards feel live;
            // production can raise this via code if needed.
            .with_interval(std::time::Duration::from_secs(5))
            .build(),
        )
        .with_resource(resource(service_name))
        .build();
    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(
            LogExporter::builder()
                .with_http()
                .with_endpoint(format!("{base}/v1/logs"))
                .build()
                .map_err(|err| format!("log exporter: {err}"))?,
        )
        .with_resource(resource(service_name))
        .build();
    Ok(TelemetryGuard {
        tracer: tracer_provider,
        meter: meter_provider,
        logger: logger_provider,
    })
}

/// Initialize tracing, metrics, and logs.
///
/// Installs the global propagator/providers and a subscriber combining a
/// stderr formatter with the OTLP trace/metric/log pipelines. Returns `None`
/// (after a stderr notice) when disabled via `OTEL_SDK_DISABLED=true` or when
/// setup fails, so the server always starts with or without a collector.
///
/// Call once in `main` before serving and hold the guard for process lifetime.
#[must_use]
pub fn init_telemetry() -> Option<TelemetryGuard> {
    if std::env::var("OTEL_SDK_DISABLED").is_ok_and(|value| value.eq_ignore_ascii_case("true")) {
        eprintln!("telemetry disabled via OTEL_SDK_DISABLED");
        return None;
    }
    let endpoint = env_or("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318");
    let service_name = env_or("OTEL_SERVICE_NAME", "sample");
    let guard = match build_providers(&endpoint, &service_name) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("telemetry disabled: {err}");
            return None;
        }
    };

    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(guard.tracer.clone());
    global::set_meter_provider(guard.meter.clone());

    let sdk_tracer = global::tracer_provider().tracer("sample-api");
    let trace_layer = tracing_opentelemetry::layer().with_tracer(sdk_tracer);
    let log_bridge =
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&guard.logger);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(trace_layer)
        .with(log_bridge)
        .try_init()
        .is_err()
    {
        eprintln!("telemetry disabled: global subscriber already set");
        return None;
    }

    eprintln!("telemetry exporting to {endpoint} as service `{service_name}`");
    Some(guard)
}

struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.to_str().ok()
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

struct HeaderInjector<'a>(&'a mut axum::http::HeaderMap);

impl opentelemetry::propagation::Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(header)) = (
            axum::http::HeaderName::from_bytes(key.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, header);
        }
    }
}

/// Axum middleware: one `http.server.request` span per request (continuing
/// any incoming `traceparent`), an `http.server.request.duration` histogram
/// point, and `traceparent` injection on the way out. Apply with
/// [`axum::middleware::from_fn`] via [`router_with_telemetry`].
async fn http_trace_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| path.clone(), |matched| matched.as_str().to_owned());

    // The remote parent is attached below via `set_parent` so distributed
    // traces continue across the `traceparent` header.
    let span = tracing::info_span!(
        "http.server.request",
        otel.name = format!("{method} {route}"),
        http.request.method = method.clone(),
        http.route = route.clone(),
        url.path = path,
        http.response.status_code = tracing::field::Empty,
    );
    if span.set_parent(parent).is_err() {
        tracing::warn!("dropping incoming traceparent: span has no subscriber context");
    }
    let _entered = span.enter();
    let start = std::time::Instant::now();
    let mut response = next.run(request).await;
    let status = response.status().as_u16();
    // One event per request: visible on local stderr and shipped as an OTLP
    // log record with this span's trace context attached.
    tracing::info!(
        status = status,
        elapsed_ms = start.elapsed().as_secs_f64() * 1000.0,
        "request completed"
    );
    tracing::Span::current().record("http.response.status_code", i64::from(status));
    REQUEST_DURATION.record(
        start.elapsed().as_secs_f64(),
        &[
            KeyValue::new("http.request.method", method),
            KeyValue::new("http.route", route),
            KeyValue::new("http.response.status_code", i64::from(status)),
        ],
    );
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &tracing::Span::current().context(),
            &mut HeaderInjector(response.headers_mut()),
        );
    });
    response
}

/// Build the API router with [`http_trace_middleware`] applied after routing
/// (so spans carry the matched route template, not raw paths).
pub fn router_with_telemetry(state: crate::AppState) -> axum::Router {
    crate::sample_api_router(state).route_layer(axum::middleware::from_fn(http_trace_middleware))
}

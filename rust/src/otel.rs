//! OpenTelemetry spans and metrics (feature `otel`, `docs/config.md` section 7.10),
//! after the stable HTTP client semantic conventions. The SDK only calls the API
//! crate; spans and metrics go wherever the caller's providers send them.

use std::time::{Duration, Instant};

use opentelemetry::global::BoxedTracer;
use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry::trace::{SpanKind, TraceContextExt as _, Tracer as _};
use opentelemetry::{Context, KeyValue};

use crate::error::Error;
use crate::middleware::builtins::error_kind;
use crate::middleware::{Next, Request, Response};

/// The client's tracer and instruments.
pub(crate) struct Otel {
    tracer: BoxedTracer,
    request_duration: Histogram<f64>,
    call_duration: Histogram<f64>,
    retries: Counter<u64>,
    exchanges: Counter<u64>,
}

/// The semantic conventions' buckets for `http.client.request.duration`, in seconds.
const BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

impl Otel {
    pub(crate) fn new(tracer: Option<BoxedTracer>, meter: Option<Meter>) -> Self {
        let meter = meter.unwrap_or_else(|| opentelemetry::global::meter("inorbithr"));
        Self {
            tracer: tracer.unwrap_or_else(|| opentelemetry::global::tracer("inorbithr")),
            request_duration: meter
                .f64_histogram("http.client.request.duration")
                .with_unit("s")
                .with_boundaries(BUCKETS.to_vec())
                .build(),
            call_duration: meter
                .f64_histogram("inorbit.client.call.duration")
                .with_unit("s")
                .build(),
            retries: meter
                .u64_counter("inorbit.client.retries")
                .with_unit("{retry}")
                .build(),
            exchanges: meter
                .u64_counter("inorbit.client.token.exchanges")
                .with_unit("{exchange}")
                .build(),
        }
    }

    /// `call_tracing`: one `INTERNAL` span per call, named for the operation.
    pub(crate) async fn call(&self, req: Request, next: Next<'_>) -> Result<Response, Error> {
        let parent = Context::current();
        let mut attributes = vec![KeyValue::new("inorbit.operation", req.info.operation)];
        if let Some(id) = &req.info.request_id {
            attributes.push(KeyValue::new("inorbit.request_id", id.clone()));
        }
        let span = self
            .tracer
            .span_builder(req.info.operation)
            .with_kind(SpanKind::Internal)
            .with_attributes(attributes)
            .start_with_context(&self.tracer, &parent);
        let cx = parent.with_span(span);
        *req.info
            .state
            .otel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cx.clone());
        let result = next.run(req).await;
        let span = cx.span();
        match &result {
            Err(e) => span.set_attribute(KeyValue::new("error.type", error_kind(e))),
            Ok(r) if r.status >= 400 => {
                span.set_attribute(KeyValue::new("error.type", r.status.to_string()));
            }
            Ok(_) => {}
        }
        span.end();
        result
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the conventions' attributes, one by one"
    )]
    /// `attempt_tracing`: one `CLIENT` span per attempt, a child of the call's, whose
    /// context goes out as `traceparent`; and the attempt's duration.
    pub(crate) async fn attempt(
        &self,
        mut req: Request,
        next: Next<'_>,
        tracing: bool,
        metrics: bool,
    ) -> Result<Response, Error> {
        let method = req.method.to_string();
        let host = req.url.host_str().unwrap_or_default().to_owned();
        let port = i64::from(req.url.port_or_known_default().unwrap_or(443));
        let cx = if tracing {
            let parent = req
                .info
                .state
                .otel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
                .unwrap_or_else(Context::current);
            let name = match req.info.template {
                Some(t) => format!("{method} {t}"),
                None => method.clone(),
            };
            let mut attributes = vec![
                KeyValue::new("http.request.method", method.clone()),
                KeyValue::new("server.address", host.clone()),
                KeyValue::new("server.port", port),
                KeyValue::new("url.full", redacted_url(&req.url)),
            ];
            if let Some(t) = req.info.template {
                attributes.push(KeyValue::new("url.template", t));
            }
            if req.info.attempt > 1 {
                attributes.push(KeyValue::new(
                    "http.request.resend_count",
                    i64::from(req.info.attempt - 1),
                ));
            }
            let span = self
                .tracer
                .span_builder(name)
                .with_kind(SpanKind::Client)
                .with_attributes(attributes)
                .start_with_context(&self.tracer, &parent);
            let cx = parent.with_span(span);
            let sc = cx.span().span_context().clone();
            if sc.is_valid() {
                req.headers.insert(
                    "traceparent",
                    format!(
                        "00-{:032x}-{:016x}-{:02x}",
                        sc.trace_id(),
                        sc.span_id(),
                        sc.trace_flags().to_u8()
                    ),
                );
                let state = sc.trace_state().header();
                if !state.is_empty() {
                    req.headers.insert("tracestate", state);
                }
            }
            Some(cx)
        } else {
            None
        };
        let started = Instant::now();
        let result = next.run(req).await;
        let error_type = match &result {
            Err(e) => Some(error_kind(e).to_owned()),
            Ok(r) if r.status >= 400 => Some(r.status.to_string()),
            Ok(_) => None,
        };
        if let Some(cx) = cx {
            let span = cx.span();
            if let Ok(r) = &result {
                span.set_attribute(KeyValue::new(
                    "http.response.status_code",
                    i64::from(r.status),
                ));
                if let Some(id) = r.headers.get("x-request-id") {
                    span.set_attribute(KeyValue::new("inorbit.server_request_id", id.to_owned()));
                }
                if let Some(v) = r.headers.get("idempotency-replayed") {
                    span.set_attribute(KeyValue::new(
                        "inorbit.idempotency_replayed",
                        v.eq_ignore_ascii_case("true"),
                    ));
                }
            }
            if let Some(t) = &error_type {
                span.set_attribute(KeyValue::new("error.type", t.clone()));
            }
            span.end();
        }
        if metrics {
            let mut attributes = vec![
                KeyValue::new("http.request.method", method),
                KeyValue::new("server.address", host),
                KeyValue::new("server.port", port),
            ];
            if let Ok(r) = &result {
                attributes.push(KeyValue::new(
                    "http.response.status_code",
                    i64::from(r.status),
                ));
            }
            if let Some(t) = error_type {
                attributes.push(KeyValue::new("error.type", t));
            }
            self.request_duration
                .record(started.elapsed().as_secs_f64(), &attributes);
        }
        result
    }

    /// One token exchange (or `iohr auth token` run) by the credential `source`.
    pub(crate) fn exchanged(&self, source: &str, error: Option<&str>) {
        let mut attributes = vec![KeyValue::new(
            "inorbit.credential.source",
            source.to_owned(),
        )];
        if let Some(e) = error {
            attributes.push(KeyValue::new("error.type", e.to_owned()));
        }
        self.exchanges.add(1, &attributes);
    }

    pub(crate) fn retried(&self, operation: &'static str, reason: &str) {
        self.retries.add(
            1,
            &[
                KeyValue::new("inorbit.operation", operation),
                KeyValue::new("inorbit.retry.reason", reason.to_owned()),
            ],
        );
    }

    pub(crate) fn call_done(
        &self,
        operation: &'static str,
        elapsed: Duration,
        error: Option<&Error>,
    ) {
        let mut attributes = vec![KeyValue::new("inorbit.operation", operation)];
        if let Some(e) = error {
            attributes.push(KeyValue::new("error.type", error_kind(e)));
        }
        self.call_duration
            .record(elapsed.as_secs_f64(), &attributes);
    }
}

/// The URL with every query value replaced by `REDACTED`.
fn redacted_url(url: &url::Url) -> String {
    let mut u = url.clone();
    let pairs: Vec<String> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
    if pairs.is_empty() {
        u.set_query(None);
    } else {
        let mut q = u.query_pairs_mut();
        q.clear();
        for k in pairs {
            q.append_pair(&k, "REDACTED");
        }
    }
    u.to_string()
}

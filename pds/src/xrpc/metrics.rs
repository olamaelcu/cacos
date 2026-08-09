//! XRPC per-request metrics + `/metrics` route mount.
//!
//! `RequestMetrics` is a poem [`Middleware`] that records request count,
//! error count, and duration histograms keyed by the request's NSID (the
//! full URL path under `/xrpc/`). The `/metrics` route itself is owned by
//! the observability plan ([`crate::observability::http::metrics_route`])
//! and re-exported here for route assembly.

use poem::middleware::Middleware;
use poem::{Endpoint, Request, Response, Result};
use std::time::Instant;

/// Metric names from Plan 02 use the `cacos_` prefix.
pub const REQUESTS_TOTAL: &str = "cacos_http_requests_total";
pub const REQUEST_ERRORS_TOTAL: &str = "cacos_http_request_errors_total";
pub const REQUEST_DURATION_SECONDS: &str = "cacos_http_request_duration_seconds";

/// Middleware that records per-request counters and histograms.
pub struct RequestMetrics;

impl<E: Endpoint> Middleware<E> for RequestMetrics {
    type Output = RequestMetricsEndpoint<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestMetricsEndpoint { ep }
    }
}

pub struct RequestMetricsEndpoint<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for RequestMetricsEndpoint<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let start = Instant::now();
        let nsid = req.uri().path().to_string();
        let resp = self.ep.get_response(req).await;
        let is_error = !resp.status().is_success();
        if is_error {
            metrics::counter!(REQUEST_ERRORS_TOTAL, "nsid" => nsid.clone()).increment(1);
        }
        metrics::histogram!(REQUEST_DURATION_SECONDS, "nsid" => nsid.clone())
            .record(start.elapsed().as_secs_f64());
        metrics::counter!(REQUESTS_TOTAL, "nsid" => nsid).increment(1);
        Ok(resp)
    }
}

/// Mount the `/metrics` route on a fresh poem [`poem::Route`]. Owned by
/// Plan 02; this module only contributes the request-middleware metrics.
pub fn metrics_routes() -> poem::Route {
    crate::observability::http::metrics_route()
}

//! HTTP surface for observability: the Prometheus `GET /metrics` route.

use poem::Response;

use crate::observability::metrics;

/// Serve the current metrics in Prometheus text exposition format.
#[poem::handler]
pub async fn metrics_handler() -> Response {
    Response::builder()
        .content_type("text/plain; version=0.0.4; charset=utf-8")
        .body(metrics::render())
}

/// Mount the `/metrics` route on a fresh poem [`poem::Route`].
pub fn metrics_route() -> poem::Route {
    poem::Route::new().at("/metrics", poem::get(metrics_handler))
}

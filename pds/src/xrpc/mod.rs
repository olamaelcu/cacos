//! XRPC route assembly.
//!
//! Placeholder for Plan 08: real `com.atproto.*` handlers, `well_known`, and
//! `health` land there. For now this mounts only the observability `/metrics`
//! route so the server skeleton is exercisable end to end.

pub fn build_app() -> poem::Route {
    poem::Route::new().at(
        "/metrics",
        poem::get(crate::observability::http::metrics_handler),
    )
}

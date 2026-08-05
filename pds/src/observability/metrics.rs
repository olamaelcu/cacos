//! Prometheus metrics recorder and `cacos_` metric registry.
//!
//! Pattern grounded in `vendor/rsky/rsky-relay/src/metrics.rs`:
//! `describe_*!` registers names, `PrometheusBuilder::build_recorder()` builds the
//! recorder, `set_global_recorder` installs it (its `Err` is dropped so repeated
//! calls are tolerated).

use std::sync::RwLock;

use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

// -- metric names ------------------------------------------------------------

pub const HTTP_REQUESTS_TOTAL: &str = "cacos_http_requests_total";
pub const HTTP_REQUEST_ERRORS_TOTAL: &str = "cacos_http_request_errors_total";
pub const SIGNUPS_TOTAL: &str = "cacos_signups_total";
pub const SESSIONS_TOTAL: &str = "cacos_sessions_total";
pub const INVITE_USAGE_TOTAL: &str = "cacos_invite_usage_total";
pub const SEQ_EVENTS_TOTAL: &str = "cacos_seq_events_total";
pub const LAST_SEQ: &str = "cacos_last_seq";
pub const OUTBOX_BUFFER_LAG: &str = "cacos_outbox_buffer_lag";
pub const TIMING_P50_SECONDS: &str = "cacos_timing_p50_seconds";
pub const TIMING_P90_SECONDS: &str = "cacos_timing_p90_seconds";
pub const TIMING_P99_SECONDS: &str = "cacos_timing_p99_seconds";
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "cacos_http_request_duration_seconds";
pub const SEQUENCER_POLL_INTERVAL_SECONDS: &str = "cacos_sequencer_poll_interval_seconds";
pub const BLOB_PUT_BYTES: &str = "cacos_blob_put_bytes";
pub const BLOB_GET_BYTES: &str = "cacos_blob_get_bytes";
pub const ACTOR_CACHE_HITS_TOTAL: &str = "cacos_actor_cache_hits_total";
pub const ACTOR_CACHE_MISSES_TOTAL: &str = "cacos_actor_cache_misses_total";
pub const COMMITS_TOTAL: &str = "cacos_commits_total";

/// Register a description (HELP line) for every cacos metric. Idempotent.
pub fn describe() {
    describe_counter!(
        HTTP_REQUESTS_TOTAL,
        Unit::Count,
        "Total HTTP requests handled"
    );
    describe_counter!(
        HTTP_REQUEST_ERRORS_TOTAL,
        Unit::Count,
        "HTTP requests that errored"
    );
    describe_counter!(SIGNUPS_TOTAL, Unit::Count, "New account signups");
    describe_counter!(SESSIONS_TOTAL, Unit::Count, "Auth sessions created");
    describe_counter!(INVITE_USAGE_TOTAL, Unit::Count, "Invite codes redeemed");
    describe_counter!(
        SEQ_EVENTS_TOTAL,
        Unit::Count,
        "Events sequenced by the sequencer"
    );
    describe_gauge!(LAST_SEQ, Unit::Count, "Last sequence number written");
    describe_gauge!(
        OUTBOX_BUFFER_LAG,
        Unit::Count,
        "SubscribeRepos outbox buffer lag"
    );
    describe_gauge!(
        TIMING_P50_SECONDS,
        Unit::Seconds,
        "p50 inter-event timing (seconds)"
    );
    describe_gauge!(
        TIMING_P90_SECONDS,
        Unit::Seconds,
        "p90 inter-event timing (seconds)"
    );
    describe_gauge!(
        TIMING_P99_SECONDS,
        Unit::Seconds,
        "p99 inter-event timing (seconds)"
    );
    describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        Unit::Seconds,
        "HTTP handler duration"
    );
    describe_histogram!(
        SEQUENCER_POLL_INTERVAL_SECONDS,
        Unit::Seconds,
        "Sequencer poll interval"
    );
    describe_histogram!(BLOB_PUT_BYTES, Unit::Bytes, "Blob upload size");
    describe_histogram!(BLOB_GET_BYTES, Unit::Bytes, "Blob download size");
    describe_counter!(ACTOR_CACHE_HITS_TOTAL, Unit::Count, "BlockMap cache hits");
    describe_counter!(
        ACTOR_CACHE_MISSES_TOTAL,
        Unit::Count,
        "BlockMap cache misses"
    );
    describe_counter!(COMMITS_TOTAL, Unit::Count, "Repo commits applied");
}

/// Handle kept in sync with the global recorder so `/metrics` can render it.
static METRICS_HANDLE: RwLock<Option<PrometheusHandle>> = RwLock::new(None);

/// Build the Prometheus recorder, install it globally, and keep a render handle.
/// Safe to call more than once (the previous global recorder is replaced).
pub fn init_metrics() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    drop(metrics::set_global_recorder(recorder));
    *METRICS_HANDLE
        .write()
        .expect("metrics handle lock poisoned") = Some(handle);
    describe();
}

/// Render the current metrics in Prometheus text format (`PrometheusHandle::render`).
/// Empty until a recorder is installed and at least one metric has been recorded.
/// `render()` internally drains histogram samples, so no separate upkeep is needed.
pub fn render() -> String {
    match METRICS_HANDLE
        .read()
        .expect("metrics handle lock poisoned")
        .as_ref()
    {
        Some(handle) => handle.render(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use metrics::{counter, with_local_recorder};
    use metrics_exporter_prometheus::PrometheusBuilder;

    use super::*;

    #[test]
    fn counter_increments_are_rendered() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        with_local_recorder(&recorder, || {
            describe();
            counter!(HTTP_REQUESTS_TOTAL).increment(1);
            counter!(HTTP_REQUESTS_TOTAL).increment(1);
        });
        let out = handle.render();
        assert!(out.contains(HTTP_REQUESTS_TOTAL), "missing metric: {out}");
        assert!(
            out.contains("cacos_http_requests_total 2"),
            "expected count 2 in: {out}"
        );
    }

    #[test]
    fn describe_is_idempotent() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        with_local_recorder(&recorder, || {
            describe();
            describe();
        });
        let _out = handle.render(); // must not panic
    }
}

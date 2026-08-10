//! Folds `tracing-timing` histogram percentiles into `metrics` gauges.
//!
//! The reporter holds the [`LayerDowncaster`] produced by `TimingLayer::downcaster`
//! and a captured `tracing::Dispatch` (captured at start so the downcast works on
//! whichever tokio worker thread the loop lands on). Each tick it
//! `force_synchronize()`s the layer and reads p50/p90/p99 from every
//! (span-group, event-group) histogram, setting `cacos_timing_p*_seconds` gauges
//! and emitting a `tracing::info!` snapshot log.

use std::time::{Duration, Instant};

use tracing::Instrument;
use tracing_timing::LayerDowncaster;
use tracing_timing::group::{ByMessage, ByName};

use crate::observability::metrics::{
    TIMING_MAX_SECONDS, TIMING_P50_SECONDS, TIMING_P90_SECONDS, TIMING_P99_SECONDS,
    TIMING_P999_SECONDS, TIMING_STAGE_SECONDS,
};

/// Async helper that records `cacos_timing_seconds{stage="..."}` for the wrapped
/// future and emits two `info!` events inside a `cacos.stage` span so the
/// [`TimingReporter`] also folds the stage into the p50/p90/p99/p999/max gauges.
///
/// The wrapped future's output (including its `Result::Err` value) is returned
/// unchanged. The helper never converts, swallows, or logs errors itself.
pub async fn timed<T>(stage: &'static str, fut: impl std::future::Future<Output = T>) -> T {
    let span = tracing::info_span!("cacos.stage", stage = %stage);
    let start = Instant::now();
    span.in_scope(|| tracing::info!("start"));
    let result = fut.instrument(span.clone()).await;
    let elapsed = start.elapsed().as_secs_f64();
    span.in_scope(|| tracing::info!("end"));
    metrics::histogram!(TIMING_STAGE_SECONDS, "stage" => stage).record(elapsed);
    result
}

/// Background task that periodically snapshots tracing-timing histograms.
pub struct TimingReporter {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl TimingReporter {
    /// Spawn the reporter loop with the given cadence.
    ///
    /// Captures the current [`tracing::Dispatch`] (thread-local or global) at start
    /// so the `LayerDowncaster` can find the `TimingLayer` from any thread.
    pub fn start(interval: Duration, downcaster: LayerDowncaster<ByName, ByMessage>) -> Self {
        let dispatch = tracing::dispatcher::get_default(|d| d.clone());
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = ticker.tick() => report_timings(&downcaster, &dispatch),
                    _ = shutdown_rx.changed() => break,
                }
            }
        });

        Self { shutdown_tx, task }
    }

    /// Signal shutdown and await the loop.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.task.await;
    }
}

/// One snapshot cycle: synchronize the timing layer, then fold percentiles into
/// gauges and logs.
fn report_timings(downcaster: &LayerDowncaster<ByName, ByMessage>, dispatch: &tracing::Dispatch) {
    let Some(timing_layer) = downcaster.downcast(dispatch) else {
        tracing::warn!("timing layer not present in dispatch; skipping snapshot");
        return;
    };
    timing_layer.force_synchronize();
    timing_layer.with_histograms(|histograms| {
        for (span_group, events) in histograms {
            for (event_group, histogram) in events {
                histogram.refresh();
                let count = histogram.len();
                if count == 0 {
                    continue;
                }
                // tracing-timing values are nanoseconds; gauges are seconds.
                let ns_to_seconds = |ns: u64| ns as f64 / 1_000_000_000.0;
                let p50 = ns_to_seconds(histogram.value_at_quantile(0.50));
                let p90 = ns_to_seconds(histogram.value_at_quantile(0.90));
                let p99 = ns_to_seconds(histogram.value_at_quantile(0.99));

                metrics::gauge!(
                    TIMING_P50_SECONDS,
                    "span" => span_group.to_string(),
                    "event" => event_group.to_string()
                )
                .set(p50);
                metrics::gauge!(
                    TIMING_P90_SECONDS,
                    "span" => span_group.to_string(),
                    "event" => event_group.to_string()
                )
                .set(p90);
                metrics::gauge!(
                    TIMING_P99_SECONDS,
                    "span" => span_group.to_string(),
                    "event" => event_group.to_string()
                )
                .set(p99);
                let p999 = ns_to_seconds(histogram.value_at_quantile(0.999));
                let max = ns_to_seconds(histogram.max());
                metrics::gauge!(
                    TIMING_P999_SECONDS,
                    "span" => span_group.to_string(),
                    "event" => event_group.to_string()
                )
                .set(p999);
                metrics::gauge!(
                    TIMING_MAX_SECONDS,
                    "span" => span_group.to_string(),
                    "event" => event_group.to_string()
                )
                .set(max);

                tracing::info!(
                    span = %span_group,
                    event = %event_group,
                    p50_ms = p50 * 1000.0,
                    p90_ms = p90 * 1000.0,
                    p99_ms = p99 * 1000.0,
                    p999_ms = p999 * 1000.0,
                    max_ms = max * 1000.0,
                    count,
                    "timing histogram snapshot"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use metrics_exporter_prometheus::PrometheusBuilder;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::registry::Registry;
    use tracing_timing::{Builder, Histogram};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn reporter_folds_inter_event_timing_into_gauges() {
        // Use the global metrics recorder (idempotent init). The reporter
        // thread the timing-layer fold into the global gauges; the test
        // only asserts the p50/p999/max metrics are now present.
        crate::observability::metrics::init_metrics();

        // Subscriber with the timing layer, installed as the thread-local default
        // so events on THIS thread feed the inter-event timing histograms.
        let timing_layer = Builder::default()
            .layer(|| Histogram::new_with_max(1_000_000_000, 2).expect("valid histogram config"));
        let downcaster = timing_layer.downcaster();
        let subscriber = Registry::default()
            .with(metrics_tracing_context::MetricsLayer::new())
            .with(timing_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        // Two events inside one span produce one inter-event timing sample (~5 ms).
        let span = tracing::info_span!("test_span");
        let _entered = span.enter();
        tracing::info!("event_a");
        tokio::time::sleep(Duration::from_millis(5)).await;
        tracing::info!("event_b");
        drop(_entered);

        let reporter = TimingReporter::start(Duration::from_millis(20), downcaster);
        // Generous sleep: several reporter cycles at 20 ms.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let out = crate::observability::metrics::render();
        assert!(
            out.contains(crate::observability::metrics::TIMING_P50_SECONDS),
            "expected p50 gauge in: {out}"
        );
        assert!(
            out.contains(crate::observability::metrics::TIMING_P999_SECONDS),
            "expected p999 gauge in: {out}"
        );
        assert!(
            out.contains(crate::observability::metrics::TIMING_MAX_SECONDS),
            "expected max gauge in: {out}"
        );

        reporter.shutdown().await;
    }

    fn run_with_local_recorder<F, R>(
        recorder: &metrics_exporter_prometheus::PrometheusRecorder,
        f: F,
    ) -> R
    where
        F: std::future::Future<Output = R>,
    {
        // `with_local_recorder` is a synchronous closure but the body is async.
        // Drive the future with `futures::executor::block_on`, which manages its
        // own single-thread executor and is safe to call from inside a
        // `tokio::test` (it does not nest into the surrounding runtime).
        metrics::with_local_recorder(recorder, || futures::executor::block_on(f))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_records_stage_histogram() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        crate::observability::metrics::describe();

        let value: i32 =
            run_with_local_recorder(&recorder, async { timed("test_stage", async { 42 }).await });

        assert_eq!(value, 42);
        let out = handle.render();
        assert!(
            out.contains("cacos_timing_seconds"),
            "expected stage histogram in: {out}"
        );
        assert!(
            out.contains("stage=\"test_stage\""),
            "expected stage label in: {out}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timed_preserves_error_result() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        crate::observability::metrics::describe();

        #[derive(Debug, PartialEq, Eq)]
        struct Oops;
        let value: Result<i32, Oops> = run_with_local_recorder(&recorder, async {
            timed("err_stage", async { Err::<i32, Oops>(Oops) }).await
        });

        assert_eq!(value, Err(Oops));
        let out = handle.render();
        assert!(
            out.contains("stage=\"err_stage\""),
            "expected err_stage sample even on error: {out}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timed_emits_one_inter_event_sample_in_cacos_stage_span() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Set up a timing layer + dispatcher so the timing-layer reporter can
        // extract a sample from the cacos.stage span. Subscriber is thread-local
        // (set_default), so concurrent tests do not interfere.
        let recorder = PrometheusBuilder::new().build_recorder();
        let _handle = recorder.handle();

        let timing_layer = Builder::default()
            .layer(|| Histogram::new_with_max(1_000_000_000, 2).expect("valid histogram config"));
        let downcaster = timing_layer.downcaster();
        let subscriber = Registry::default()
            .with(metrics_tracing_context::MetricsLayer::new())
            .with(timing_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let counter = Arc::new(AtomicUsize::new(0));
        run_with_local_recorder(&recorder, async {
            crate::observability::metrics::describe();
            timed("cacos.stage", async {
                // Small inner work to make the inter-event interval non-zero.
                tokio::time::sleep(Duration::from_millis(5)).await;
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        });

        let dispatch = tracing::dispatcher::get_default(|d| d.clone());
        let Some(layer) = downcaster.downcast(&dispatch) else {
            panic!("timing layer missing from dispatch");
        };
        layer.force_synchronize();
        let total_samples: u64 = layer.with_histograms(|histograms| {
            histograms
                .values()
                .flat_map(|events| events.iter().map(|(_, hist)| hist.len()))
                .sum()
        });
        assert!(
            total_samples >= 1,
            "expected at least one inter-event sample from timed(), got {total_samples}"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "inner future should run exactly once"
        );
    }
}

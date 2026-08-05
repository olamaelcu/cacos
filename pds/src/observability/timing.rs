//! Folds `tracing-timing` histogram percentiles into `metrics` gauges.
//!
//! The reporter holds the [`LayerDowncaster`] produced by `TimingLayer::downcaster`
//! and a captured `tracing::Dispatch` (captured at start so the downcast works on
//! whichever tokio worker thread the loop lands on). Each tick it
//! `force_synchronize()`s the layer and reads p50/p90/p99 from every
//! (span-group, event-group) histogram, setting `cacos_timing_p*_seconds` gauges
//! and emitting a `tracing::info!` snapshot log.

use std::time::Duration;

use tracing_timing::group::{ByMessage, ByName};
use tracing_timing::{Builder, Histogram, LayerDowncaster};

use crate::observability::metrics::{TIMING_P50_SECONDS, TIMING_P90_SECONDS, TIMING_P99_SECONDS};

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
    pub fn start(
        interval: Duration,
        downcaster: LayerDowncaster<ByName, ByMessage>,
    ) -> Self {
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

                tracing::info!(
                    span = %span_group,
                    event = %event_group,
                    p50_ms = p50 * 1000.0,
                    p90_ms = p90 * 1000.0,
                    p99_ms = p99 * 1000.0,
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

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn reporter_folds_inter_event_timing_into_gauges() {
        // Fresh global recorder scoped to this test; no other unit test asserts
        // on the global recorder contents.
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        drop(metrics::set_global_recorder(recorder));
        crate::observability::metrics::describe();

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

        let out = handle.render();
        assert!(
            out.contains(crate::observability::metrics::TIMING_P50_SECONDS),
            "expected p50 gauge in: {out}"
        );

        reporter.shutdown().await;
    }
}

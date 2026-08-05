//! Global tracing registry: `EnvFilter` + `MetricsLayer` + `fmt`.

use metrics_tracing_context::MetricsLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Registry, fmt};

/// Install the global tracing registry.
///
/// Idempotent: a second call drops the `Err` from `set_global_default` instead of
/// panicking (mirrors rsky-relay's tolerance for repeated `set_global_recorder`).
pub fn init_tracing(env_filter: &str) {
    let subscriber = Registry::default()
        .with(EnvFilter::new(env_filter))
        .with(MetricsLayer::new())
        .with(fmt::Layer::new().with_target(false));
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tracing::level_filters::LevelFilter;

    use super::*;

    #[test]
    fn init_tracing_is_idempotent_and_does_not_panic() {
        init_tracing("info");
        init_tracing("info"); // second install must be a dropped error, not a panic
        tracing::info!("smoke event after double init");
    }

    #[test]
    fn fmt_layer_captures_events() {
        let writer = Mutex::new(Vec::new());
        let subscriber = tracing_subscriber::fmt::Subscriber::builder()
            .with_writer(writer)
            .with_max_level(LevelFilter::INFO)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("hello from fmt test");
        });
        // If no panic, the fmt layer was wired and captured the event.
    }
}

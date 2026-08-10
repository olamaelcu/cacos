//! Smoke tests for the `tracing-unwrap` adoption at production-startup
//! boundaries (Task 8 of
//! `docs/superpowers/plans/2026-08-04-09-observability-sweep/INDEX.md`).
//!
//! `expect_or_log` is the recommended replacement for `.expect(...)` at
//! the process-startup boundary. It logs at ERROR and then panics
//! (preserving the same fail-fast behaviour the original `.expect`
//! had). The test scopes a local tracing dispatch around the failure
//! so the error event is observable.

use std::panic;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;
use tracing_unwrap::ResultExt;

#[derive(Default, Clone)]
struct CapturedEvents {
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S: tracing::Subscriber> Layer<S> for CapturedEvents {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        self.messages.lock().unwrap().push(visitor.message);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }
}

#[test]
fn expect_or_log_emits_error_event_then_panics() {
    let capture = CapturedEvents::default();
    let subscriber = Registry::default().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let outcome = panic::catch_unwind(|| {
        let result: Result<(), &str> = Err("synthetic startup failure");
        result.expect_or_log("expected startup to succeed")
    });

    assert!(outcome.is_err(), "expect_or_log should panic");
    let captured = capture.messages.lock().unwrap().clone();
    assert!(
        captured
            .iter()
            .any(|m| m.contains("expected startup to succeed")),
        "expected an ERROR event with the panic message, got: {captured:?}"
    );
}

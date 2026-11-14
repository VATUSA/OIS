//! Records every `tracing` event a future emits, so a test can assert on a log line.
//!
//! `#[sqlx::test]` runs a current-thread runtime, and `scope_test_support::send` drives the
//! production router in-task, so a subscriber attached to the future with `WithSubscriber` sees the
//! handler's events and reqlog's per-request line (`reqlog.rs`'s
//! `the_middleware_actually_obeys_the_rule` is the precedent). Work the future hands to a spawned
//! task is not captured.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

/// One event emitted while the future ran.
#[derive(Debug, Clone)]
pub(crate) struct Captured {
    pub(crate) level: tracing::Level,
    pub(crate) target: String,
    /// The formatted message, e.g. ``tmu: no Discord channel mapped for `tmu-ntml`; …``.
    pub(crate) message: String,
    /// Every other field, string values unquoted (`tmi` → the TMI id).
    pub(crate) fields: BTreeMap<String, String>,
}

struct Recorder(Arc<Mutex<Vec<Captured>>>);

#[derive(Default)]
struct Visitor {
    message: String,
    fields: BTreeMap<String, String>,
}

impl tracing::field::Visit for Visitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Recorder {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        let metadata = event.metadata();
        self.0
            .lock()
            .expect("the recorder's mutex is only held for a push")
            .push(Captured {
                level: *metadata.level(),
                target: metadata.target().to_owned(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

/// Runs `fut` and returns its output with every event it emitted.
pub(crate) async fn capturing<F: Future>(fut: F) -> (F::Output, Vec<Captured>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&events)));
    let output = fut.with_subscriber(subscriber).await;
    let captured = events
        .lock()
        .expect("the recorder's mutex is only held for a push")
        .clone();
    (output, captured)
}

/// The `tmu` skipped-post WARNs among `events` (`handlers::tmu::skipped_post` and
/// `skipped_advisory_post`), matched on the message's fixed prefix so a test asserting there are
/// none cannot pass on a reworded call.
pub(crate) fn skip_warnings(events: &[Captured]) -> Vec<&Captured> {
    events
        .iter()
        .filter(|e| e.message.starts_with("tmu: no Discord channel mapped"))
        .collect()
}

/// Positive control: reqlog's per-request INFO line, proving the capture saw the request.
pub(crate) fn saw_the_request(events: &[Captured]) -> bool {
    events
        .iter()
        .any(|e| e.level == tracing::Level::INFO && e.target == "ois_backend::reqlog")
}

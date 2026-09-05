//! A small in-memory registry of background jobs for the admin "Background Tasks" viewer
//! (issue #34): each job reports when it last started/finished, whether it succeeded, a short
//! detail, and its run count; triggerable jobs also carry a `Notify` the admin API pokes to run
//! them on demand. Lives in `AppState`, shared lock-free-ish behind a `Mutex` (writes are tiny).

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tokio::sync::Notify;
use utoipa::ToSchema;

/// A single background job's observable status.
#[derive(Clone, Serialize, ToSchema)]
pub struct JobStatus {
    pub name: String,
    pub description: String,
    /// Nominal run interval in seconds; `None` for continuous/event-driven jobs.
    pub interval_secs: Option<u64>,
    /// Whether the admin viewer may trigger an immediate run.
    pub triggerable: bool,
    /// Epoch-ms of the last run start / finish (0 = never).
    pub last_started_ms: i64,
    pub last_finished_ms: i64,
    /// Outcome of the last completed run (`None` = never finished).
    pub last_ok: Option<bool>,
    pub last_detail: Option<String>,
    pub runs: u64,
    pub running: bool,
}

struct Entry {
    status: JobStatus,
    notify: Arc<Notify>,
}

/// The shared registry. Cheap to clone the `Arc` around it.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<BTreeMap<String, Entry>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a job (idempotent) and return the handle its loop awaits for manual triggers.
    pub fn register(
        &self,
        name: &str,
        description: &str,
        interval_secs: Option<u64>,
        triggerable: bool,
    ) -> Arc<Notify> {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.entry(name.to_string())
            .or_insert_with(|| Entry {
                status: JobStatus {
                    name: name.to_string(),
                    description: description.to_string(),
                    interval_secs,
                    triggerable,
                    last_started_ms: 0,
                    last_finished_ms: 0,
                    last_ok: None,
                    last_detail: None,
                    runs: 0,
                    running: false,
                },
                notify: Arc::new(Notify::new()),
            })
            .notify
            .clone()
    }

    /// Mark a run as started.
    pub fn begin(&self, name: &str) {
        if let Some(e) = self.jobs.lock().unwrap().get_mut(name) {
            e.status.last_started_ms = Utc::now().timestamp_millis();
            e.status.running = true;
        }
    }

    /// Mark a run as finished with an outcome + short detail (an ok summary or an error string).
    pub fn finish(&self, name: &str, ok: bool, detail: impl Into<String>) {
        if let Some(e) = self.jobs.lock().unwrap().get_mut(name) {
            e.status.last_finished_ms = Utc::now().timestamp_millis();
            e.status.last_ok = Some(ok);
            e.status.last_detail = Some(detail.into());
            e.status.runs += 1;
            e.status.running = false;
        }
    }

    /// All job statuses, name-ordered.
    pub fn snapshot(&self) -> Vec<JobStatus> {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .map(|e| e.status.clone())
            .collect()
    }

    /// Request an immediate run of a job. Returns false if it's unknown or not triggerable.
    pub fn trigger(&self, name: &str) -> bool {
        let jobs = self.jobs.lock().unwrap();
        match jobs.get(name) {
            Some(e) if e.status.triggerable => {
                e.notify.notify_one();
                true
            }
            _ => false,
        }
    }
}

/// Run `body` every `interval` — or immediately when the job is triggered from the admin viewer —
/// recording each run's start, outcome, and a short detail in the registry. `body` returns
/// `Ok(detail)` on success or `Err(detail)` on failure; it should clone anything it needs to own
/// (it runs once per tick). This is the standard loop for a triggerable interval job.
pub async fn run_interval<F, Fut>(
    reg: Arc<JobRegistry>,
    name: &'static str,
    description: &'static str,
    interval: Duration,
    mut body: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let notify = reg.register(name, description, Some(interval.as_secs()), true);
    // `interval`'s first tick fires immediately, so the job runs once at startup (as these jobs did
    // before), then every `interval` after — or right away when triggered.
    let mut ticker = tokio::time::interval(interval);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = notify.notified() => { ticker.reset(); }
        }
        reg.begin(name);
        match body().await {
            Ok(detail) => reg.finish(name, true, detail),
            Err(detail) => reg.finish(name, false, detail),
        }
    }
}

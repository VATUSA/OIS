mod ace;
mod thread;
mod tmi;

use std::sync::Arc;
use std::time::Duration;

use ois_client::{OisClient, OutboundJob};
use serde_json::Value;
use serenity::all::Http;

use crate::snapshot::snapshot_and_push;
use ace::{notify_ace_claim, post_ace_request};
use thread::create_event_thread;
use tmi::post_tmi;

/// Poll → perform → ack, forever. One job's failure never stops the loop.
pub(crate) async fn job_loop(api: OisClient, http: Arc<Http>, poll: Duration) {
    loop {
        match api.lease_jobs(10).await {
            Ok(jobs) => {
                for job in jobs {
                    let id = job.id.clone();
                    match perform_job(&api, &http, &job).await {
                        Ok(result) => {
                            if let Err(e) = api.ack_job(&id, true, result, None).await {
                                tracing::error!(error = %e, job = %id, "ack(success) failed");
                            }
                        }
                        Err(reason) => {
                            tracing::warn!(job = %id, reason, "job failed; nacking for retry");
                            if let Err(e) = api.ack_job(&id, false, None, Some(&reason)).await {
                                tracing::error!(error = %e, job = %id, "ack(failure) failed");
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::error!(error = %e, "lease failed"),
        }
        tokio::time::sleep(poll).await;
    }
}

/// Dispatch a single job by type. `Ok(Some(result))` records ids back on the job (e.g. the posted
/// message id); `Err(msg)` nacks it for retry/park.
async fn perform_job(
    api: &OisClient,
    http: &Arc<Http>,
    job: &OutboundJob,
) -> Result<Option<Value>, String> {
    match job.job_type.as_str() {
        "ace_request_post" => post_ace_request(http, &job.payload).await,
        "ace_request_notify" => notify_ace_claim(http, &job.payload).await,
        "tmi_publish" => post_tmi(http, &job.payload).await,
        "event_thread_create" => create_event_thread(http, &job.payload).await,
        // The admin's "Refresh from Discord" button — re-pull + push the guild snapshot.
        "guild_snapshot" => snapshot_and_push(http, api).await.map(|()| None),
        other => Err(format!("unknown job type: {other}")),
    }
}

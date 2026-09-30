mod ace;
mod dm;
mod thread;
mod tmi;

use std::sync::Arc;
use std::time::Duration;

use ois_client::{OisClient, OutboundJob};
use serde_json::Value;
use serenity::all::Http;

use crate::snapshot::snapshot_and_push;
use ace::{notify_ace_claim, post_ace_request};
use dm::send_claim_dm;
use thread::create_event_thread;
use tmi::post_tmi;

/// Poll → perform → ack, forever. One job's failure never stops the loop.
pub(crate) async fn job_loop(api: OisClient, http: Arc<Http>, poll: Duration) {
    loop {
        match api.lease_jobs(10).await {
            Ok(jobs) => {
                for job in jobs {
                    let id = job.id.clone();
                    // The lease this worker holds. Echoed back on the ack so a successor that
                    // re-leased the job after a reap doesn't have its result overwritten by ours
                    // (VATUSA/OIS#472).
                    let attempt = Some(job.attempt_count);
                    match perform_job(&api, &http, &job).await {
                        Ok(result) => {
                            if let Err(e) = api.ack_job(&id, true, result, None, attempt).await {
                                tracing::error!(error = %e, job = %id, "ack(success) failed");
                            }
                        }
                        Err(reason) => {
                            tracing::warn!(job = %id, reason, "job failed; nacking for retry");
                            if let Err(e) =
                                api.ack_job(&id, false, None, Some(&reason), attempt).await
                            {
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

/// What a job type dispatches to.
///
/// Split out of [`perform_job`] so the table can be asserted. `perform_job` needs a live `Http` and an
/// `OisClient`, so it is unreachable from a test — which meant the one-line arm deciding whether a
/// cancellation posts at all had no coverage: dropping `"tmi_cancel"` from it left every test green
/// while cancels parked as `failed` (#436 review). Same reason `build_message` is split out in
/// `jobs::tmi`.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    AceRequestPost,
    AceRequestNotify,
    AceClaimDm,
    /// A published TMI *or* a cancellation — both carry an already-assembled NTML row, and the bot
    /// only decides how it is framed (#436).
    PostTmi,
    EventThreadCreate,
    GuildSnapshot,
    Unknown,
}

fn route(job_type: &str) -> Route {
    match job_type {
        "ace_request_post" => Route::AceRequestPost,
        "ace_request_notify" => Route::AceRequestNotify,
        "ace_claim_dm" | "ace_claim_reminder_24h" | "ace_claim_reminder_6h" => Route::AceClaimDm,
        "tmi_publish" | "tmi_cancel" => Route::PostTmi,
        "event_thread_create" => Route::EventThreadCreate,
        "guild_snapshot" => Route::GuildSnapshot,
        _ => Route::Unknown,
    }
}

/// Dispatch a single job by type. `Ok(Some(result))` records ids back on the job (e.g. the posted
/// message id); `Err(msg)` nacks it for retry/park.
async fn perform_job(
    api: &OisClient,
    http: &Arc<Http>,
    job: &OutboundJob,
) -> Result<Option<Value>, String> {
    match route(job.job_type.as_str()) {
        Route::AceRequestPost => post_ace_request(http, &job.payload).await,
        Route::AceRequestNotify => notify_ace_claim(http, &job.payload).await,
        Route::AceClaimDm => send_claim_dm(http, &job.payload).await,
        Route::PostTmi => post_tmi(http, &job.payload).await,
        Route::EventThreadCreate => create_event_thread(http, &job.payload).await,
        // The admin's "Refresh from Discord" button — re-pull + push the guild snapshot.
        Route::GuildSnapshot => snapshot_and_push(http, api).await.map(|()| None),
        Route::Unknown => Err(format!("unknown job type: {}", job.job_type)),
    }
}

#[cfg(test)]
mod route_tests {
    use super::{Route, route};

    /// AC 3 rests on this arm: a cancellation only posts because `"tmi_cancel"` routes to the same
    /// place as a publish. Dropping it left the whole suite green while every cancel parked as
    /// `failed` after `MAX_ATTEMPTS`, with nothing in the channel to show for it (#436 review).
    #[test]
    fn a_tmi_cancellation_routes_to_the_same_poster_as_a_publish() {
        assert_eq!(route("tmi_cancel"), Route::PostTmi);
        assert_eq!(route("tmi_publish"), Route::PostTmi);
    }

    /// An unrecognised type must be rejected rather than silently doing nothing — that is what nacks
    /// the job so it parks with a reason instead of looking delivered.
    #[test]
    fn an_unknown_job_type_is_not_silently_accepted() {
        assert_eq!(route("tmi_cancelled"), Route::Unknown);
        assert_eq!(route(""), Route::Unknown);
    }

    /// The rest of the table, so a future edit cannot quietly repoint one of these either.
    #[test]
    fn every_other_job_type_keeps_its_route() {
        for (job_type, expected) in [
            ("ace_request_post", Route::AceRequestPost),
            ("ace_request_notify", Route::AceRequestNotify),
            ("ace_claim_dm", Route::AceClaimDm),
            ("ace_claim_reminder_24h", Route::AceClaimDm),
            ("ace_claim_reminder_6h", Route::AceClaimDm),
            ("event_thread_create", Route::EventThreadCreate),
            ("guild_snapshot", Route::GuildSnapshot),
        ] {
            assert_eq!(route(job_type), expected, "{job_type}");
        }
    }
}

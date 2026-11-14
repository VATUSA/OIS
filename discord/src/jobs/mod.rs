mod ace;
mod advisory;
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
use advisory::post_advisory;
use dm::send_claim_dm;
use thread::create_event_thread;
use tmi::post_tmi;

/// The bot's name on the job queue: it leases only jobs enqueued for `discord` (#590), which the
/// backend's `integration::DISCORD_CONSUMER` stamps on everything it enqueues.
const CONSUMER: &str = "discord";

/// Poll → perform → ack, forever. One job's failure never stops the loop.
pub(crate) async fn job_loop(api: OisClient, http: Arc<Http>, poll: Duration) {
    loop {
        match api.lease_jobs(CONSUMER, 10).await {
            Ok(jobs) => {
                for job in jobs {
                    let id = job.id.clone();
                    // The lease this worker holds. Echoed back on the ack so a successor that
                    // re-leased the job after a reap doesn't have its result overwritten by ours
                    // (VATUSA/OIS#472).
                    let attempt = Some(job.attempt_count);
                    match perform_job(&api, &http, &job).await {
                        Ok(result) => {
                            if let Err(e) = api
                                .ack_job(CONSUMER, &id, true, result, None, attempt)
                                .await
                            {
                                tracing::error!(error = %e, job = %id, "ack(success) failed");
                            }
                        }
                        Err(reason) => {
                            tracing::warn!(job = %id, reason, "job failed; nacking for retry");
                            if let Err(e) = api
                                .ack_job(CONSUMER, &id, false, None, Some(&reason), attempt)
                                .await
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
/// Split out of [`perform_job`] so every job type's arm can be asserted without a fake backend and a
/// fake Discord. Before the split, the one-line arm deciding whether a cancellation posts at all had
/// no coverage: dropping `"tmi_cancel"` from it left every test green while cancels parked as
/// `failed` (#436 review). Same reason `build_message` is split out in `jobs::tmi`.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    AceRequestPost,
    AceRequestNotify,
    AceClaimDm,
    /// A published TMI *or* a cancellation — both carry an already-assembled NTML row, and the bot
    /// only decides how it is framed (#436).
    PostTmi,
    /// A published advisory *or* its cancellation correction — both carry an already-rendered
    /// document, and the bot only fences and splits it (VATUSA/OIS#459).
    PostAdvisory,
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
        "adv_publish" | "adv_cancel" => Route::PostAdvisory,
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
        Route::PostAdvisory => post_advisory(http, &job.payload).await,
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

    /// The same arm, for advisories (VATUSA/OIS#459). Exactly the failure #436 recorded: without this
    /// the cancellation correction never posts and every `adv_cancel` parks as `failed`, with a
    /// cancelled advisory still standing uncorrected in the channel.
    #[test]
    fn an_advisory_cancellation_routes_to_the_same_poster_as_a_publish() {
        assert_eq!(route("adv_cancel"), Route::PostAdvisory);
        assert_eq!(route("adv_publish"), Route::PostAdvisory);
    }

    /// An advisory and a TMI must not share a poster: the bot fences a whole multi-line document for
    /// one and a single row for the other, so a mis-route would post the wrong shape entirely.
    #[test]
    fn advisories_and_tmis_do_not_share_a_route() {
        assert_ne!(route("adv_publish"), route("tmi_publish"));
    }

    /// An unrecognised type must be rejected rather than silently doing nothing — that is what nacks
    /// the job so it parks with a reason instead of looking delivered.
    #[test]
    fn an_unknown_job_type_is_not_silently_accepted() {
        assert_eq!(route("tmi_cancelled"), Route::Unknown);
        assert_eq!(route("advisory_publish"), Route::Unknown);
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

#[cfg(test)]
mod loop_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ois_client::OisClient;
    use serenity::all::Http;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;

    use super::job_loop;

    const LEASE: &str = "POST /api/v1/integration/jobs/lease";
    const POLL: Duration = Duration::from_millis(100);
    /// D72's 15 s request timeout, plus the poll, plus one second of slack: a hung call must have
    /// failed and the loop leased again inside this. A literal, so a request timeout raised past
    /// 16 s fails here.
    const NEXT_LEASE_WITHIN: Duration = Duration::from_millis(16_100);

    /// Reads one whole request, its `content-length` body included, and returns its request line,
    /// followed by a space and the body when there is one. The body is drained so that answering a
    /// POST never closes on unread bytes, which would reset the connection under the client.
    async fn read_request(stream: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return String::new(),
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let body_len = head
            .lines()
            .find_map(|l| {
                let (name, value) = l.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        while buf.len() < head_end + body_len {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let line = head.lines().next().unwrap_or_default();
        let body = String::from_utf8_lossy(&buf[head_end..]);
        if body.is_empty() {
            line.to_string()
        } else {
            format!("{line} {body}")
        }
    }

    async fn reply_json(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    /// A fake backend, which also stands in for Discord's REST API when an `Http` is pointed at it.
    /// It reports every request line on the channel. The first lease is answered with `first_lease`
    /// or, when that is `None`, held open with no reply; every ack is held open; every later lease,
    /// and any other request, is answered with an empty JSON array.
    async fn fake_backend(
        first_lease: Option<&'static str>,
    ) -> (String, mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut held = Vec::new();
            let mut leases = 0;
            while let Ok((mut stream, _)) = listener.accept().await {
                let line = read_request(&mut stream).await;
                let is_lease = line.starts_with(LEASE);
                let is_ack = line.contains("/ack");
                // A send fails only once the test has stopped listening.
                let _ = tx.send(line);
                if is_lease {
                    leases += 1;
                    match (leases, first_lease) {
                        (1, None) => held.push(stream),
                        (1, Some(body)) => reply_json(&mut stream, body).await,
                        _ => reply_json(&mut stream, "[]").await,
                    }
                } else if is_ack {
                    held.push(stream);
                } else {
                    reply_json(&mut stream, "[]").await;
                }
            }
        });
        (format!("http://{addr}"), rx)
    }

    /// AC 4(a) (#930): the first lease's connection never answers. The loop must give up on it and
    /// lease again. The clock is real, for the reason in `ois-client`'s timeout tests.
    #[tokio::test]
    async fn job_loop_leases_again_after_a_lease_hangs() {
        let (base, mut requests) = fake_backend(None).await;
        let api = OisClient::new(base, "t").unwrap();
        let worker = tokio::spawn(job_loop(api, Arc::new(Http::new("")), POLL));

        let first = tokio::time::timeout(Duration::from_secs(5), requests.recv()).await;
        assert!(
            first.ok().flatten().is_some_and(|l| l.starts_with(LEASE)),
            "the loop never leased"
        );
        let second = tokio::time::timeout(NEXT_LEASE_WITHIN, requests.recv()).await;
        worker.abort();

        let second = second.expect("no second lease: the loop is stuck on the hung one");
        assert!(second.is_some_and(|l| l.starts_with(LEASE)));
    }

    /// AC 4(b) (#930), the nack arm: the lease hands out one job of an unknown type, which `perform_job` fails
    /// without touching Discord, so the loop nacks it. That ack's connection never answers. The loop
    /// must give up on the ack and lease again.
    #[tokio::test]
    async fn job_loop_leases_again_after_an_ack_hangs() {
        let one_job = r#"[{"id":"j1","job_type":"no_such_type","payload":{},"subject_type":null,"subject_id":null,"attempt_count":1}]"#;
        let (base, mut requests) = fake_backend(Some(one_job)).await;
        let api = OisClient::new(base, "t").unwrap();
        let worker = tokio::spawn(job_loop(api, Arc::new(Http::new("")), POLL));

        let mut seen = Vec::new();
        for _ in 0..2 {
            match tokio::time::timeout(Duration::from_secs(5), requests.recv()).await {
                Ok(Some(line)) => seen.push(line),
                _ => break,
            }
        }
        let next = tokio::time::timeout(NEXT_LEASE_WITHIN, requests.recv()).await;
        worker.abort();

        assert_eq!(seen.len(), 2, "expected a lease then an ack, got {seen:?}");
        assert!(seen[0].starts_with(LEASE), "{seen:?}");
        assert!(
            seen[1].starts_with("POST /api/v1/integration/jobs/j1/ack"),
            "the job was not acked, so the hung ack was never reached: {seen:?}"
        );
        assert!(
            seen[1].contains(r#""success":false"#),
            "the ack was not a nack, so the failure arm was never reached: {seen:?}"
        );
        let next = next.expect("no lease after the hung ack: the loop is stuck on it");
        assert!(next.is_some_and(|l| l.starts_with(LEASE)));
    }

    /// AC 4(b) (#930), the success arm: a `guild_snapshot` job, with Discord's REST API pointed at
    /// the fake (`HttpBuilder::proxy`). The fake lists no guilds and accepts the snapshot push, so the
    /// job succeeds and the loop acks it as a success. That ack's connection never answers. The loop
    /// must give up on the ack and lease again.
    #[tokio::test]
    async fn job_loop_leases_again_after_a_success_ack_hangs() {
        let one_job = r#"[{"id":"j1","job_type":"guild_snapshot","payload":{},"subject_type":null,"subject_id":null,"attempt_count":1}]"#;
        let (base, mut requests) = fake_backend(Some(one_job)).await;
        let api = OisClient::new(base.clone(), "t").unwrap();
        let discord = serenity::http::HttpBuilder::new("")
            .proxy(base)
            .ratelimiter_disabled(true)
            .build();
        let worker = tokio::spawn(job_loop(api, Arc::new(discord), POLL));

        let mut seen = Vec::new();
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(5), requests.recv()).await {
                Ok(Some(line)) => seen.push(line),
                _ => break,
            }
        }
        let next = tokio::time::timeout(NEXT_LEASE_WITHIN, requests.recv()).await;
        worker.abort();

        assert_eq!(
            seen.len(),
            4,
            "expected lease, guilds, snapshot, ack: {seen:?}"
        );
        assert!(seen[0].starts_with(LEASE), "{seen:?}");
        assert!(
            seen[1].starts_with("GET /api/v10/users/@me/guilds"),
            "{seen:?}"
        );
        assert!(
            seen[2].starts_with("POST /api/v1/integration/discord/guilds/snapshot"),
            "{seen:?}"
        );
        assert!(
            seen[3].starts_with("POST /api/v1/integration/jobs/j1/ack"),
            "the job was not acked, so the hung ack was never reached: {seen:?}"
        );
        assert!(
            seen[3].contains(r#""success":true"#),
            "the ack was not a success, so the success arm was never reached: {seen:?}"
        );
        let next = next.expect("no lease after the hung success ack: the loop is stuck on it");
        assert!(next.is_some_and(|l| l.starts_with(LEASE)));
    }
}

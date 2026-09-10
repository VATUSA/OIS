//! VATUSA events sync. Periodically pulls upcoming events from the public VATUSA v3
//! events API into the `events.event` cache — the anchor for per-event planning.
//!
//! The API returns all events sorted by `start_time` ascending, 25 per page, with the
//! total page count in the `x-total-pages` header. Upcoming events therefore sit on the
//! LAST pages, so the sync pages backward from the end and stops once a whole page is
//! older than the keep window.

use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use crate::models::EventBody;
use crate::repos::events as events_repo;

const VATUSA_EVENTS_URL: &str = "https://api.vatusa.net/v3/events";
const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Keep events that ended within this window (so recent/in-progress events stay visible).
const KEEP_PAST_SECS: i64 = 7 * 24 * 3600;
/// Drop cached events that ended more than this long ago.
const PRUNE_PAST_SECS: i64 = 30 * 24 * 3600;

/// One event as returned by the VATUSA v3 events API.
#[derive(Debug, Deserialize)]
struct VatusaEvent {
    id: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    banner_image_url: String,
    #[serde(default)]
    facility: String,
    start_time: i64, // unix seconds
    end_time: i64,
    #[serde(default)]
    review_status: String,
}

impl VatusaEvent {
    /// Convert to the cache row, dropping events with unrepresentable timestamps.
    fn into_body(self) -> Option<EventBody> {
        let start = Utc.timestamp_opt(self.start_time, 0).single()?;
        let end = Utc.timestamp_opt(self.end_time, 0).single()?;
        Some(EventBody {
            id: self.id,
            title: self.title,
            body: self.body,
            banner_image_url: self.banner_image_url,
            facility: self.facility,
            start_time: start,
            end_time: end,
            review_status: self.review_status,
            // List-only status flags — populated by `events_repo::list_all`, not the sync.
            recording: String::new(),
            ace_requested: false,
            facility_support: false,
        })
    }
}

/// From one page (ascending by start_time), keep the events still within the window and
/// report whether to continue to the previous page. Once a whole page is older than the
/// cutoff, every earlier page is older too, so paging can stop.
fn filter_page(events: Vec<EventBody>, keep_cutoff: DateTime<Utc>) -> (Vec<EventBody>, bool) {
    let kept: Vec<EventBody> = events
        .into_iter()
        .filter(|e| e.end_time >= keep_cutoff)
        .collect();
    let keep_paging = !kept.is_empty();
    (kept, keep_paging)
}

pub fn spawn_sync(pool: PgPool) {
    tokio::spawn(async move { sync_loop(pool).await });
}

async fn sync_loop(pool: PgPool) {
    let client = match reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "events: failed to build HTTP client");
            return;
        }
    };

    let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
    loop {
        ticker.tick().await;
        match sync_once(&client, &pool).await {
            Ok(n) => tracing::info!(events = n, "VATUSA events synced"),
            Err(e) => tracing::warn!(error = %e, "VATUSA events sync failed; keeping cache"),
        }
    }
}

async fn sync_once(
    client: &reqwest::Client,
    pool: &PgPool,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let now = Utc::now();
    let keep_cutoff = now - chrono::Duration::seconds(KEEP_PAST_SECS);

    // Read the last page number from the pagination headers.
    let first = client
        .get(VATUSA_EVENTS_URL)
        .query(&[("page", "1")])
        .send()
        .await?
        .error_for_status()?;
    let total_pages: i64 = first
        .headers()
        .get("x-total-pages")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // Page backward from the newest page, collecting events still within the window.
    let mut collected: Vec<EventBody> = Vec::new();
    let mut page = total_pages;
    while page >= 1 {
        let raw: Vec<VatusaEvent> = client
            .get(VATUSA_EVENTS_URL)
            .query(&[("page", page.to_string())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let bodies: Vec<EventBody> = raw.into_iter().filter_map(VatusaEvent::into_body).collect();
        let (kept, keep_paging) = filter_page(bodies, keep_cutoff);
        collected.extend(kept);
        if !keep_paging {
            break;
        }
        page -= 1;
    }

    events_repo::upsert_many(pool, &collected).await?;
    events_repo::prune(pool, now - chrono::Duration::seconds(PRUNE_PAST_SECS)).await?;
    Ok(collected.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().unwrap()
    }

    fn ev(id: i64, end: i64) -> EventBody {
        EventBody {
            id,
            title: format!("E{id}"),
            body: String::new(),
            banner_image_url: String::new(),
            facility: String::new(),
            start_time: ts(end - 3600),
            end_time: ts(end),
            review_status: "approved".into(),
            recording: String::new(),
            ace_requested: false,
            facility_support: false,
        }
    }

    #[test]
    fn converts_unix_timestamps() {
        let v = VatusaEvent {
            id: 42,
            title: "Banking on Charlotte".into(),
            body: String::new(),
            banner_image_url: String::new(),
            facility: "ZTL".into(),
            start_time: 1_700_000_000,
            end_time: 1_700_010_000,
            review_status: "approved".into(),
        };
        let b = v.into_body().unwrap();
        assert_eq!(b.id, 42);
        assert_eq!(b.start_time, ts(1_700_000_000));
        assert_eq!(b.end_time, ts(1_700_010_000));
    }

    #[test]
    fn keeps_recent_and_future_events_and_signals_more() {
        let cutoff = ts(1_000_000);
        // one already old, two within/after the window
        let page = vec![ev(1, 999_999), ev(2, 1_000_001), ev(3, 2_000_000)];
        let (kept, more) = filter_page(page, cutoff);
        assert_eq!(kept.iter().map(|e| e.id).collect::<Vec<_>>(), vec![2, 3]);
        assert!(more); // some kept → previous page might still hold recent events
    }

    #[test]
    fn stops_paging_on_a_fully_old_page() {
        let cutoff = ts(1_000_000);
        let page = vec![ev(1, 900_000), ev(2, 950_000)];
        let (kept, more) = filter_page(page, cutoff);
        assert!(kept.is_empty());
        assert!(!more); // whole page older than cutoff → earlier pages are older still
    }
}

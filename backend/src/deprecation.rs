//! `Deprecation` and `Sunset` response headers (#591): how a supported operation is retired after 1.0.
//!
//! Before 1.0 nothing in `/api/v1` is stable and nothing needs this. From 1.0, an operation on the
//! supported surface (`docs/architecture/api-surface.md`) is retired only after at least
//! [`NOTICE_DAYS`] of these headers on its responses, plus an API changelog entry
//! (`AGENTS.md` § Versioning). A handler marks its response by returning [`Deprecated`] as a response
//! part — the dates are constants, so a too-short window is a bug, not a runtime case:
//! `Ok((Deprecated::new(SINCE, SUNSET).expect("at least NOTICE_DAYS of notice"), Json(body)))` —
//! and its `utoipa::path` gets `deprecated` so the OpenAPI document says so too.

use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponseParts, ResponseParts};
use chrono::{DateTime, Duration, Utc};

/// The minimum notice between an operation being marked deprecated and its sunset.
pub const NOTICE_DAYS: i64 = 30;

/// A deprecated response: RFC 9745 `Deprecation` (when it was deprecated) and RFC 8594 `Sunset` (when
/// it stops working).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deprecated {
    since: DateTime<Utc>,
    sunset: DateTime<Utc>,
}

impl Deprecated {
    /// `None` when `sunset` gives callers less than [`NOTICE_DAYS`] of warning, so a too-short window
    /// can't be shipped by accident.
    pub fn new(since: DateTime<Utc>, sunset: DateTime<Utc>) -> Option<Self> {
        (sunset - since >= Duration::days(NOTICE_DAYS)).then_some(Self { since, sunset })
    }
}

impl IntoResponseParts for Deprecated {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        let headers = res.headers_mut();
        // RFC 9745: a structured-field date, `@` + Unix seconds.
        let deprecation = format!("@{}", self.since.timestamp());
        // RFC 8594: an HTTP-date (IMF-fixdate), always GMT.
        let sunset = self.sunset.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        for (name, value) in [("deprecation", deprecation), ("sunset", sunset)] {
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(HeaderName::from_static(name), value);
            }
        }
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, routing::get};
    use chrono::TimeZone;
    use tower::ServiceExt;

    use super::*;

    fn since() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2027, 1, 4, 12, 0, 0).unwrap()
    }

    /// The owner's 30 days, as absolute dates straddling it rather than `since + NOTICE_DAYS`, which
    /// would pass for any value of the constant.
    #[test]
    fn the_notice_window_is_at_least_thirty_days() {
        let thirty = Utc.with_ymd_and_hms(2027, 2, 3, 12, 0, 0).unwrap();
        assert!(
            Deprecated::new(since(), thirty).is_some(),
            "exactly 30 days is enough"
        );
        assert!(
            Deprecated::new(since(), thirty - Duration::seconds(1)).is_none(),
            "a second short of 30 days is not"
        );
        assert!(Deprecated::new(since(), since()).is_none());
    }

    /// The headers on the wire, through a real router: one route marked deprecated, one not.
    #[tokio::test]
    async fn a_deprecated_route_says_so_and_others_do_not() {
        let marked = Deprecated::new(since(), since() + Duration::days(90)).unwrap();
        let app = Router::new()
            .route("/old", get(move || async move { (marked, "still here") }))
            .route("/current", get(|| async { "fine" }));
        let call = |uri: &'static str| {
            let app = app.clone();
            async move {
                app.oneshot(Request::get(uri).body(Body::empty()).unwrap())
                    .await
                    .unwrap()
            }
        };

        let old = call("/old").await;
        assert_eq!(old.headers()["deprecation"], "@1799064000");
        assert_eq!(old.headers()["sunset"], "Sun, 04 Apr 2027 12:00:00 GMT");

        let current = call("/current").await;
        assert!(current.headers().get("deprecation").is_none());
        assert!(current.headers().get("sunset").is_none());
    }
}

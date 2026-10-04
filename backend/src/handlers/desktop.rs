//! Desktop installer downloads (#534).
//!
//! Resolves the current release's installer for a platform and redirects to it, so the browser never
//! talks to GitHub itself.
//!
//! # Why this exists at all
//!
//! `web/src/pages/download.tsx` used to call `api.github.com` **from the browser** and fall back to
//! the generic releases page when that failed. It failed in two ordinary situations: the host is
//! absent from the desktop app's CSP `connect-src` (`desktop/src-tauri/tauri.conf.json`), so inside
//! the app the call is blocked outright; and GitHub's unauthenticated limit is 60 requests/hour per
//! IP, which a NAT'd network or an event-night rush exhausts. Every row then silently pointed at the
//! releases page while still looking like a direct download.
//!
//! Moving the lookup here fixes all three at once — the API origin is already allowlisted, one
//! cached server-side call replaces one call per visitor, and the matching below can be strict
//! because it is no longer squeezed into a `.endsWith` in a React effect.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{extract::Path, response::Redirect};
use serde::Deserialize;

use crate::errors::ApiError;

/// Where the installers live. The repo is public, so Releases is a free CDN that needs no auth.
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/VATUSA/OIS/releases/latest";

/// How long a resolved release is reused.
///
/// Releases are cut by hand and rarely, so this is generously long: the point is that a thousand
/// visitors cost one upstream call, not that the answer is fresh to the second. A new release becomes
/// available within this window of being published, which for a desktop installer is immaterial —
/// and the in-app updater (`tauri.conf.json`'s endpoint) is what keeps installed copies current
/// regardless.
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// GitHub is slow far more often than it is down; this bounds a stuck request rather than letting it
/// hold a connection open.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// The platforms Tauri's bundler produces for, and the asset extensions each one ships.
///
/// Order within a platform is **significant** — the first match wins. Windows lists `-setup.exe`
/// before `.msi` deliberately: both are produced, and the NSIS installer is the one to hand a user.
/// The browser-side matcher this replaces had the same order by accident rather than intent, which
/// is the kind of thing that gets "tidied" into the wrong behaviour.
const PLATFORMS: &[(&str, &[&str])] = &[
    ("macos", &[".dmg"]),
    ("windows", &["-setup.exe", ".msi"]),
    ("linux", &[".AppImage", ".deb"]),
];

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

/// Pick the download URL for `platform` out of a release's assets.
///
/// Split out and pure so the matching rules are testable without the network — they are the part
/// most likely to go quietly wrong, and the part the issue flagged as loose.
///
/// Matching is on a **suffix of the filename**, which is as strict as the bundler's naming allows:
/// every asset carries its version, so an exact name cannot be hardcoded. `OIS_0.2.0_universal.dmg`
/// matches `.dmg`; `notes-about-dmg.txt` does not, because the extension must end the name.
fn asset_for(platform: &str, assets: &[GithubAsset]) -> Option<String> {
    let extensions = PLATFORMS
        .iter()
        .find(|(id, _)| *id == platform)
        .map(|(_, exts)| *exts)?;

    extensions.iter().find_map(|ext| {
        assets
            .iter()
            .find(|a| a.name.ends_with(ext))
            .map(|a| a.browser_download_url.clone())
    })
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(UPSTREAM_TIMEOUT)
            // GitHub rejects API requests without one.
            .user_agent("ois-desktop-download/1.0")
            .build()
            .expect("reqwest client")
    })
}

/// The cached release.
///
/// Two instants, because they answer different questions: `checked` is the last refresh *attempt* and
/// decides whether to ask GitHub again; `fetched` is when these assets actually came from GitHub and is
/// only reported, when a stale entry is served (#562).
///
/// The assets are an `Arc<[_]>` so a cache hit is a pointer clone rather than a copy of every asset —
/// and so the lock is released before the caller touches the data.
struct Cached {
    fetched: Instant,
    checked: Instant,
    assets: Arc<[GithubAsset]>,
}

type AssetCache = Mutex<Option<Cached>>;

fn cache() -> &'static AssetCache {
    static CACHE: OnceLock<AssetCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The current release's assets: from cache when it is fresh, from GitHub otherwise.
async fn assets() -> Result<Arc<[GithubAsset]>, ApiError> {
    assets_from(cache(), CACHE_TTL, fetch_latest()).await
}

async fn fetch_latest() -> Result<Arc<[GithubAsset]>, ApiError> {
    let release: GithubRelease = http()
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| ApiError::ServiceUnavailable)?
        .json()
        .await
        .map_err(|_| ApiError::ServiceUnavailable)?;
    Ok(release.assets.into())
}

/// Serve from `cache`, using `refresh` only when the entry is older than `ttl` — the cache, the TTL and
/// the upstream call are arguments so the policy is testable without GitHub, and without backdating an
/// `Instant` by fifteen minutes (which panics on a CI runner that booted more recently than that).
///
/// **A failed refresh serves the stale entry rather than a 503** (#562). Release asset URLs stay valid
/// for months, so an answer from an hour ago is almost certainly still right; refusing to serve it
/// turned every GitHub outage or rate-limit exhaustion into a download outage. Only an *empty* cache
/// and a failed refresh is a 503. There is no age cap: the cache is process memory, emptied by every
/// deploy or restart, so a stale entry never outlives the process.
///
/// A failed refresh also counts as a check, so GitHub is asked again at most once per `CACHE_TTL`
/// rather than on every visitor's click — hammering it during a rate-limit exhaustion is what keeps
/// the limit exhausted, and each visitor would wait on a call that was going to fail.
///
/// A cache miss while another request is already fetching results in both fetching; that is
/// deliberate. Holding the lock across the `await` would serialise every visitor behind one upstream
/// call, and a rare duplicate request costs far less than that. A poisoned lock is treated as a miss
/// rather than an error: the worst case is one extra upstream call, which is strictly better than
/// failing a download because a previous request panicked.
async fn assets_from(
    cache: &AssetCache,
    ttl: Duration,
    refresh: impl std::future::Future<Output = Result<Arc<[GithubAsset]>, ApiError>>,
) -> Result<Arc<[GithubAsset]>, ApiError> {
    if let Ok(guard) = cache.lock()
        && let Some(entry) = guard.as_ref()
        && entry.checked.elapsed() < ttl
    {
        return Ok(Arc::clone(&entry.assets));
    }

    match refresh.await {
        Ok(assets) => {
            if let Ok(mut guard) = cache.lock() {
                let now = Instant::now();
                *guard = Some(Cached {
                    fetched: now,
                    checked: now,
                    assets: Arc::clone(&assets),
                });
            }
            Ok(assets)
        }
        Err(err) => {
            let Ok(mut guard) = cache.lock() else {
                return Err(err);
            };
            let Some(entry) = guard.as_mut() else {
                return Err(err);
            };
            entry.checked = Instant::now();
            tracing::warn!(
                age_secs = entry.fetched.elapsed().as_secs(),
                "desktop release refresh failed; serving the cached release"
            );
            Ok(Arc::clone(&entry.assets))
        }
    }
}

/// Redirect to the current installer for `platform`.
///
/// Public by design — you download the app before you have any reason to be signed in — which is why
/// it sits under `/api/v1/public/`, beside the other unauthenticated reference endpoints. A GET with
/// no `RequirePermission` anywhere else in `router.rs` would read as an oversight.
///
/// `503` rather than a redirect to the releases page when the lookup fails: a silent substitution is
/// exactly the behaviour this issue was filed about. The page keeps an explicit "all releases" link
/// for the cases this cannot serve. `ServiceUnavailable` is reused rather than adding a `BadGateway`
/// variant for one handler — the download genuinely cannot be served, which is what 503 says.
#[utoipa::path(
    get,
    path = "/api/v1/public/desktop/download/{platform}",
    tag = "desktop",
    params(("platform" = String, Path, description = "macos | windows | linux")),
    responses(
        (status = 307, description = "Redirect to the installer"),
        (status = 400, description = "Unknown platform"),
        (status = 503, description = "The release could not be resolved upstream")
    )
)]
pub async fn download(Path(platform): Path<String>) -> Result<Redirect, ApiError> {
    let platform = platform.to_ascii_lowercase();
    if !PLATFORMS.iter().any(|(id, _)| *id == platform) {
        return Err(ApiError::BadRequest);
    }
    let assets = assets().await?;
    let url = asset_for(&platform, &assets).ok_or(ApiError::ServiceUnavailable)?;
    Ok(Redirect::temporary(&url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    fn assets(names: &[&str]) -> Vec<GithubAsset> {
        names
            .iter()
            .map(|n| GithubAsset {
                name: (*n).to_string(),
                browser_download_url: format!("https://example.test/{n}"),
            })
            .collect()
    }

    // ---- #562: a failed refresh serves the cached release ----

    use std::sync::atomic::{AtomicBool, Ordering};

    /// A cache holding `names`, last checked `checked_ago` ago.
    fn cached(names: &[&str], checked_ago: Duration) -> AssetCache {
        let checked = Instant::now() - checked_ago;
        Mutex::new(Some(Cached {
            fetched: checked,
            checked,
            assets: assets(names).into(),
        }))
    }

    /// The policy is tested with a short TTL rather than `CACHE_TTL`: backdating an `Instant` by
    /// fifteen minutes panics on a machine that booted less than fifteen minutes ago, which a fresh CI
    /// runner can be. Two seconds is safe anywhere.
    const TTL: Duration = Duration::from_secs(1);

    /// Older than `TTL`, so the next request must try GitHub.
    fn stale() -> Duration {
        Duration::from_secs(2)
    }

    fn names(assets: &[GithubAsset]) -> Vec<&str> {
        assets.iter().map(|a| a.name.as_str()).collect()
    }

    /// A refresh that records whether it was polled, so every test can prove which path it took —
    /// a "stale" entry that the policy actually treated as fresh would otherwise pass AC1 vacuously.
    async fn refresh(
        polled: &AtomicBool,
        result: Result<Vec<GithubAsset>, ApiError>,
    ) -> Result<Arc<[GithubAsset]>, ApiError> {
        // An `async fn` body runs only when polled, so this records exactly whether the policy asked.
        polled.store(true, Ordering::SeqCst);
        result.map(Into::into)
    }

    /// AC1. GitHub down, an entry held: the visitor still gets the installer. Before #562 this was a
    /// 503, and every outage or rate-limit exhaustion took the download page down with it.
    #[tokio::test]
    async fn a_failed_refresh_serves_the_cached_release() {
        let cache = cached(&["OIS_0.2.0_universal.dmg"], stale());
        let polled = AtomicBool::new(false);

        let served = assets_from(
            &cache,
            TTL,
            refresh(&polled, Err(ApiError::ServiceUnavailable)),
        )
        .await
        .expect("a held release must be served when GitHub fails");

        assert!(
            polled.load(Ordering::SeqCst),
            "the stale entry must trigger a refresh"
        );
        assert_eq!(names(&served), ["OIS_0.2.0_universal.dmg"]);
    }

    /// AC2. Nothing held and GitHub down is the one case that cannot be served.
    #[tokio::test]
    async fn a_failed_refresh_with_nothing_cached_is_a_503() {
        let cache: AssetCache = Mutex::new(None);
        let polled = AtomicBool::new(false);

        let result = assets_from(
            &cache,
            TTL,
            refresh(&polled, Err(ApiError::ServiceUnavailable)),
        )
        .await;

        assert!(matches!(result, Err(ApiError::ServiceUnavailable)));
        assert!(
            cache.lock().unwrap().is_none(),
            "a failure must not invent an entry"
        );
    }

    /// AC3. A successful refresh replaces the stale entry, and the next request is served from it.
    #[tokio::test]
    async fn a_successful_refresh_replaces_the_entry() {
        let cache = cached(&["OIS_0.1.0_universal.dmg"], stale());
        let polled = AtomicBool::new(false);

        let served = assets_from(
            &cache,
            TTL,
            refresh(&polled, Ok(assets(&["OIS_0.2.0_universal.dmg"]))),
        )
        .await
        .unwrap();

        assert!(polled.load(Ordering::SeqCst));
        assert_eq!(names(&served), ["OIS_0.2.0_universal.dmg"]);
        let held = cache.lock().unwrap();
        assert_eq!(
            names(&held.as_ref().unwrap().assets),
            ["OIS_0.2.0_universal.dmg"]
        );
    }

    /// After a failed refresh, GitHub is not asked again until the TTL passes: the next visitor gets
    /// the cached release straight away rather than waiting on another failing call.
    #[tokio::test]
    async fn a_failed_refresh_is_not_retried_on_the_next_request() {
        let cache = cached(&["OIS_0.2.0_universal.dmg"], stale());
        let first = AtomicBool::new(false);
        assets_from(
            &cache,
            TTL,
            refresh(&first, Err(ApiError::ServiceUnavailable)),
        )
        .await
        .unwrap();
        assert!(first.load(Ordering::SeqCst));

        let second = AtomicBool::new(false);
        let served = assets_from(
            &cache,
            TTL,
            refresh(&second, Err(ApiError::ServiceUnavailable)),
        )
        .await
        .unwrap();

        assert!(
            !second.load(Ordering::SeqCst),
            "GitHub must not be asked again inside the TTL after a failure"
        );
        assert_eq!(names(&served), ["OIS_0.2.0_universal.dmg"]);
    }

    /// A fresh entry is served without touching GitHub at all — the property the cache exists for.
    #[tokio::test]
    async fn a_fresh_entry_never_reaches_github() {
        let cache = cached(&["OIS_0.2.0_universal.dmg"], Duration::ZERO);
        let polled = AtomicBool::new(false);

        assets_from(
            &cache,
            TTL,
            refresh(&polled, Err(ApiError::ServiceUnavailable)),
        )
        .await
        .unwrap();

        assert!(!polled.load(Ordering::SeqCst));
    }

    /// The real v0.2.0 asset set, so the happy path is pinned against names that actually ship.
    fn release() -> Vec<GithubAsset> {
        assets(&[
            "OIS_0.2.0_universal.dmg",
            "OIS_0.2.0_x64-setup.exe",
            "OIS_0.2.0_x64_en-US.msi",
            "OIS_0.2.0_amd64.AppImage",
            "OIS_0.2.0_amd64.deb",
            "latest.json",
        ])
    }

    #[test]
    fn each_platform_gets_its_own_installer() {
        assert_eq!(
            asset_for("macos", &release()).as_deref(),
            Some("https://example.test/OIS_0.2.0_universal.dmg")
        );
        assert_eq!(
            asset_for("linux", &release()).as_deref(),
            Some("https://example.test/OIS_0.2.0_amd64.AppImage")
        );
    }

    /// Windows ships both an NSIS `-setup.exe` and an `.msi`, and the order in `PLATFORMS` decides
    /// which a user gets. Pinned because it is a one-line reorder away from silently changing, and
    /// the matcher this replaced had the order by accident.
    #[test]
    fn windows_prefers_the_nsis_installer_over_the_msi() {
        assert_eq!(
            asset_for("windows", &release()).as_deref(),
            Some("https://example.test/OIS_0.2.0_x64-setup.exe")
        );
    }

    /// Falls through to the next extension rather than giving up, so a release missing one artifact
    /// still serves the platform.
    #[test]
    fn windows_falls_back_to_the_msi_when_no_exe_was_built() {
        let only_msi = assets(&["OIS_0.2.0_x64_en-US.msi"]);
        assert_eq!(
            asset_for("windows", &only_msi).as_deref(),
            Some("https://example.test/OIS_0.2.0_x64_en-US.msi")
        );
    }

    /// The extension has to **end** the filename. A file merely mentioning it is not an installer.
    #[test]
    fn a_lookalike_filename_is_not_matched() {
        let decoys = assets(&["read-me-about-the-dmg.txt", "dmg-notes.md", "OIS.dmg.sig"]);
        assert_eq!(asset_for("macos", &decoys), None);
    }

    #[test]
    fn a_release_with_nothing_for_this_platform_matches_nothing() {
        assert_eq!(asset_for("macos", &assets(&["latest.json"])), None);
    }

    /// The 400 is a **documented response** in the `utoipa::path` block above, and it is the only
    /// input validation this endpoint has. Deleting the allowlist check left the whole suite green:
    /// an unknown platform fell through to `asset_for` returning `None` and yielded 503, so the
    /// published contract said one thing and the route did another.
    ///
    /// Driven through the real router (`scope_test_support::send`, #364) rather than by calling
    /// `download` directly — the point is that the route is reachable, unauthenticated, and rejects
    /// before it reaches the network. This case needs no upstream at all, because the guard returns
    /// ahead of the fetch.
    #[sqlx::test]
    async fn the_route_rejects_an_unknown_platform_before_any_upstream_call(pool: PgPool) {
        let state = crate::scope_test_support::test_state(pool, std::collections::HashMap::new());

        for bogus in ["solaris", "..", "%2e%2e", "macos-x", ""] {
            let status = crate::scope_test_support::send(
                &state,
                http::Method::GET,
                &format!("/api/v1/public/desktop/download/{bogus}"),
                "",
                None,
            )
            .await;
            assert!(
                status == http::StatusCode::BAD_REQUEST || status == http::StatusCode::NOT_FOUND,
                "{bogus:?} must be refused without an upstream call, got {status}"
            );
        }
    }

    /// The defect #534 was filed about: the page silently substituted the generic releases page for a
    /// real installer whenever the lookup failed. Moving the lookup server-side fixed it, and nothing
    /// kept it fixed — reinstating `unwrap_or_else(|| ".../releases")` here left all 795 Rust and 786
    /// web tests green, because the web tests only assert the link's href, which does not change.
    ///
    /// A source scan because the failure is an *absence*: no response assertion can prove the handler
    /// will never invent a fallback. Same instrument as `features/dashboard/nas-template-gate.test.ts`,
    /// and aimed at the one substitution this issue exists to prevent.
    #[test]
    fn the_handler_never_falls_back_to_the_releases_page() {
        let src = include_str!("desktop.rs");
        // The module doc and these tests both discuss the releases page; only the handler must not
        // *reach* for it, so the scan looks for it being produced as a value.
        let offending: Vec<&str> = src
            .lines()
            .filter(|l| {
                let code = l.trim_start();
                !code.starts_with("//") && !code.starts_with("///") && !code.starts_with("//!")
            })
            .filter(|l| l.contains("/releases") && !l.contains("releases/latest"))
            .collect();

        assert!(
            offending.is_empty(),
            "the handler must return 503 rather than substitute the releases page (#534); found: {offending:?}"
        );
    }

    #[test]
    fn an_unknown_platform_matches_nothing() {
        assert_eq!(asset_for("solaris", &release()), None);
        assert_eq!(asset_for("", &release()), None);
    }
}

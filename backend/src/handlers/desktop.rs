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

/// The cached release: when it was fetched, and the assets it carried.
///
/// Held as an `Arc<[_]>` so a cache hit is a pointer clone rather than a copy of every asset — and
/// so the lock is released before the caller touches the data. Aliased because the bare type is what
/// `clippy::type_complexity` objects to, and the alias is the better documentation anyway.
type AssetCache = Mutex<Option<(Instant, Arc<[GithubAsset]>)>>;

fn cache() -> &'static AssetCache {
    static CACHE: OnceLock<AssetCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The current release's assets, from cache when it is fresh.
///
/// A cache miss while another request is already fetching results in both fetching; that is
/// deliberate. Holding the lock across the `await` would serialise every visitor behind one upstream
/// call, and a rare duplicate request costs far less than that.
///
/// A poisoned lock is treated as a miss rather than an error: the worst case is one extra upstream
/// call, which is strictly better than failing a download because a previous request panicked.
async fn assets() -> Result<Arc<[GithubAsset]>, ApiError> {
    if let Ok(guard) = cache().lock() {
        if let Some((fetched_at, assets)) = guard.as_ref() {
            if fetched_at.elapsed() < CACHE_TTL {
                return Ok(Arc::clone(assets));
            }
        }
    }

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

    let assets: Arc<[GithubAsset]> = release.assets.into();
    if let Ok(mut guard) = cache().lock() {
        *guard = Some((Instant::now(), Arc::clone(&assets)));
    }
    Ok(assets)
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

//! Serves the generated OpenAPI document and the interactive API reference built on it.
//!
//! The reference is Scalar's standalone bundle, vendored in `backend/assets/scalar/` and served from
//! this origin. The page runs where try-it-out requests carry the caller's session cookie, so its
//! script comes from here, not a CDN, and its configuration turns off every feature that would
//! call a third party (#1093).

use axum::{
    Json,
    http::header,
    response::{Html, IntoResponse, Redirect},
};
use utoipa::OpenApi;

use crate::openapi::ApiDoc;

/// The vendored `@scalar/api-reference` version. A macro so the asset path, the route and the page
/// are all built from one literal at compile time; `backend/assets/scalar/README` says how to bump it.
macro_rules! scalar_version {
    () => {
        "1.77.0"
    };
}

pub const OPENAPI_PATH: &str = "/docs/api/v1/openapi.json";
pub const REFERENCE_PATH: &str = "/docs/v1";
pub const SCALAR_BUNDLE_PATH: &str = concat!("/docs/v1/scalar-", scalar_version!(), ".js");

const SCALAR_BUNDLE: &[u8] = include_bytes!(concat!(
    "../../assets/scalar/api-reference-",
    scalar_version!(),
    ".js"
));

/// Scalar reads `data-url` and `data-configuration` from the `#api-reference` element, so the page
/// carries no inline script. `withDefaultFonts: false` stops the font download from
/// fonts.scalar.com, `agent.disabled` stops the spec upload to api.scalar.com, and `proxyUrl` is left
/// unset so try-it-out calls go straight to this origin instead of through proxy.scalar.com.
const REFERENCE_PAGE: &str = concat!(
    r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OIS API reference</title>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="/docs/api/v1/openapi.json"
      data-configuration='{"withDefaultFonts":false,"agent":{"disabled":true},"showDeveloperTools":"never","telemetry":false,"persistAuth":false}'
    ></script>
    <script src="/docs/v1/scalar-"#,
    scalar_version!(),
    r#".js"></script>
  </body>
</html>
"#
);

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

pub async fn api_reference() -> Html<&'static str> {
    Html(REFERENCE_PAGE)
}

/// The bundle's path carries its version, so a year's immutable cache is safe: a bump changes the URL.
pub async fn scalar_bundle() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        SCALAR_BUNDLE,
    )
}

/// The old Swagger UI path; links to it land on the reference.
pub async fn swagger_redirect() -> Redirect {
    Redirect::permanent(REFERENCE_PATH)
}

/// Swagger UI served its own copy of the spec here.
pub async fn swagger_openapi_redirect() -> Redirect {
    Redirect::permanent(OPENAPI_PATH)
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        response::Response,
    };
    use tower::ServiceExt;

    use super::SCALAR_BUNDLE;
    use crate::{router::build_router, state::AppState};

    const PAGE: &str = "/docs/v1";
    const BUNDLE: &str = "/docs/v1/scalar-1.77.0.js";

    fn router() -> Router {
        build_router(AppState::without_db())
    }

    async fn get(uri: &str) -> Response {
        router()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn header(response: &Response, name: &str) -> String {
        response
            .headers()
            .get(name)
            .unwrap_or_else(|| panic!("no {name} header"))
            .to_str()
            .unwrap()
            .to_string()
    }

    async fn body_text(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Every `src="…"` and `href="…"` value in `html`.
    fn linked_urls(html: &str) -> Vec<String> {
        let mut urls = Vec::new();
        for attr in ["src=\"", "href=\""] {
            for (at, _) in html.match_indices(attr) {
                let rest = &html[at + attr.len()..];
                urls.push(rest[..rest.find('"').unwrap()].to_string());
            }
        }
        urls
    }

    /// The JSON in the page's `data-configuration='…'` attribute.
    fn scalar_configuration(html: &str) -> serde_json::Value {
        let attr = "data-configuration='";
        let start = html.find(attr).expect("a data-configuration attribute") + attr.len();
        let end = start + html[start..].find('\'').unwrap();
        serde_json::from_str(&html[start..end]).expect("data-configuration is JSON")
    }

    #[tokio::test]
    async fn the_reference_page_reads_the_spec_and_the_local_bundle() {
        let response = get(PAGE).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(header(&response, "content-type").starts_with("text/html"));
        let html = body_text(response).await;

        assert!(html.contains(r#"id="api-reference""#), "{html}");
        assert!(
            html.contains(r#"data-url="/docs/api/v1/openapi.json""#),
            "{html}"
        );
        assert_eq!(linked_urls(&html), vec![BUNDLE.to_string()], "{html}");
    }

    #[tokio::test]
    async fn the_reference_page_turns_off_every_scalar_feature_that_calls_a_third_party() {
        let config = scalar_configuration(&body_text(get(PAGE).await).await);

        assert_eq!(
            config["withDefaultFonts"], false,
            "fonts.scalar.com: {config}"
        );
        assert_eq!(
            config["agent"]["disabled"], true,
            "api.scalar.com: {config}"
        );
        assert_eq!(config["showDeveloperTools"], "never", "{config}");
        assert_eq!(config["telemetry"], false, "{config}");
        assert_eq!(
            config["persistAuth"], false,
            "a key would sit in localStorage: {config}"
        );
        assert!(
            config.get("proxyUrl").is_none(),
            "try-it-out must not go through a proxy: {config}"
        );
    }

    #[tokio::test]
    async fn the_reference_page_links_nothing_on_another_origin() {
        let html = body_text(get(PAGE).await).await;
        let urls = linked_urls(&html);

        assert!(!urls.is_empty(), "the scan found no links at all: {html}");
        for url in &urls {
            assert!(
                url.starts_with('/') && !url.starts_with("//"),
                "off-origin link {url}"
            );
        }
        // The scan itself catches an absolute and a protocol-relative link.
        let planted =
            r#"<script src="https://cdn.example/x.js"></script><link href="//cdn.example/y.css">"#;
        assert_eq!(
            linked_urls(planted),
            vec!["https://cdn.example/x.js", "//cdn.example/y.css"]
        );
    }

    #[tokio::test]
    async fn the_scalar_bundle_is_served_from_the_vendored_file_with_an_immutable_cache() {
        let response = get(BUNDLE).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header(&response, "content-type"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            header(&response, "cache-control"),
            "public, max-age=31536000, immutable"
        );

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.len(), 4_930_805, "the vendored 1.77.0 standalone.js");
        assert_eq!(&bytes[..], SCALAR_BUNDLE);
    }

    #[tokio::test]
    async fn the_old_swagger_paths_redirect_permanently_to_their_new_homes() {
        for (old, new) in [
            ("/docs/swagger", "/docs/v1"),
            ("/docs/swagger/", "/docs/v1"),
            ("/docs/swagger/openapi.json", "/docs/api/v1/openapi.json"),
        ] {
            let response = get(old).await;
            assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT, "{old}");
            assert_eq!(header(&response, "location"), new, "{old}");
        }
    }

    #[tokio::test]
    async fn the_spec_the_page_reads_is_served() {
        let response = get("/docs/api/v1/openapi.json").await;
        assert_eq!(response.status(), StatusCode::OK);
        let spec: serde_json::Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert!(
            spec["openapi"].as_str().is_some_and(|v| v.starts_with('3')),
            "{spec}"
        );
        assert!(
            spec["paths"].as_object().is_some_and(|p| !p.is_empty()),
            "{spec}"
        );
    }
}

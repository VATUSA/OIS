use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request")]
    BadRequest,
    #[error("oauth login origin mismatch")]
    OAuthLoginOriginMismatch,
    #[error("oauth state cookie missing")]
    OAuthStateCookieMissing,
    #[error("oauth state mismatch")]
    OAuthStateMismatch,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("conflict")]
    Conflict,
    /// A conflict the caller can act on, named by a stable code — e.g. `held_by_person` when a machine
    /// tries to replace a release a person holds (#585).
    #[error("conflict: {0}")]
    ConflictReason(&'static str),
    /// An `If-Match` / `If-None-Match` precondition did not hold (#585). Carries the resource's current
    /// version, sent back as the `ETag` so the caller can see what it lost to — `None` when there is
    /// nothing there now.
    #[error("precondition failed")]
    PreconditionFailed { etag: Option<i64> },
    /// A write that must be conditional arrived without `If-Match` or `If-None-Match` (#585).
    #[error("precondition required")]
    PreconditionRequired,
    /// A request body over its route's limit (#629).
    #[error("payload too large")]
    PayloadTooLarge,
    /// The caller has hit a limit on how often it may do this (#629: diagnostics reports per hour).
    #[error("too many requests")]
    TooManyRequests,
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("internal server error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::OAuthLoginOriginMismatch => {
                (StatusCode::BAD_REQUEST, "oauth_login_origin_mismatch")
            }
            Self::OAuthStateCookieMissing => {
                (StatusCode::BAD_REQUEST, "oauth_state_cookie_missing")
            }
            Self::OAuthStateMismatch => (StatusCode::BAD_REQUEST, "oauth_state_mismatch"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::ConflictReason(code) => (StatusCode::CONFLICT, code),
            Self::PreconditionFailed { etag } => {
                let mut response = (
                    StatusCode::PRECONDITION_FAILED,
                    Json(ErrorBody {
                        error: "precondition_failed",
                    }),
                )
                    .into_response();
                if let Some(version) = etag
                    && let Ok(value) = axum::http::HeaderValue::from_str(&format!("\"{version}\""))
                {
                    response
                        .headers_mut()
                        .insert(axum::http::header::ETAG, value);
                }
                return response;
            }
            Self::PreconditionRequired => {
                (StatusCode::PRECONDITION_REQUIRED, "precondition_required")
            }
            Self::PayloadTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"),
            Self::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests"),
            Self::ServiceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}

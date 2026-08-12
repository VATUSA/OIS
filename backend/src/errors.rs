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
            Self::ServiceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        (status, Json(ErrorBody { error: message })).into_response()
    }
}

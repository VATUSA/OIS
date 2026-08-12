//! Serves the generated OpenAPI document.

use axum::Json;
use utoipa::OpenApi;

use crate::openapi::ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

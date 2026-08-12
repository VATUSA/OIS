//! OpenAPI document for the OIS API. Emitted at `/docs/api/v1/openapi.json`; the web
//! + desktop clients are generated from it (see docs/architecture/api-conventions.md).

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "OIS API",
        version = "0.1.0",
        description = "VATUSA Event Operational Information System API"
    ),
    paths(
        crate::handlers::health::health,
        crate::handlers::auth::me,
        crate::handlers::auth::vatsim_login,
        crate::handlers::auth::vatsim_callback,
        crate::handlers::auth::logout,
        crate::handlers::users::search_users,
        crate::handlers::facilities::list_facilities,
        crate::handlers::facilities::get_facility,
        crate::handlers::access::get_access_catalog,
        crate::handlers::access::get_self_access,
        crate::handlers::access::get_user_access,
        crate::handlers::access::update_user_access,
        crate::handlers::audit::list_audit_logs,
        crate::handlers::service_accounts::list_service_accounts,
        crate::handlers::service_accounts::create_service_account,
        crate::handlers::service_accounts::rotate_service_account,
        crate::handlers::service_accounts::disable_service_account,
        crate::handlers::service_accounts::set_service_account_roles,
    ),
    components(schemas(
        crate::models::MeBody,
        crate::models::UserSummary,
        crate::models::FacilityBody,
        crate::models::AccessCatalogBody,
        crate::models::SelfAccessBody,
        crate::models::UserAccessBody,
        crate::models::ScopeAccess,
        crate::models::UpdateUserAccessRequest,
        crate::models::ScopeUpdate,
        crate::models::AuditLogEntry,
        crate::models::AuditLogPage,
        crate::models::CreateServiceAccountRequest,
        crate::models::SetServiceAccountRolesRequest,
        crate::models::ServiceAccountBody,
        crate::models::ServiceAccountTokenBody,
    )),
    tags(
        (name = "system", description = "Health"),
        (name = "auth", description = "Authentication + current user"),
        (name = "users", description = "User directory"),
        (name = "facilities", description = "ARTCC directory"),
        (name = "access", description = "Fine-grained access control"),
        (name = "audit", description = "Audit log"),
        (name = "service-accounts", description = "Machine client credentials")
    )
)]
pub struct ApiDoc;

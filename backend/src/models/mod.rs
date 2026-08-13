use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// The `/me` response for an authenticated session.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeBody {
    pub id: String,
    pub cid: i64,
    pub email: String,
    pub display_name: String,
    pub rating: Option<String>,
    pub server_admin: bool,
    pub role_names: Vec<String>,
    /// Effective permissions as the nested tree the access editor renders.
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// A lightweight user match for the directory search.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct UserSummary {
    pub cid: i64,
    pub display_name: String,
    pub rating: Option<String>,
}

/// A VATUSA facility (ARTCC). `artcc_id` scope values reference `id`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FacilityBody {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
    pub active: bool,
}

/// Assignable roles + permission catalog + facilities for the access editor.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccessCatalogBody {
    pub roles: Vec<String>,
    /// Every assignable permission as the nested checkbox tree.
    #[schema(value_type = Object)]
    pub permissions: Value,
    /// ARTCCs a grant can be scoped to (drives the scope selector in the editor).
    pub facilities: Vec<FacilityBody>,
}

/// The acting user's own effective access (staff debug view).
#[derive(Debug, Serialize, ToSchema)]
pub struct SelfAccessBody {
    pub server_admin: bool,
    pub role_names: Vec<String>,
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// A target user's editable access: direct permission grants + role assignments,
/// grouped by scope (national first, then each ARTCC the user has grants/roles in).
#[derive(Debug, Serialize, ToSchema)]
pub struct UserAccessBody {
    pub id: String,
    pub cid: i64,
    pub server_admin: bool,
    pub scopes: Vec<ScopeAccess>,
}

/// Direct grants + roles at one scope. `artcc_id = null` is national.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScopeAccess {
    pub artcc_id: Option<String>,
    pub role_names: Vec<String>,
    /// Direct permission grants at this scope, as the nested checkbox tree.
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// The editor's SAVE payload. `reason` is required (audited). Each entry in `scopes`
/// replaces that scope's direct permission grants; when its `role_names` is present it
/// also replaces the assignable-role set at that scope. Scopes not listed are untouched.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserAccessRequest {
    pub reason: String,
    pub scopes: Vec<ScopeUpdate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScopeUpdate {
    /// null / omitted = national scope; otherwise a known ARTCC id.
    #[serde(default)]
    pub artcc_id: Option<String>,
    #[schema(value_type = Object)]
    pub permissions: Value,
    #[serde(default)]
    pub role_names: Option<Vec<String>>,
}

// --- audit log ---

/// One audit-log entry (the "recorded on this controller's log" trail).
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogEntry {
    pub id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub artcc_id: Option<String>,
    pub reason: Option<String>,
    pub actor_cid: Option<i64>,
    pub actor_display_name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub before_state: Option<Value>,
    #[schema(value_type = Option<Object>)]
    pub after_state: Option<Value>,
    pub created_at: DateTime<Utc>,
}

/// A page of audit-log entries.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogPage {
    pub items: Vec<AuditLogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

// --- tmu: traffic management initiatives (TMIs) ---

/// A Traffic Management Initiative.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct TmiBody {
    pub id: String,
    /// Requesting facility (ARTCC/TRACON).
    pub requesting: String,
    /// Providing facility (ARTCC/TRACON).
    pub providing: String,
    pub restriction: String,
    pub start_time: DateTime<Utc>,
    pub stop_time: Option<DateTime<Utc>>,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Author display name (from the creating user).
    pub author: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTmiRequest {
    pub requesting: String,
    pub providing: String,
    pub restriction: String,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub stop_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTmiRequest {
    #[serde(default)]
    pub requesting: Option<String>,
    #[serde(default)]
    pub providing: Option<String>,
    #[serde(default)]
    pub restriction: Option<String>,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub stop_time: Option<DateTime<Utc>>,
}

// --- TMU CFR / departures ---

/// A locked (issued) Call-For-Release wheels-up time.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct IssuedCfrBody {
    pub callsign: String,
    pub airport: String,
    pub wheels_up: DateTime<Utc>,
    pub issued_by: Option<String>,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IssueCfrRequest {
    pub callsign: String,
    /// The metered arrival airport this departure is bound for.
    pub airport: String,
    /// Explicit wheels-up to lock; when omitted, the flight's proposed CFR is used.
    #[serde(default)]
    pub ready_time: Option<DateTime<Utc>>,
}

/// A pending ground/proposed departure out of a field (for the Departures view).
#[derive(Debug, Serialize, ToSchema)]
pub struct DepartureFlight {
    pub callsign: String,
    /// Origin airport ICAO (a facility query spans several).
    pub dep: String,
    /// Destination airport.
    pub arrival: String,
    pub aircraft_type: String,
    pub gate: Option<String>,
    pub status: String,
    /// True when the destination has a TMU program (so this departure is metered).
    pub has_program: bool,
    pub eta: Option<DateTime<Utc>>,
    pub sta: Option<DateTime<Utc>>,
    pub delay_min: i64,
    pub cfr: Option<DateTime<Utc>>,
    pub cfr_issued: bool,
    pub seq: Option<i64>,
}

/// The Departures view for one field (airport, TRACON, or ARTCC) plus summary counts.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeparturesResponse {
    /// `tracon` | `artcc` when the queried field is a facility, else null (plain airport).
    pub facility_kind: Option<String>,
    /// The airports the field resolved to (one for an airport; several for a facility).
    pub airports: Vec<String>,
    pub total: usize,
    /// How many departures are bound for a metered destination.
    pub to_metered: usize,
    /// How many are currently held by a metering delay.
    pub holding_on_cfr: usize,
    /// The destinations (across all origin airports) that have a TMU program.
    pub program_destinations: Vec<String>,
    pub departures: Vec<DepartureFlight>,
}

// --- TMU ground stops ---

/// A ground stop: holds departures into `airport` from within `scope` until `until`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct GroundStopBody {
    pub id: String,
    pub airport: String,
    /// Space-separated ARTCC/FIR codes; empty = every departure (field-wide).
    pub scope: String,
    /// HHMM Zulu clock time the stop runs until; null = until further notice.
    pub until: Option<String>,
    /// Lifecycle: draft | published | expired | cancelled.
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// Display name of whoever last touched the stop.
    pub updated_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGroundStopRequest {
    pub airport: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
}

// --- events (per-event planning) ---

/// A VATUSA event, cached from the events API — the anchor for per-event planning.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct EventBody {
    /// VATUSA event id.
    pub id: i64,
    pub title: String,
    /// HTML/BBCode blurb straight from VATUSA (render sanitized on the client).
    pub body: String,
    pub banner_image_url: String,
    /// Host ARTCC id (e.g. ZTL).
    pub facility: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    /// VATUSA review state (e.g. "approved").
    pub review_status: String,
}

// --- TMU rate programs ---

/// One per-gate restriction inside a program: an arrival fix/STAR with its own spacing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GateRule {
    /// Arrival fix / STAR name (uppercase alphanumeric).
    pub name: String,
    /// Minutes-in-trail for this gate (0 = use MIT or airport default).
    #[serde(default)]
    pub trail: i32,
    /// Miles-in-trail for this gate (0 = unused; overrides `trail` when > 0).
    #[serde(default)]
    pub mit: i32,
}

/// An airport rate program (vatflow "TMU tab"). Keyed by ICAO.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct ProgramBody {
    pub icao: String,
    pub aar: i32,
    /// Airport-wide minutes-in-trail default.
    pub trail: i32,
    /// Airport-wide miles-in-trail (overrides `trail` when > 0).
    pub mit: i32,
    #[schema(value_type = Vec<GateRule>)]
    pub gates: sqlx::types::Json<Vec<GateRule>>,
    pub exclude_wake: Vec<String>,
    pub exclude_types: Vec<String>,
    pub jets_only: bool,
    /// Scheduled end; null = indefinite. Auto-removed an hour after this time.
    pub active_until: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// Display name of whoever last edited the program.
    pub updated_by: Option<String>,
}

/// Upsert a program (`PUT /tmu/programs/{icao}`) — the full normalized program body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertProgramRequest {
    pub aar: i32,
    #[serde(default)]
    pub trail: i32,
    #[serde(default)]
    pub mit: i32,
    #[serde(default)]
    pub gates: Vec<GateRule>,
    #[serde(default)]
    pub exclude_wake: Vec<String>,
    #[serde(default)]
    pub exclude_types: Vec<String>,
    #[serde(default)]
    pub jets_only: bool,
    #[serde(default)]
    pub active_until: Option<DateTime<Utc>>,
}

// --- service accounts ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServiceAccountRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetServiceAccountRolesRequest {
    pub role_names: Vec<String>,
}

/// A service account as listed (no secret). `roles` are its granted role names.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountBody {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub roles: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Returned once on create/rotate — the plaintext bearer token is never stored or
/// shown again.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountTokenBody {
    pub account: ServiceAccountBody,
    pub token: String,
}

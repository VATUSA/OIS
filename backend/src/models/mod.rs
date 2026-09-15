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
    /// VATUSA member details, once synced (null until the first successful sync).
    pub vatusa: Option<VatusaProfile>,
}

/// A signed-in member's VATUSA details, surfaced on their profile.
#[derive(Debug, Serialize, ToSchema)]
pub struct VatusaProfile {
    pub home_facility: Option<String>,
    pub rating_numeric: Option<i32>,
    pub home_controller: Option<bool>,
    pub facility_join: Option<DateTime<Utc>>,
    pub synced_at: Option<DateTime<Utc>>,
    pub roles: Vec<VatusaRoleEntry>,
    /// Facilities the member visits.
    pub visits: Vec<String>,
}

/// One VATUSA role, e.g. `INS` at `ZDC`.
#[derive(Debug, Serialize, ToSchema)]
pub struct VatusaRoleEntry {
    pub facility: String,
    pub role: String,
}

/// A lightweight user match for the directory search.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct UserSummary {
    pub cid: i64,
    pub display_name: String,
    pub rating: Option<String>,
}

/// One row of the access-admin "all users" browser: identity plus the distinct role names the user
/// holds across any scope (for at-a-glance badges).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AdminUserRow {
    pub cid: i64,
    pub display_name: String,
    pub rating: Option<String>,
    pub roles: Vec<String>,
}

/// A page of the access-admin user browser.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminUserPage {
    pub items: Vec<AdminUserRow>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// A VATUSA facility (ARTCC). `artcc_id` scope values reference `id`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FacilityBody {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
    pub active: bool,
}

/// A configured reference document (SOP/LOA/etc.) for a facility.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FacilityDocumentBody {
    pub id: String,
    pub facility_id: String,
    pub title: String,
    pub url: String,
    pub updated_at: DateTime<Utc>,
    /// Whether the requesting user may edit this facility's documents (per their ARTCC scope).
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertFacilityDocumentRequest {
    pub title: String,
    pub url: String,
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
    /// `user`, `api_key`, `service_account`, or `system` — how the action was authenticated.
    pub actor_type: Option<String>,
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

// --- taxi insights (#183): browsable history over raw observations + derived estimates ---

/// One raw pushback+taxi observation (#164 sub-issue C).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct TaxiObservationEntry {
    pub id: i64,
    pub airport: String,
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub pushback_sec: Option<i32>,
    pub taxi_sec: i32,
    pub observed_at: DateTime<Utc>,
    /// True when this row falls outside `taxi_estimate`'s own sanity-clamp bounds — computed at
    /// query time, never persisted.
    pub is_outlier: bool,
}

/// A page of raw observations.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiObservationPage {
    pub items: Vec<TaxiObservationEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// One derived per-(gate, aircraft, runway) estimate (#164 sub-issue D), computed live from the
/// currently filtered sample set — not a stored row.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiEstimateEntry {
    pub airport: String,
    pub gate_id: Option<String>,
    pub aircraft: Option<String>,
    pub runway: Option<String>,
    pub pushback_sec: f64,
    pub pushback_tier: String,
    pub pushback_sample_count: i64,
    pub taxi_sec: f64,
    pub taxi_tier: String,
    pub taxi_sample_count: i64,
}

/// A page of derived estimates.
#[derive(Debug, Serialize, ToSchema)]
pub struct TaxiEstimatePage {
    pub items: Vec<TaxiEstimateEntry>,
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
    /// The canonical raw NTML line (typed directly, or encoded from `structured`).
    pub restriction: String,
    pub start_time: DateTime<Utc>,
    pub stop_time: Option<DateTime<Utc>>,
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Author display name (from the creating user).
    pub author: Option<String>,
    /// Parsed NTML fields when built with the structured form (null for a raw-typed TMI).
    #[schema(value_type = Option<NtmlRestriction>)]
    pub structured: Option<sqlx::types::Json<NtmlRestriction>>,
    /// Plain-English rendering of `structured` for pilots (null for a raw-typed TMI).
    pub decoded: Option<String>,
}

/// A structured NTML restriction (the form-built TMI content). Encoded to the raw `restriction` line
/// and rendered to `decoded` English by `crate::tmi`. The requesting/providing facilities and the
/// valid window live on the TMI itself, not here.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NtmlRestriction {
    /// Airport/facility element, e.g. `JFK` or `EWR,LGA`.
    pub element: String,
    /// `arrivals` | `departures` | `enroute`.
    pub direction: String,
    /// Fix / NAVAID / airway the restriction is via, e.g. `CAMRN`, `J152`.
    #[serde(default)]
    pub via: Option<String>,
    /// Restriction type: `MIT` | `MINIT` | `STOP` | `DSP` | `APREQ` | `TBM` | `CFR` | `TXT`.
    pub kind: String,
    /// Value for `MIT` (miles) / `MINIT` (minutes).
    #[serde(default)]
    pub value: Option<i32>,
    /// Free text when `kind = TXT`.
    #[serde(default)]
    pub text: Option<String>,
    /// Qualifier, e.g. `NO STACKS`, `PER AIRPORT`, `AS ONE`, `SINGLE STREAM`.
    #[serde(default)]
    pub qualifier: Option<String>,
    /// Aircraft type: `ALL` | `JET` | `PROP` | `TURBOPROP`.
    #[serde(default)]
    pub aircraft: Option<String>,
    /// Speed limit, e.g. `{ op: "≤", value: 210 }`.
    #[serde(default)]
    pub speed: Option<Bound>,
    /// Altitude limit, e.g. `{ op: "AOB", value: 90 }` (FL090).
    #[serde(default)]
    pub altitude: Option<Bound>,
    /// Impacting-condition category, e.g. `VOLUME`, `WEATHER`, `EQUIPMENT`.
    #[serde(default)]
    pub condition: Option<String>,
    /// Impacting-condition detail, e.g. `THUNDERSTORMS`, `STARS`.
    #[serde(default)]
    pub condition_detail: Option<String>,
    /// Excluded facilities/airports, e.g. `["PHL"]`.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A speed/altitude limit: an operator plus a value. Speed ops are `=`/`≤`/`≥`; altitude ops are
/// `AT`/`AOB` (at or below) / `AOA` (at or above).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Bound {
    pub op: String,
    pub value: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTmiRequest {
    pub requesting: String,
    pub providing: String,
    /// Raw NTML line for a raw-typed TMI; ignored (overwritten by the encoded line) when `structured`
    /// is present.
    #[serde(default)]
    pub restriction: String,
    /// Structured NTML fields — when present the backend encodes the raw line + renders the decoded
    /// English from these, and `restriction` is derived.
    #[serde(default)]
    pub structured: Option<NtmlRestriction>,
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

/// A Ground Delay Program — meters inbound demand to a constrained airport to its AAR.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct GdpBody {
    pub id: String,
    pub airport: String,
    /// Airport Acceptance Rate (arrivals/hour) the program meters to.
    pub aar: i32,
    /// Space-separated departure ARTCC codes in scope; empty = all departures.
    pub scope: String,
    /// HHMM Zulu program window start.
    pub start_time: String,
    /// HHMM Zulu program window end.
    pub end_time: String,
    /// Scope tier: only inbounds within this many enroute minutes are controllable; null = no limit.
    pub max_enroute_min: Option<i32>,
    pub exempt_airborne: bool,
    /// Rate changes across the window (empty = flat AAR).
    #[schema(value_type = Vec<AarStep>)]
    pub aar_steps: sqlx::types::Json<Vec<AarStep>>,
    /// Lifecycle: draft | published | expired | cancelled.
    pub status: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// Display name of whoever last touched the program.
    pub updated_by: Option<String>,
}

/// One rate change within a GDP window: the AAR takes effect at `start_time` (HHMM Zulu)
/// and holds until the next step or the window end.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AarStep {
    /// HHMM Zulu when this rate begins.
    pub start_time: String,
    pub aar: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGdpRequest {
    pub airport: String,
    pub aar: i32,
    pub start_time: String,
    pub end_time: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub max_enroute_min: Option<i32>,
    #[serde(default = "default_true")]
    pub exempt_airborne: bool,
    /// Optional rate changes across the window (empty = flat AAR).
    #[serde(default)]
    pub aar_steps: Vec<AarStep>,
}

/// Revise a GDP — a full replace of its mutable fields (the airport can't change).
/// Revising a *published* program re-rations off the live feed and re-freezes control
/// times, so issued EDCTs may shift.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateGdpRequest {
    pub aar: i32,
    pub start_time: String,
    pub end_time: String,
    #[serde(default)]
    pub scope: Option<String>,
    /// null clears the distance tier.
    #[serde(default)]
    pub max_enroute_min: Option<i32>,
    #[serde(default = "default_true")]
    pub exempt_airborne: bool,
    /// Optional rate changes across the window (empty = flat AAR).
    #[serde(default)]
    pub aar_steps: Vec<AarStep>,
}

fn default_true() -> bool {
    true
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
    /// Stats-capture state (list view only): `off` | `scheduled` | `recording` | `recorded`.
    /// Empty from `GET /events/{id}` and the VATUSA sync.
    #[sqlx(default)]
    pub recording: String,
    /// True when the event has an ACE support request that wasn't cancelled — support was
    /// requested (still open, or since completed). List view only; always `false` from
    /// `GET /events/{id}`.
    #[sqlx(default)]
    pub ace_requested: bool,
    /// True when any facility has been added to the event's facility support (list view only).
    #[sqlx(default)]
    pub facility_support: bool,
}

/// Whether an event needs national DCC support.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct DccRequestBody {
    /// not_needed | requested | confirmed
    pub status: String,
    pub notes: String,
    pub updated_at: Option<DateTime<Utc>>,
    /// Display name of whoever last set it.
    pub updated_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDccRequest {
    /// not_needed | requested | confirmed
    pub status: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// One facility's involvement in an event — its support level plus why it was surfaced.
///
/// The list is *derived*: a facility appears because it hosts the event, owns a configured airport,
/// or has an ACE staffing request — or because a support row was saved for it. `stored` is false for
/// a purely-derived suggestion the planner hasn't confirmed yet.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FacilitySupportBody {
    /// ARTCC/TRACON id (e.g. ZTL, N90).
    pub facility: String,
    /// required | preferred | not_required
    pub level: String,
    pub notes: String,
    /// When the support row was last saved; null for an unconfirmed (derived-only) suggestion.
    pub updated_at: Option<DateTime<Utc>>,
    pub updated_by: Option<String>,
    /// This facility hosts the event (`event.facility`).
    #[sqlx(default)]
    pub is_host: bool,
    /// ICAOs of the event's configured airports that this facility owns.
    #[sqlx(default)]
    pub airports: Vec<String>,
    /// This facility has an ACE staffing request on the event.
    #[sqlx(default)]
    pub has_staffing: bool,
    /// A support row exists (the planner has confirmed a level), vs. a derived-only suggestion.
    #[sqlx(default)]
    pub stored: bool,
    /// Whether the requesting user may edit this facility's support (per their ARTCC scope).
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertFacilitySupportRequest {
    /// required | preferred | not_required
    pub level: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A planned per-airport arrival/departure rate for an event.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AirportRateBody {
    pub icao: String,
    /// Airport arrival rate (per hour).
    pub aar: i32,
    /// Airport departure rate (per hour).
    pub adr: i32,
    /// Owning ARTCC resolved when set; empty if unknown.
    pub artcc: String,
    /// The airport config this rate came from (if chosen from the airport's configs); null = manual.
    pub config_id: Option<String>,
    /// `predicted` (from the forecast wind) or `override` (manually set).
    #[sqlx(default)]
    pub source: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
    /// Whether the requesting user may edit this airport's rate (per their ARTCC scope).
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAirportRateRequest {
    pub aar: i32,
    pub adr: i32,
    /// Optional: the airport config this rate was chosen from.
    #[serde(default)]
    pub config_id: Option<String>,
    /// `predicted` or `override`; defaults to `override` when omitted.
    #[serde(default)]
    pub source: Option<String>,
}

/// A reusable per-airport runway configuration (named, with a favored-wind rule + AAR/ADR).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AirportConfigBody {
    pub id: String,
    pub icao: String,
    pub name: String,
    pub aar: i32,
    pub adr: i32,
    pub landing_runways: Vec<String>,
    /// Favored-wind rule: applies when the surface wind direction is within [from, to] (wrap-around
    /// allowed). Ignored when `calm_default`.
    pub wind_from_deg: i32,
    pub wind_to_deg: i32,
    /// Used when the wind is light/variable or nothing matches (at most one per airport).
    pub calm_default: bool,
    pub artcc: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
    /// Whether the requesting user may edit this airport's configs (per their ARTCC scope).
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAirportConfigRequest {
    pub name: String,
    pub aar: i32,
    pub adr: i32,
    #[serde(default)]
    pub landing_runways: Vec<String>,
    pub wind_from_deg: i32,
    pub wind_to_deg: i32,
    #[serde(default)]
    pub calm_default: bool,
}

/// An airport surface point (a gate or parking position). Runways are not modeled here — they're
/// already covered by the bundled OurAirports data in `feed::runway_db`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AirportGateBody {
    pub id: String,
    pub icao: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// `manual` | `osm` | `crc`.
    pub source: String,
    pub updated_at: DateTime<Utc>,
    /// Whether the requesting user may edit this airport's surface data (per their ARTCC scope).
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAirportGateRequest {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// An airport ramp or apron area. `rings` is an array of rings, each an array of `[lat, lon]`.
/// Every row created so far (manual entry, the OSM seed) has exactly one ring — the outer
/// boundary — and the map editor (`SurfaceMap.tsx`) only ever draws/edits that first ring; nothing
/// in `validate_ramp_area` rejects further rings (e.g. a hole), so a future ingestion path (a CRC
/// profile/GeoJSON import) producing one isn't a contract violation — the editor preserves any
/// rings beyond the first untouched rather than dropping them on save.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AirportRampAreaBody {
    pub id: String,
    pub icao: String,
    pub name: String,
    /// `ramp` | `apron`.
    pub kind: String,
    #[schema(value_type = Vec<Vec<Vec<f64>>>)]
    pub rings: sqlx::types::Json<Vec<Vec<[f64; 2]>>>,
    /// `manual` | `osm` | `crc`.
    pub source: String,
    pub updated_at: DateTime<Utc>,
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAirportRampAreaRequest {
    pub name: String,
    pub kind: String,
    pub rings: Vec<Vec<[f64; 2]>>,
}

/// An airport taxiway centerline. `points` is an ordered array of `[lat, lon]`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AirportTaxiwayBody {
    pub id: String,
    pub icao: String,
    pub name: String,
    #[schema(value_type = Vec<Vec<f64>>)]
    pub points: sqlx::types::Json<Vec<[f64; 2]>>,
    /// `manual` | `osm` | `crc`.
    pub source: String,
    pub updated_at: DateTime<Utc>,
    #[sqlx(default)]
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAirportTaxiwayRequest {
    pub name: String,
    pub points: Vec<[f64; 2]>,
}

/// An airport's full surface geometry, combined for one read.
#[derive(Debug, Serialize, ToSchema)]
pub struct AirportSurfaceBody {
    pub gates: Vec<AirportGateBody>,
    pub ramp_areas: Vec<AirportRampAreaBody>,
    pub taxiways: Vec<AirportTaxiwayBody>,
}

/// A configurable aircraft performance profile (climb / cruise / descent schedules) used by the
/// trajectory / ETA model. Keyed by `kind` (`type` / `wake` / `default`) + `key` (ICAO type, wake
/// token, or empty). See migration 0059 and `feed::trajectory`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AircraftProfileBody {
    pub kind: String,
    pub key: String,
    pub name: String,
    pub climb_ias_lo: f64,
    pub climb_ias_hi: f64,
    pub climb_mach: Option<f64>,
    pub climb_fpm_lo: f64,
    pub climb_fpm_hi: f64,
    pub cruise_tas: Option<f64>,
    pub cruise_mach: Option<f64>,
    pub service_ceiling_ft: f64,
    pub desc_mach: Option<f64>,
    pub desc_ias_hi: f64,
    pub desc_ias_lo: f64,
    pub desc_fpm: f64,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

/// Create or update an aircraft performance profile. `kind` + `key` come from the URL path.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAircraftProfileRequest {
    #[serde(default)]
    pub name: String,
    pub climb_ias_lo: f64,
    pub climb_ias_hi: f64,
    #[serde(default)]
    pub climb_mach: Option<f64>,
    pub climb_fpm_lo: f64,
    pub climb_fpm_hi: f64,
    #[serde(default)]
    pub cruise_tas: Option<f64>,
    #[serde(default)]
    pub cruise_mach: Option<f64>,
    pub service_ceiling_ft: f64,
    #[serde(default)]
    pub desc_mach: Option<f64>,
    pub desc_ias_hi: f64,
    pub desc_ias_lo: f64,
    pub desc_fpm: f64,
}

/// The forecast wind at an airport for a given time (Open-Meteo, or live METAR fallback).
#[derive(Debug, Serialize, ToSchema)]
pub struct AirportForecastBody {
    pub icao: String,
    /// ISO time of the forecast hour returned.
    pub time: DateTime<Utc>,
    /// Surface wind direction (degrees true), or null if calm/variable.
    pub wind_dir: Option<i32>,
    pub wind_kt: i32,
    pub gust_kt: Option<i32>,
    /// `forecast` (Open-Meteo) or `metar` (live fallback) or `none`.
    pub source: String,
}

/// One draft TMI inside a package (kind + the create-shape payload for that kind).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct TmiPackageItemBody {
    pub id: String,
    /// program | restriction | ground_stop
    pub kind: String,
    #[schema(value_type = Object)]
    pub payload: sqlx::types::Json<Value>,
}

/// A named bundle of draft TMIs for an event; activating it creates live tmu.* rows.
#[derive(Debug, Serialize, ToSchema)]
pub struct TmiPackageBody {
    pub id: String,
    pub name: String,
    /// draft | activated | archived
    pub status: String,
    /// Whether this package auto-activates 30 min before the event starts.
    pub auto_publish: bool,
    pub activated_at: Option<DateTime<Utc>>,
    /// When the package was deactivated (its live rows cancelled); null unless archived.
    pub archived_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
    pub items: Vec<TmiPackageItemBody>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePackageRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddPackageItemRequest {
    /// program | restriction | ground_stop
    pub kind: String,
    #[schema(value_type = Object)]
    pub payload: Value,
}

// --- per-event stats capture + generated stats ---

/// An event's stats-capture config plus the current capture status.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventCaptureBody {
    /// Whether automatic capture is enabled for this event.
    pub enabled: bool,
    /// Minutes before the event start / after the event end to include in the capture window.
    pub pre_minutes: i32,
    pub post_minutes: i32,
    pub updated_at: Option<DateTime<Utc>>,
    pub updated_by: Option<String>,
    /// Current capture state for the event: `open` (recording), `saved` (kept), or null (none yet).
    pub capture_status: Option<String>,
    /// The capture's id (for map replay), when one exists.
    pub capture_id: Option<String>,
    pub capture_start: Option<DateTime<Utc>>,
    pub capture_end: Option<DateTime<Utc>>,
    /// Whether the requesting user may change the capture config.
    pub can_edit: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEventCaptureRequest {
    pub enabled: bool,
    #[serde(default)]
    pub pre_minutes: Option<i32>,
    #[serde(default)]
    pub post_minutes: Option<i32>,
}

/// Debrief stats for one featured (configured) event airport over the capture window.
#[derive(Debug, Serialize, ToSchema)]
pub struct AirportStatBody {
    pub icao: String,
    pub arrivals: i64,
    pub departures: i64,
    /// arrivals + departures.
    pub movements: i64,
    /// Distinct pilots (CIDs) that arrived at or departed from this airport.
    pub unique_pilots: i64,
    /// Top aircraft types to/from this airport.
    pub top_aircraft: Vec<KeyCountBody>,
}

/// The featured airports combined: arrivals/departures summed across airports; pilots deduped
/// (a pilot flying between two featured airports counts once); top aircraft across them all.
#[derive(Debug, Serialize, ToSchema)]
pub struct CombinedStatBody {
    pub arrivals: i64,
    pub departures: i64,
    pub movements: i64,
    pub unique_pilots: i64,
    pub top_aircraft: Vec<KeyCountBody>,
}

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct KeyCountBody {
    pub key: Option<String>,
    pub count: i64,
}

/// One aggregated delay group — a set of flight legs sharing an airport, runway, or procedure.
#[derive(Debug, Serialize, ToSchema)]
pub struct DelayGroup {
    /// The group value (airport ICAO / runway id / procedure base name); empty for the overall row.
    pub key: String,
    pub count: i64,
    /// Mean / median / 90th-percentile leg duration, in seconds.
    pub avg_sec: i64,
    pub median_sec: i64,
    pub p90_sec: i64,
}

/// Average-delay summary for one leg `kind` over a rolling window, with optional filters applied.
#[derive(Debug, Serialize, ToSchema)]
pub struct DelaySummary {
    /// `departure` (taxi-out) or `arrival` (transit).
    pub kind: String,
    pub window_hours: i64,
    /// Aggregate over the whole filtered set.
    pub overall: DelayGroup,
    /// Per-airport aggregates, paginated (`page`/`page_size` below); each airport's `median_sec`
    /// is its normalization baseline.
    pub by_airport: Vec<DelayGroup>,
    /// Total distinct airports matching the filters (across all pages).
    pub by_airport_total: i64,
    /// 1-based page number and page size `by_airport` was fetched with.
    pub page: i64,
    pub page_size: i64,
    /// Per-runway / per-procedure breakdowns — populated only when an `airport` filter is set.
    /// Unpaginated: inherently small, scoped to one airport.
    pub by_runway: Vec<DelayGroup>,
    pub by_procedure: Vec<DelayGroup>,
}

// --- stats read API (/api/v1/stats/*) ---

/// Serialize an i64 session id as a JSON string — the hashed ids exceed JS's safe-integer range,
/// so a number would lose precision in the browser and break flight links.
pub(crate) mod id_str {
    use serde::Serializer;
    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
}

/// One hour of network totals (from `stats.snapshot`).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct NetworkPointBody {
    pub hour: DateTime<Utc>,
    pub avg_pilots: Option<i32>,
    pub peak_pilots: Option<i32>,
    pub avg_controllers: Option<i32>,
    pub peak_clients: Option<i32>,
}

/// A compact flight row for lists (airport movements, member history).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct StatsFlightSummary {
    #[serde(serialize_with = "id_str::serialize")]
    #[schema(value_type = String)]
    pub session_id: i64,
    pub callsign: String,
    pub status: String,
    pub logon_time: DateTime<Utc>,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub aircraft_short: Option<String>,
    pub duration_s: Option<i32>,
    pub distance_nm: Option<f32>,
}

/// Full flight metadata + summary.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct StatsFlightDetail {
    #[serde(serialize_with = "id_str::serialize")]
    #[schema(value_type = String)]
    pub session_id: i64,
    pub cid: i32,
    pub callsign: String,
    pub server: Option<String>,
    pub status: String,
    pub logon_time: DateTime<Utc>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub alternate: Option<String>,
    pub aircraft_short: Option<String>,
    pub cruise_alt: Option<i32>,
    pub route: Option<String>,
    pub duration_s: Option<i32>,
    pub distance_nm: Option<f32>,
    pub max_altitude: Option<i32>,
    pub max_groundspeed: Option<i32>,
    /// Flight-plan revisions over the flight, oldest first (one entry = never amended). Populated
    /// separately from `stats.flight_plan`, so it's skipped by the row mapping.
    #[sqlx(skip)]
    pub revisions: Vec<FlightPlanRevisionBody>,
}

/// One flight-plan revision, for the flight-detail amendment history.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FlightPlanRevisionBody {
    pub effective_from: DateTime<Utc>,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub aircraft_short: Option<String>,
    pub route: Option<String>,
}

/// Airport activity: dep/arr counts + top aircraft / destinations / origins.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsAirportBody {
    pub icao: String,
    pub departures: i64,
    pub arrivals: i64,
    pub top_aircraft: Vec<KeyCountBody>,
    pub top_destinations: Vec<KeyCountBody>,
    pub top_origins: Vec<KeyCountBody>,
}

/// A flight's track: full 15s `positions` if still recent, else the stored simplified path.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsTrackBody {
    /// `full` | `simplified` | `none`
    pub resolution: String,
    #[schema(value_type = Object)]
    pub points: Value,
}

/// A saved/open capture window (for the replay picker).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct CaptureSummaryBody {
    pub id: String,
    pub event_id: Option<i64>,
    pub event_title: Option<String>,
    pub label: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    /// `open` (recording) or `saved`.
    pub status: String,
}

/// Save an already-viewed `[from, to]` window as a permanent, named capture.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveCaptureRequest {
    /// Ties the capture to an event; omit for a manual (ad-hoc) save.
    pub event_id: Option<i64>,
    pub label: String,
    /// Window start (Unix epoch seconds).
    pub from: i64,
    /// Window end (Unix epoch seconds).
    pub to: i64,
}

/// Headline current-size and projected-growth numbers for the `stats` schema, driven by the
/// current raw ingest rate. A simple, explicitly naive projection — it does not model the
/// compaction ladder's ongoing thinning, so it's an upper bound, not a forecast of steady state.
#[derive(Debug, Serialize, ToSchema)]
pub struct StorageForecastBody {
    /// Total on-disk bytes across the `stats` schema's tables (incl. indexes).
    pub total_bytes: i64,
    /// On-disk bytes of `stats.position` alone (the fast-growing table compaction targets).
    pub position_bytes: i64,
    /// Rows ingested into `stats.position` in the last 24h.
    pub daily_ingest_rows: i64,
    /// Naive projected daily growth in bytes, from the current ingest rate and average row size.
    pub daily_growth_bytes: i64,
    /// Naive `total_bytes + 30 * daily_growth_bytes`, assuming no further compaction ever ran.
    pub projected_30d_bytes: i64,
    /// Naive `total_bytes + 90 * daily_growth_bytes`, assuming no further compaction ever ran.
    pub projected_90d_bytes: i64,
}

/// One flight's downsampled track within a replay window.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReplayFlightBody {
    #[serde(serialize_with = "id_str::serialize")]
    #[schema(value_type = String)]
    pub session_id: i64,
    pub callsign: String,
    /// Flight-plan revisions over the window, ascending by `t`. The plan in effect at a given replay
    /// clock is the last entry with `t <= clock`; a flight that never amended has a single entry. So a
    /// mid-route amendment shows the old plan before its `t` and the new plan after.
    pub plans: Vec<ReplayPlan>,
    /// Compact samples: `[t_seconds_from_start, lat, lon, altitude_ft, heading_deg, groundspeed_kt]`.
    #[schema(value_type = Vec<Vec<f64>>)]
    pub samples: Vec<[f64; 6]>,
}

/// One flight-plan revision within a replay window.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReplayPlan {
    /// Seconds from the window start at which this revision took effect (0 = in force at window open).
    pub t: f64,
    pub departure: Option<String>,
    pub arrival: Option<String>,
    pub aircraft: Option<String>,
    /// Filed route string, for plotting the planned route on the replay map.
    pub route: Option<String>,
}

/// Everything needed to replay a capture window on a map.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReplayBody {
    pub capture_id: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// Sample spacing (seconds) the tracks were thinned to.
    pub step_s: i64,
    pub flights: Vec<ReplayFlightBody>,
}

/// One chunk of a progressive replay: the flights (with `t` relative to the window start) that have
/// samples in the requested sub-window, plus each one's callsign and full-window plan timeline.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReplayChunkBody {
    /// The sample spacing (seconds) actually used (the server's adaptive/clamped value).
    pub step_s: i64,
    pub flights: Vec<ReplayFlightBody>,
}

/// Event debrief: per-featured-airport stats plus their combined total, over the capture window.
/// `captured` is false when no capture exists yet.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventStatsBody {
    pub captured: bool,
    /// `open` (still recording) or `saved`.
    pub status: Option<String>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    /// One entry per featured (configured) airport, busiest first.
    pub airports: Vec<AirportStatBody>,
    /// All featured airports combined.
    pub combined: CombinedStatBody,
}

/// An event's free-text post-event debrief notes.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventDebriefBody {
    pub notes: String,
    /// Display name of whoever last edited it.
    pub updated_by: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Whether the caller may edit the notes (holds `events.debrief.create`).
    pub editable: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateEventDebriefRequest {
    pub notes: String,
}

// --- ACE support ---

/// One claim on an ACE request — who took a slot, with their notes + availability window (within the
/// event). Aggregated onto the request via `json_agg`.
#[derive(Debug, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct AceClaimBody {
    pub cid: i64,
    pub display_name: String,
    pub notes: String,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub claimed_at: DateTime<Utc>,
}

/// One ACE support request for an event. `slots` positions are claimed one-per-person; "filled" is
/// derived client-side from `claims_count >= slots`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct AceRequestBody {
    pub id: String,
    pub event_id: i64,
    pub requested_by_cid: Option<i64>,
    pub requested_by_name: Option<String>,
    pub artcc_id: Option<String>,
    pub position: Option<String>,
    pub slots: i32,
    pub details: String,
    /// `open` | `completed` | `cancelled`.
    pub status: String,
    #[schema(value_type = Vec<AceClaimBody>)]
    pub claims: sqlx::types::Json<Vec<AceClaimBody>>,
    pub claims_count: i64,
    pub decided_by_name: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

fn default_ace_slots() -> i32 {
    1
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAceRequestRequest {
    #[serde(default)]
    pub artcc_id: Option<String>,
    #[serde(default)]
    pub position: Option<String>,
    #[serde(default = "default_ace_slots")]
    pub slots: i32,
    pub details: String,
}

/// Claim a slot on an ACE request, with the claimer's notes + availability window.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ClaimAceRequest {
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecideAceRequestRequest {
    /// `completed` or `cancelled`.
    pub outcome: String,
}

/// Outcome of generating Tier-1 support requests for an FNO: the neighbouring ARTCCs a request was
/// opened for, and those skipped because they already had an open request on the event.
#[derive(Debug, Serialize, ToSchema)]
pub struct Tier1GenerateResult {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

/// One person's availability response for an event (from the DCC thread 🟢/🟡/🔴 buttons). `roles`
/// carries the responder's assignable roles (e.g. `NTMO`) so the planner can read NOM vs shadow intent.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct EventAvailabilityBody {
    pub cid: i64,
    pub display_name: String,
    /// `available` | `partial` | `unavailable`.
    pub status: String,
    #[schema(value_type = Vec<String>)]
    pub roles: sqlx::types::Json<Vec<String>>,
    pub updated_at: DateTime<Utc>,
}

// --- integration / Discord ---

/// The bot relays an availability button press: which Discord user pressed which colour. The event
/// id travels in the path.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DiscordAvailabilityRequest {
    pub discord_user_id: String,
    /// `available` | `partial` | `unavailable`.
    pub status: String,
}

/// Outcome of an availability press, shaped for the bot's ephemeral reply. `ok=false` is a soft
/// refusal — `reason` is `unlinked` | `forbidden` | `invalid` (never a hard error, so the bot can
/// tell the user why).
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordAvailabilityResult {
    pub ok: bool,
    pub reason: Option<String>,
    pub display_name: Option<String>,
    pub status: Option<String>,
}

/// One outbound job handed to the bot on lease. `payload` carries everything the handler needs.
#[derive(Debug, Serialize, ToSchema)]
pub struct OutboundJobBody {
    pub id: String,
    pub job_type: String,
    #[schema(value_type = Object)]
    pub payload: Value,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
    pub attempt_count: i32,
    pub created_at: DateTime<Utc>,
}

/// The bot's acknowledgement of a leased job. `result` records ids the backend must remember
/// (message/thread id); `error` explains a failure (triggers backoff + retry).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AckJobRequest {
    pub success: bool,
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A logical-name → Discord snowflake entry (channel or role).
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema, sqlx::FromRow)]
pub struct DiscordMapEntry {
    pub name: String,
    pub id: String,
}

/// One configured guild (DCC / VATUSA / …) + its logical-name maps.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordGuildConfigBody {
    /// Null until this guild's config row has been saved.
    pub id: Option<String>,
    pub name: String,
    pub guild_id: String,
    pub channels: Vec<DiscordMapEntry>,
    pub roles: Vec<DiscordMapEntry>,
    /// ARTCCs this guild serves — lets `channel_id`/`role_id` prefer this guild over another one
    /// defining the same logical name for a different facility (#194). Empty = no facility
    /// preference (only ever wins via the sort-order fallback).
    pub facilities: Vec<String>,
}

/// The whole Discord config: the configured guilds, plus a snapshot of the guilds the bot is in
/// (channels + roles) so the editor can offer dropdowns instead of hand-typed snowflakes.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordConfigBody {
    pub guilds: Vec<DiscordGuildConfigBody>,
    pub available: Vec<DiscordGuildSnapshotBody>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertDiscordConfigRequest {
    pub guilds: Vec<DiscordGuildConfigInput>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DiscordGuildConfigInput {
    pub name: String,
    pub guild_id: String,
    #[serde(default)]
    pub channels: Vec<DiscordMapEntry>,
    #[serde(default)]
    pub roles: Vec<DiscordMapEntry>,
    #[serde(default)]
    pub facilities: Vec<String>,
}

/// One channel in a guild snapshot (the real channels the bot sees).
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema, sqlx::FromRow)]
pub struct DiscordGuildChannel {
    pub id: String,
    pub name: String,
    /// `text` | `voice` | `category` | `forum` | `announcement` | `stage` | …
    pub kind: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub position: i32,
}

/// One role in a guild snapshot.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema, sqlx::FromRow)]
pub struct DiscordGuildRole {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub position: i32,
}

/// A snapshot of one guild the bot is in — used for the config dropdowns + the guild picker.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct DiscordGuildSnapshotBody {
    pub guild_id: String,
    pub name: String,
    pub channels: Vec<DiscordGuildChannel>,
    pub roles: Vec<DiscordGuildRole>,
}

/// The bot's push of every guild it's in (full replace of the snapshot).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PushGuildSnapshotRequest {
    pub guilds: Vec<DiscordGuildSnapshotBody>,
}

/// The event-thread message body template (placeholders substituted by the bot at render time).
#[derive(Debug, Serialize, ToSchema)]
pub struct EventThreadTemplateBody {
    pub body: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertEventThreadTemplateRequest {
    pub body: String,
}

/// The current user's Discord account link.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordLinkBody {
    pub linked: bool,
    /// The linked Discord user id (snowflake), when `linked`.
    pub discord_id: Option<String>,
    /// The linked Discord username/handle, when known.
    pub username: Option<String>,
}

/// What the bot needs to render the Discord claim time-selectors for a request: the event window +
/// pre-computed Zulu HHMM slot options, and the current slot fill.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordAceInfoBody {
    pub event_title: String,
    /// A human window label, e.g. `2300–0300z`.
    pub window_label: String,
    pub slots: i32,
    pub claims_count: i64,
    /// Zulu HHMM options spanning the event window (for the start/end dropdowns).
    pub time_options: Vec<String>,
}

/// What the bot needs for the "View structured" reply on a TMI post: the raw line (already what's
/// posted) plus its plain-English structured breakdown, when the TMI was entered via the structured
/// form (null for a raw-typed TMI).
#[derive(Debug, Serialize, ToSchema)]
pub struct DiscordTmiInfoBody {
    pub restriction: String,
    pub decoded: Option<String>,
}

/// Bot interaction callback: a Discord user submitted the claim modal on an ACE request. The backend
/// resolves the Discord id to the linked OIS user and claims a slot on their behalf. `start_hhmm` /
/// `end_hhmm` are the modal's raw Zulu times (e.g. "2330"); the backend parses them against the
/// event window (the bot has no per-message window state).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DiscordAceClaimRequest {
    pub discord_user_id: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub start_hhmm: Option<String>,
    #[serde(default)]
    pub end_hhmm: Option<String>,
}

// --- flow constrained areas (FCAs) ---

/// A Flow Constrained Area — a drawn polyline the metering engine sequences traffic against.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FcaBody {
    pub id: String,
    pub name: String,
    pub color: String,
    pub artcc: String,
    /// Polyline vertices as `[lat, lon]` pairs (>= 2).
    #[schema(value_type = Vec<Vec<f64>>)]
    pub points: sqlx::types::Json<Vec<[f64; 2]>>,
    pub dests: Vec<String>,
    pub origins: Vec<String>,
    pub fixes: Vec<String>,
    pub scope: Vec<String>,
    pub min_fl: Option<i32>,
    pub max_fl: Option<i32>,
    /// any | N | S | E | W
    pub dir: String,
    /// rate | mit
    pub mode: String,
    pub rate: i32,
    pub mit: i32,
    pub enabled: bool,
    /// Controller's manual crossing order (callsigns); empty = auto.
    pub manual_order: Vec<String>,
    pub manual_seq: bool,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
    /// Event this FCA belongs to (null = an ordinary shared FCA visible on every map).
    pub event_id: Option<i64>,
    /// Event FCA lifecycle: planned | published | archived (null for shared FCAs).
    pub event_status: Option<String>,
    /// Whether an event FCA auto-publishes 30 min before the event starts.
    pub auto_publish: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertFcaRequest {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub artcc: String,
    /// Polyline vertices as `[lat, lon]` pairs (>= 2).
    #[schema(value_type = Vec<Vec<f64>>)]
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub dests: Vec<String>,
    #[serde(default)]
    pub origins: Vec<String>,
    #[serde(default)]
    pub fixes: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub min_fl: Option<i32>,
    #[serde(default)]
    pub max_fl: Option<i32>,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub rate: Option<i32>,
    #[serde(default)]
    pub mit: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Toggle an event FCA's auto-publish flag (publish 30 min before the event starts).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetFcaAutoRequest {
    pub auto_publish: bool,
}

/// A named reference route on the flow map, defined by a filed-route string and resolved to a
/// track by the nav engine (kept fresh on every read). Shared; not tied to any aircraft.
#[derive(Debug, Serialize, ToSchema)]
pub struct RouteBody {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Owning ARTCC (e.g. `ZDC`), or null for a global route shown on every facility map.
    pub artcc: Option<String>,
    /// The filed-route string, e.g. `RBV Q430 BYRDD J48 MOL FLASK OZZZI2`.
    pub route: String,
    /// Optional departure airport ICAO (helps SID / preferred-route resolution).
    pub dep: String,
    /// Optional arrival airport ICAO (helps STAR resolution).
    pub arr: String,
    /// Resolved track vertices as `[lat, lon]` pairs.
    #[schema(value_type = Vec<Vec<f64>>)]
    pub points: Vec<[f64; 2]>,
    /// Named waypoints along the route (for optional per-route fix labels).
    pub waypoints: Vec<RouteWaypoint>,
    /// Route tokens the nav engine couldn't resolve (shown as a warning).
    pub unresolved: Vec<String>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertRouteRequest {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    /// Owning ARTCC (e.g. `ZDC`); null/blank = a global route. The server facility-scopes editing to it.
    #[serde(default)]
    pub artcc: Option<String>,
    /// The filed-route string to resolve, e.g. `RBV Q430 BYRDD J48 MOL FLASK OZZZI2`.
    pub route: String,
    #[serde(default)]
    pub dep: Option<String>,
    #[serde(default)]
    pub arr: Option<String>,
}

/// An aircraft whose filed route crosses an FCA, with its ETA to the crossing.
#[derive(Debug, Serialize, ToSchema)]
pub struct FcaFlight {
    pub callsign: String,
    pub dep: String,
    pub arr: String,
    pub aircraft_type: String,
    /// airborne | ground | proposed
    pub status: String,
    pub lat: f64,
    pub lon: f64,
    /// Where the route crosses the FCA line.
    pub cross_lat: f64,
    pub cross_lon: f64,
    /// The resolved route track (remaining route for airborne) as `[lat, lon]` pairs, for
    /// drawing the aircraft's full path across the FCA.
    #[schema(value_type = Vec<Vec<f64>>)]
    pub path: Vec<[f64; 2]>,
    /// Distance along the (remaining) route to the crossing, nm.
    pub distance_nm: i64,
    /// Unmetered ETA to the crossing.
    pub eta: Option<DateTime<Utc>>,
    /// Metered crossing time (after sequencing).
    pub cross_time: Option<DateTime<Utc>>,
    pub delay_min: i64,
    /// Metered delay in seconds (metered crossing − raw ETA), for a precise `+MM:SS` readout.
    pub delay_sec: i64,
    /// The delay expressed as extra track distance (nm) at the predicted crossing speed — how much
    /// further back in the flow this aircraft must effectively be to hold separation.
    pub delay_nm: i64,
    /// 1-based sequence in the metered order.
    pub seq: i64,
    /// Release / wheels-up time when a CFR has been issued.
    pub edct: Option<DateTime<Utc>>,
    /// True when this aircraft has a frozen (issued) CFR release.
    pub released: bool,
    pub groundspeed: i64,
    pub altitude: i64,
    pub heading: i64,
    /// Debug detail for the ETA/metering model — only present when the request asks for it
    /// (`?debug=1`) and the user has debug mode on. Omitted from normal payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<FcaFlightDebug>,
}

/// Per-flight debug info surfaced by the client's debug mode: which performance profile drove the
/// ETA, the speeds/wind used, and any filed-route tokens that failed to resolve (issue #37).
#[derive(Debug, Serialize, ToSchema)]
pub struct FcaFlightDebug {
    /// The resolved aircraft performance profile: "type:C172", "wake:H", or "default".
    pub profile: String,
    /// Cruise TAS (kt) used for the ETA, after the profile cap.
    pub cruise_tas: i64,
    /// Filed cruise altitude (ft) used.
    pub cruise_alt: i64,
    /// Mean route headwind (kt) applied (+ head / − tail); null = still air.
    pub headwind: Option<i64>,
    /// Filed-route tokens that didn't resolve to a nav fix/navaid/airway/procedure — a likely
    /// source of ETA/track error.
    pub unresolved: Vec<String>,
}

/// Issue a CFR release for a crossing aircraft. `ready` (HHMMz) pins a wheels-up time;
/// omitted = release at the earliest metered slot.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReleaseRequest {
    #[serde(default)]
    pub ready: Option<String>,
}

/// Replace an FCA's manual crossing order (callsigns, in sequence).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderRequest {
    pub order: Vec<String>,
}

/// Route-fix tokens that don't resolve to a known nav fix/navaid/airway/procedure — likely typos in
/// an FCA's route-fix filter.
#[derive(Debug, Serialize, ToSchema)]
pub struct FixValidationBody {
    pub unknown: Vec<String>,
}

// --- IDST (Integrated Departure Scheduling): FCA-metered ground departures across a scope ---

/// One FCA-metered ground departure in the IDST console. One row per metering FCA — a flight metered
/// by several FCAs appears once per FCA, each with its own release.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdstFlight {
    pub callsign: String,
    pub dep: String,
    pub arr: String,
    pub aircraft_type: String,
    /// `ground` | `proposed`.
    pub status: String,
    /// The FCA metering this flight (the "program").
    pub fca_id: String,
    pub fca_name: String,
    /// 1-based sequence in that FCA's metered order.
    pub seq: i64,
    pub delay_min: i64,
    /// Metered crossing time (CTA) at the FCA line.
    pub cross_time: Option<DateTime<Utc>>,
    /// Frozen wheels-up (EDCT) once released; null while unscheduled.
    pub edct: Option<DateTime<Utc>>,
    pub released: bool,
}

/// The IDST board: FCA-metered ground departures in scope, split by release state.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdstResponse {
    pub unscheduled: Vec<IdstFlight>,
    pub released: Vec<IdstFlight>,
    pub metered_count: i64,
    pub as_of: DateTime<Utc>,
}

/// An aircraft's filed route resolved to lat/lon anchors, for plotting + a detail popup.
#[derive(Debug, Serialize, ToSchema)]
pub struct AircraftRoute {
    pub callsign: String,
    pub aircraft_type: String,
    pub dep: String,
    pub arr: String,
    pub altitude: i64,
    pub groundspeed: i64,
    /// Raw filed route string.
    pub route: String,
    /// Resolved route anchors as `[lat, lon]` pairs (the drawn track).
    #[schema(value_type = Vec<Vec<f64>>)]
    pub points: Vec<[f64; 2]>,
    /// Filed tokens that couldn't be resolved to a coordinate.
    pub unresolved: Vec<String>,
    /// Named anchors along the full filed route, for on-map labels.
    pub waypoints: Vec<RouteWaypoint>,
    /// FAA NASR cycle date backing the resolution (e.g. `2026-07-09`).
    pub nav_cycle: String,
}

/// A named point along a resolved route.
#[derive(Debug, Serialize, ToSchema)]
pub struct RouteWaypoint {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// One flight to resolve a filed route for (batch route resolution for the replay map).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveRouteRequest {
    pub callsign: String,
    #[serde(default)]
    pub dep: String,
    #[serde(default)]
    pub arr: String,
    #[serde(default)]
    pub route: String,
}

/// A filed route resolved to a drawable polyline.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolvedRoute {
    pub callsign: String,
    /// Route polyline as `[lat, lon]` pairs.
    pub points: Vec<[f64; 2]>,
    pub waypoints: Vec<RouteWaypoint>,
    /// Filed-route tokens the nav engine couldn't resolve.
    pub unresolved: Vec<String>,
}

/// Health of the runtime nav + winds data backing route/ETA prediction.
#[derive(Debug, Serialize, ToSchema)]
pub struct DataStatus {
    /// FAA NASR cycle date currently loaded (e.g. `2026-08-06`).
    pub nav_cycle: String,
    /// Provenance of the loaded nav data (e.g. `runtime fetch (faa)` or the bundle).
    pub nav_source: String,
    pub fixes: usize,
    pub navaids: usize,
    pub airways: usize,
    pub procedures: usize,
    /// Last successful runtime nav fetch (null = still on the compile-time bundle seed).
    pub nav_refreshed: Option<DateTime<Utc>>,
    pub winds_stations: usize,
    /// Last successful winds fetch (null = still air, not yet fetched).
    pub winds_refreshed: Option<DateTime<Utc>>,
}

/// A lightweight live-traffic record for plotting on the FCA + facility maps. Carries just enough of
/// the flight plan for the facility map's client-side color rules (arrival gate/STAR, wake, rules,
/// filed altitude) without shipping the full route.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrafficAircraft {
    pub callsign: String,
    pub lat: f64,
    pub lon: f64,
    pub heading: i64,
    pub gs: i64,
    pub alt: i64,
    pub dep: String,
    pub arr: String,
    pub actype: String,
    /// Arrival gate / STAR (base name, revision stripped) derived from the filed route; null if none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub star: Option<String>,
    /// Wake category (`L`/`M`/`H`/`J`), empty if unfiled.
    pub wake: String,
    /// Flight rules as filed (`I`/`V`/…), empty if no flight plan.
    pub flight_rules: String,
    /// Filed cruise altitude in feet (0 if unfiled/unparseable).
    pub filed_alt: i32,
}

// --- Facility map (per-facility TMU map color rules) ---

/// One aircraft-coloring condition. `field` names the flight attribute, `op` the comparison, and
/// `values` its operand(s). Conditions within a rule are ANDed. Semantics live in the client rule
/// engine; the backend stores this opaquely.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuleCondition {
    /// `arr` | `dep` | `star` | `type` | `wake` | `rules` | `alt`.
    pub field: String,
    /// `eq` | `in` | `prefix` | `lt` | `gt` | `range`.
    pub op: String,
    pub values: Vec<String>,
}

/// One color rule: aircraft matching all `conditions` are painted `color`. Rules are evaluated in
/// order; the first enabled match wins.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ColorRule {
    pub id: String,
    pub label: String,
    /// Hex color (`#rrggbb`) from the shared palette.
    pub color: String,
    pub enabled: bool,
    pub conditions: Vec<RuleCondition>,
}

/// A facility's map color-rule configuration (one per ARTCC).
#[derive(Debug, Serialize, ToSchema)]
pub struct FacilityMapConfigBody {
    pub facility_id: String,
    pub rules: Vec<ColorRule>,
    /// Hex color for aircraft matching no rule; empty = the map's theme default.
    pub default_color: String,
    /// Whether the current caller may edit this facility's rules (scope-resolved; false when signed out).
    pub editable: bool,
}

/// Upsert body for a facility's map color rules.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertFacilityMapConfigRequest {
    pub rules: Vec<ColorRule>,
    #[serde(default)]
    pub default_color: String,
}

// --- Online ATC (the map "ATC" layer) ---

/// An ATC facility (ARTCC/center or TRACON/approach) and the airports it covers — the dashboard's
/// facility directory. Derived from VATSpy + the SimAware TRACON project (see `feed/facilities.rs`).
#[derive(Debug, Serialize, ToSchema)]
pub struct FlowFacility {
    pub id: String,
    /// "artcc" (center) | "tracon" (approach).
    pub kind: String,
    /// Display name, when known (null for now).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Member airport ICAOs.
    pub airports: Vec<String>,
}

/// Everything the ATC layer needs: airport ground stations (badges), TRACON areas, and
/// center positions. TRACON polygons are inlined (only the active ones); centers reference
/// an ARTCC id the client already has boundary geometry for.
#[derive(Debug, Serialize, ToSchema)]
pub struct AtcBoard {
    pub airports: Vec<AtcAirport>,
    pub tracons: Vec<AtcArea>,
    pub centers: Vec<AtcCenter>,
    pub as_of: DateTime<Utc>,
}

/// One staffed airport and its ground-level positions (DEL/GND/TWR/ATIS), for the badge stack.
#[derive(Debug, Serialize, ToSchema)]
pub struct AtcAirport {
    pub icao: String,
    pub lat: f64,
    pub lon: f64,
    pub positions: Vec<AtcPosition>,
}

/// A single ATC position. `kind` is one of DEL/GND/TWR/APP/CTR/FSS/ATIS.
#[derive(Debug, Serialize, ToSchema)]
pub struct AtcPosition {
    pub kind: String,
    pub callsign: String,
    pub frequency: String,
    /// The controller's name (empty for ATIS / when unknown).
    pub name: String,
    /// The controller's VATSIM rating id (0 when unknown).
    pub rating: i32,
    /// When the controller logged on (RFC3339; empty when unknown / ATIS).
    pub logon_time: String,
    /// ATIS broadcast letter, only for `kind == "ATIS"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atis_code: Option<String>,
}

/// A TRACON/approach area: the matched SimAware polygon (or a circle fallback) plus the
/// positions working it. `rings` are outer rings in `[lat, lon]`; empty when `circle` is set.
#[derive(Debug, Serialize, ToSchema)]
pub struct AtcArea {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Preferred label anchor `[lat, lon]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<[f64; 2]>,
    pub positions: Vec<AtcPosition>,
    #[schema(value_type = Vec<Vec<[f64; 2]>>)]
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Fallback center `[lat, lon]` for an approach with no matching polygon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circle: Option<[f64; 2]>,
}

/// A center (ARTCC) position. The client shades its own bundled ARTCC polygon by `id`.
#[derive(Debug, Serialize, ToSchema)]
pub struct AtcCenter {
    pub id: String,
    pub positions: Vec<AtcPosition>,
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

// --- api keys (user-owned personal access tokens) ---

/// One requested `(permission, scope)` grant on a key. `artcc_id = null` means national — allowed
/// only if the owner holds the permission nationally.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApiKeyPermissionInput {
    pub permission: String,
    #[serde(default)]
    pub artcc_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub permissions: Vec<ApiKeyPermissionInput>,
    /// Optional free-text reason recorded to the audit log.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetApiKeyPermissionsRequest {
    pub permissions: Vec<ApiKeyPermissionInput>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Optional audit reason on an admin revoke/disable/delete.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeApiKeyRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Clone, ToSchema)]
pub struct ApiKeyPermissionBody {
    pub permission: String,
    pub artcc_id: Option<String>,
}

/// A key as listed (never the secret). `permissions` is the granted subset; effective authority at
/// request time is this ∩ the owner's live access.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyBody {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub prefix: String,
    pub status: String,
    pub owner_cid: Option<i64>,
    pub owner_display_name: Option<String>,
    pub permissions: Vec<ApiKeyPermissionBody>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub last_used_ip: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Returned once on create/rotate — the plaintext `ois_pat_…` token is never stored or shown again.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyTokenBody {
    pub key: ApiKeyBody,
    pub token: String,
}

/// One permission the current user may delegate to a key, with the scope they can grant it at.
/// `national = true` means they can grant it nationally (and therefore at any ARTCC); otherwise
/// `artccs` lists the specific ARTCCs they may grant.
#[derive(Debug, Serialize, ToSchema)]
pub struct GrantablePermissionBody {
    pub permission: String,
    pub national: bool,
    pub artccs: Vec<String>,
}

// --- public advisories board (no-auth, read-only) ---
//
// Lean, pilot-facing projections of the active TMIs and FCAs. These intentionally
// omit internal/author fields (updated_by, manual ordering, draft rows) and are
// served without any permission — see handlers/public.rs.

/// An active inter-facility restriction (MIT / spacing) as pilots see it.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct PublicRestriction {
    pub id: String,
    /// Requesting facility (ARTCC/TRACON).
    pub requesting: String,
    /// Providing facility (ARTCC/TRACON).
    pub providing: String,
    /// The raw NTML line.
    pub restriction: String,
    /// Plain-English rendering for pilots (null for a raw-typed restriction).
    pub decoded: Option<String>,
    pub start_time: DateTime<Utc>,
    /// null = until further notice.
    pub stop_time: Option<DateTime<Utc>>,
}

/// An active ground stop.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct PublicGroundStop {
    pub id: String,
    pub airport: String,
    /// Space-separated ARTCC/FIR codes; empty = field-wide.
    pub scope: String,
    /// HHMM Zulu clock the stop runs until; null = until further notice.
    pub until: Option<String>,
}

/// An active Ground Delay Program.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct PublicGdp {
    pub id: String,
    pub airport: String,
    /// Airport Acceptance Rate the program meters to (arrivals/hour).
    pub aar: i32,
    /// Space-separated departure ARTCC codes in scope; empty = all departures.
    pub scope: String,
    /// HHMM Zulu window start.
    pub start_time: String,
    /// HHMM Zulu window end.
    pub end_time: String,
    pub max_enroute_min: Option<i32>,
    pub exempt_airborne: bool,
    /// Number of controlled (delayed) flights, from the frozen slots.
    pub controlled: i64,
    /// Average assigned delay across controlled flights (minutes).
    pub avg_delay_min: i64,
    /// Worst assigned delay (minutes).
    pub max_delay_min: i64,
    /// Live inbounds estimated to land within the next 60 min (feed-derived).
    pub demand_60min: i64,
    /// True when live demand exceeds the AAR.
    pub over_capacity: bool,
}

/// An active airport rate program (AAR + spacing).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct PublicProgram {
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
    /// Scheduled end; null = indefinite.
    pub active_until: Option<DateTime<Utc>>,
    /// Live inbounds estimated to land within the next 60 min (feed-derived).
    pub demand_60min: i64,
    /// True when live demand exceeds the AAR.
    pub over_capacity: bool,
}

// --- public per-flight advisory ("my flight" lookup) ---

/// A GDP affecting a looked-up flight (its arrival airport), with this flight's
/// frozen control times when it's a controlled slot.
#[derive(Debug, Serialize, ToSchema)]
pub struct FlightGdp {
    pub airport: String,
    pub aar: i32,
    pub start_time: String,
    pub end_time: String,
    /// True when this flight holds a frozen control slot (subject to an EDCT).
    pub controlled: bool,
    pub edct: Option<DateTime<Utc>>,
    pub cta: Option<DateTime<Utc>>,
    pub delay_min: i64,
}

/// A ground stop affecting a looked-up flight's arrival airport.
#[derive(Debug, Serialize, ToSchema)]
pub struct FlightGroundStop {
    pub airport: String,
    pub scope: String,
    pub until: Option<String>,
}

/// A rate program metering a looked-up flight into its arrival airport.
#[derive(Debug, Serialize, ToSchema)]
pub struct FlightProgram {
    pub airport: String,
    pub aar: i32,
    pub delay_min: i64,
    pub sta: Option<DateTime<Utc>>,
    pub cfr: Option<DateTime<Utc>>,
}

/// An FCA a looked-up flight crosses, with its metered crossing.
#[derive(Debug, Serialize, ToSchema)]
pub struct FlightFcaCrossing {
    pub fca_id: String,
    pub fca_name: String,
    pub color: String,
    pub cross_time: Option<DateTime<Utc>>,
    pub delay_min: i64,
    pub edct: Option<DateTime<Utc>>,
    pub seq: Option<i64>,
}

/// Everything currently affecting one flight (by callsign), for the public "my
/// flight" lookup and the FCA-map search. `found` is false when the callsign
/// isn't in the live feed.
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct FlightAdvisory {
    pub callsign: String,
    pub found: bool,
    pub dep: String,
    pub arr: String,
    pub aircraft_type: String,
    /// `airborne` | `ground`.
    pub status: String,
    pub altitude: i64,
    pub groundspeed: i64,
    pub lat: f64,
    pub lon: f64,
    pub heading: i64,
    pub gdp: Option<FlightGdp>,
    pub ground_stop: Option<FlightGroundStop>,
    pub rate_program: Option<FlightProgram>,
    pub fcas: Vec<FlightFcaCrossing>,
    /// The binding (worst) predicted delay across all applicable initiatives.
    pub total_delay_min: i64,
    /// The latest expect-departure-clearance time across ground programs, if any.
    pub edct: Option<DateTime<Utc>>,
}

/// The full public advisories board — every active initiative in one payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicBoard {
    pub ground_stops: Vec<PublicGroundStop>,
    pub gdps: Vec<PublicGdp>,
    pub restrictions: Vec<PublicRestriction>,
    pub programs: Vec<PublicProgram>,
    /// Server time this snapshot was taken (for the "updated" line).
    pub as_of: DateTime<Utc>,
}

// (The public FCA overview reuses the shared FCA map + the now-public
// GET /api/v1/flow/fcas, so no separate lean FCA projection is needed.)

// --- Dashboards (multiple named boards per user; see migration 0035) ---

/// A dashboard in the caller's library (list item — no data blob).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct DashboardSummary {
    pub id: String,
    pub name: String,
    pub collection_id: Option<String>,
    pub share_slug: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A full dashboard, including its opaque client-owned DashboardState `data`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct DashboardBody {
    pub id: String,
    pub name: String,
    pub collection_id: Option<String>,
    pub share_slug: Option<String>,
    #[schema(value_type = Object)]
    pub data: sqlx::types::Json<Value>,
    pub updated_at: DateTime<Utc>,
}

/// A shared dashboard as any signed-in viewer sees it (read-only, no ids).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct SharedDashboardBody {
    pub name: String,
    pub owner: String,
    #[schema(value_type = Object)]
    pub data: sqlx::types::Json<Value>,
}

/// A dashboard collection (folder).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct DashboardCollection {
    pub id: String,
    pub name: String,
}

/// The dashboards library: the caller's boards + collections.
#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardLibrary {
    pub dashboards: Vec<DashboardSummary>,
    pub collections: Vec<DashboardCollection>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDashboardRequest {
    pub name: String,
    #[schema(value_type = Object)]
    pub data: Option<Value>,
    pub collection_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDashboardRequest {
    pub name: Option<String>,
    #[schema(value_type = Object)]
    pub data: Option<Value>,
    pub collection_id: Option<String>,
}

/// A minimal `{ name }` body for creating/renaming collections + share responses.
#[derive(Debug, Deserialize, ToSchema)]
pub struct NameRequest {
    pub name: String,
}

/// The slug returned when a board is shared.
#[derive(Debug, Serialize, ToSchema)]
pub struct ShareResponse {
    pub share_slug: String,
}

/// The new board id returned when copying a shared board.
#[derive(Debug, Serialize, ToSchema)]
pub struct CopyResponse {
    pub id: String,
}

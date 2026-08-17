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

/// One facility's ACE staffing request for an event (positions wanted vs signed up).
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct StaffingRequestBody {
    /// ARTCC id (e.g. ZTL).
    pub facility: String,
    pub positions_requested: i32,
    pub positions_filled: i32,
    /// open | met | closed
    pub status: String,
    pub notes: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertStaffingRequest {
    pub positions_requested: i32,
    pub positions_filled: i32,
    /// open | met | closed
    pub status: String,
    #[serde(default)]
    pub notes: Option<String>,
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
    /// draft | activated
    pub status: String,
    pub activated_at: Option<DateTime<Utc>>,
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

/// A named reference route on the flow map, defined by a filed-route string and resolved to a
/// track by the nav engine (kept fresh on every read). Shared; not tied to any aircraft.
#[derive(Debug, Serialize, ToSchema)]
pub struct RouteBody {
    pub id: String,
    pub name: String,
    pub color: String,
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
    /// 1-based sequence in the metered order.
    pub seq: i64,
    /// Release / wheels-up time when a CFR has been issued.
    pub edct: Option<DateTime<Utc>>,
    /// True when this aircraft has a frozen (issued) CFR release.
    pub released: bool,
    pub groundspeed: i64,
    pub altitude: i64,
    pub heading: i64,
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

/// A lightweight live-traffic record for plotting on the FCA map.
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
}

// --- Online ATC (the map "ATC" layer) ---

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
    pub restriction: String,
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

# API permissions

Every OIS API endpoint and the permission it requires. Use it to ask for exactly what an integration
needs: grant an [API key](/reference/api-keys) or a service account those permissions and no more.

How to read it:

- A permission is granted nationally or scoped to ARTCCs (see [Roles & permissions](/reference/permissions)).
  Some endpoints also check the scope against the facility a request touches. The table shows the
  permission, not the scope rule.
- **"— (no permission marker)"** means the endpoint isn't gated by a specific permission. It may still
  need a signed-in caller, or decide access itself. Check the endpoint in [Swagger UI](/reference/api#swagger-ui).
- An API key never holds an `api_keys.*` permission, whatever its owner has.

::: info Generated, not written by hand
This table is generated from the API's source. A test fails CI whenever it falls out of date, so it
always matches the running API.
:::

<!-- generated:permission-map:begin -->
| Endpoint | Method | Requires |
| --- | --- | --- |
| `/api/v1/access/catalog` | GET | `access.catalog.read` |
| `/api/v1/access/self` | GET | `access.self.read` |
| `/api/v1/admin/api-keys` | GET | `api_keys.key.read` |
| `/api/v1/admin/api-keys/{id}` | DELETE | `api_keys.key.delete` |
| `/api/v1/admin/api-keys/{id}/disable` | POST | `api_keys.key.delete` |
| `/api/v1/admin/audit` | GET | `audit.logs.read` |
| `/api/v1/admin/diagnostics` | GET | `diagnostics.reports.read` |
| `/api/v1/admin/diagnostics/{id}` | DELETE | `diagnostics.reports.delete` |
| `/api/v1/admin/diagnostics/{id}` | GET | `diagnostics.reports.read` |
| `/api/v1/admin/diagnostics/{id}/logs` | GET | `diagnostics.reports.read` |
| `/api/v1/admin/groups` | GET | `access.groups.read` |
| `/api/v1/admin/groups` | POST | `access.groups.update` |
| `/api/v1/admin/groups/{name}` | DELETE | `access.groups.update` |
| `/api/v1/admin/groups/{name}` | PUT | `access.groups.update` |
| `/api/v1/admin/groups/{name}/members` | DELETE | `access.groups.update` |
| `/api/v1/admin/groups/{name}/members` | GET | `access.groups.read` |
| `/api/v1/admin/groups/{name}/members` | POST | `access.groups.update` |
| `/api/v1/admin/jobs` | GET | `system.jobs.read` |
| `/api/v1/admin/jobs/{name}/run` | POST | `system.jobs.update` |
| `/api/v1/admin/service-accounts` | GET | `service_accounts.read` |
| `/api/v1/admin/service-accounts` | POST | `service_accounts.create` |
| `/api/v1/admin/service-accounts/grantable-permissions` | GET | `service_accounts.update` |
| `/api/v1/admin/service-accounts/roles` | GET | `service_accounts.update` |
| `/api/v1/admin/service-accounts/{id}/disable` | POST | `service_accounts.delete` |
| `/api/v1/admin/service-accounts/{id}/permissions` | PUT | `service_accounts.update` |
| `/api/v1/admin/service-accounts/{id}/roles` | PUT | `service_accounts.update` |
| `/api/v1/admin/service-accounts/{id}/rotate` | POST | `service_accounts.update` |
| `/api/v1/admin/summary` | GET | — (no permission marker) |
| `/api/v1/admin/users` | GET | `access.users.read` |
| `/api/v1/admin/users/{cid}/access` | GET | `access.users.read` |
| `/api/v1/admin/users/{cid}/access` | POST | `access.users.update` |
| `/api/v1/admin/vatusa-role-mappings` | GET | `access.groups.read` |
| `/api/v1/admin/vatusa-role-mappings` | POST | `access.groups.update` |
| `/api/v1/admin/vatusa-role-mappings/{id}` | DELETE | `access.groups.update` |
| `/api/v1/airport-configs` | GET | `events.plan.read` |
| `/api/v1/airport-configs/{icao}` | GET | `events.plan.read` |
| `/api/v1/airport-configs/{icao}` | POST | `events.config.update` |
| `/api/v1/airport-configs/{icao}/{id}` | DELETE | `events.config.update` |
| `/api/v1/airport-configs/{icao}/{id}` | PUT | `events.config.update` |
| `/api/v1/airports/{icao}/gates` | POST | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/gates/{id}` | DELETE | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/gates/{id}` | PUT | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/ramp-areas` | POST | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/ramp-areas/{id}` | DELETE | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/ramp-areas/{id}` | PUT | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/runways` | POST | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/runways/{id}` | DELETE | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/runways/{id}` | PUT | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/surface` | GET | `events.plan.read` |
| `/api/v1/airports/{icao}/surface/repull-faa` | POST | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/taxiways` | POST | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/taxiways/{id}` | DELETE | `flow.surface_data.update` |
| `/api/v1/airports/{icao}/taxiways/{id}` | PUT | `flow.surface_data.update` |
| `/api/v1/api-keys` | GET | `api_keys.key.create` |
| `/api/v1/api-keys` | POST | `api_keys.key.create` |
| `/api/v1/api-keys/grantable-permissions` | GET | `api_keys.key.create` |
| `/api/v1/api-keys/{id}` | DELETE | `api_keys.key.create` |
| `/api/v1/api-keys/{id}` | GET | `api_keys.key.create` |
| `/api/v1/api-keys/{id}/audit` | GET | — (no permission marker) |
| `/api/v1/api-keys/{id}/disable` | POST | `api_keys.key.create` |
| `/api/v1/api-keys/{id}/permissions` | PUT | `api_keys.key.create` |
| `/api/v1/api-keys/{id}/rotate` | POST | `api_keys.key.create` |
| `/api/v1/auth/desktop/exchange` | POST | — (no permission marker) |
| `/api/v1/auth/desktop/refresh` | POST | — (no permission marker) |
| `/api/v1/auth/logout` | POST | `auth.sessions.delete` |
| `/api/v1/auth/vatsim/callback` | GET | — (no permission marker) |
| `/api/v1/auth/vatsim/login` | GET | — (no permission marker) |
| `/api/v1/dashboard-collections` | POST | `auth.profile.read` |
| `/api/v1/dashboard-collections/{id}` | DELETE | `auth.profile.read` |
| `/api/v1/dashboard-collections/{id}` | PUT | `auth.profile.read` |
| `/api/v1/dashboards` | GET | `auth.profile.read` |
| `/api/v1/dashboards` | POST | `auth.profile.read` |
| `/api/v1/dashboards/shared/{slug}` | GET | `auth.profile.read` |
| `/api/v1/dashboards/shared/{slug}/copy` | POST | `auth.profile.read` |
| `/api/v1/dashboards/{id}` | DELETE | `auth.profile.read` |
| `/api/v1/dashboards/{id}` | GET | `auth.profile.read` |
| `/api/v1/dashboards/{id}` | PUT | `auth.profile.read` |
| `/api/v1/dashboards/{id}/share` | DELETE | `auth.profile.read` |
| `/api/v1/dashboards/{id}/share` | POST | `auth.profile.read` |
| `/api/v1/events` | GET | `events.plan.read` |
| `/api/v1/events/{id}` | GET | `events.plan.read` |
| `/api/v1/events/{id}/ace` | GET | `events.plan.read` |
| `/api/v1/events/{id}/ace` | POST | `ace.requests.create` |
| `/api/v1/events/{id}/ace/{req}` | DELETE | `ace.requests.decide` |
| `/api/v1/events/{id}/ace/{req}/claim` | DELETE | `ace.requests.claim` |
| `/api/v1/events/{id}/ace/{req}/claim` | POST | `ace.requests.claim` |
| `/api/v1/events/{id}/ace/{req}/decide` | POST | `ace.requests.decide` |
| `/api/v1/events/{id}/availability` | GET | `events.plan.read` |
| `/api/v1/events/{id}/banner` | GET | `events.plan.read` |
| `/api/v1/events/{id}/capture` | GET | `events.plan.read` |
| `/api/v1/events/{id}/capture` | PUT | `stats.capture.update` |
| `/api/v1/events/{id}/dcc` | GET | `events.plan.read` |
| `/api/v1/events/{id}/dcc` | PUT | `events.plan.update` |
| `/api/v1/events/{id}/debrief` | GET | `events.plan.read` |
| `/api/v1/events/{id}/debrief` | PUT | `events.debrief.create` |
| `/api/v1/events/{id}/discord/publish` | POST | `events.discord.publish` |
| `/api/v1/events/{id}/facilities` | GET | `events.plan.read` |
| `/api/v1/events/{id}/facilities/tier1` | POST | `events.support.update` |
| `/api/v1/events/{id}/facilities/{facility}` | DELETE | `events.support.update` |
| `/api/v1/events/{id}/facilities/{facility}` | PUT | `events.support.update` |
| `/api/v1/events/{id}/fcas` | GET | `events.plan.read` |
| `/api/v1/events/{id}/fcas` | POST | `events.plan.update` |
| `/api/v1/events/{id}/fcas/{fca_id}` | DELETE | `events.plan.update` |
| `/api/v1/events/{id}/fcas/{fca_id}` | PUT | `events.plan.update` |
| `/api/v1/events/{id}/fcas/{fca_id}/archive` | POST | `events.plan.update` |
| `/api/v1/events/{id}/fcas/{fca_id}/auto` | PUT | `events.plan.update` |
| `/api/v1/events/{id}/fcas/{fca_id}/publish` | POST | `events.plan.update` |
| `/api/v1/events/{id}/packages` | GET | `events.plan.read` |
| `/api/v1/events/{id}/packages` | POST | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}` | DELETE | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}/activate` | POST | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}/auto` | PUT | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}/deactivate` | POST | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}/items` | POST | `events.plan.update` |
| `/api/v1/events/{id}/packages/{package_id}/items/{item_id}` | DELETE | `events.plan.update` |
| `/api/v1/events/{id}/rates` | GET | `events.plan.read` |
| `/api/v1/events/{id}/rates/{icao}` | DELETE | `events.rate.update` |
| `/api/v1/events/{id}/rates/{icao}` | PUT | `events.rate.update` |
| `/api/v1/events/{id}/stats` | GET | `events.plan.read` |
| `/api/v1/facilities` | GET | — (no permission marker) |
| `/api/v1/facilities/{facility_id}/documents` | GET | `facilities.docs.read` |
| `/api/v1/facilities/{facility_id}/documents` | POST | `facilities.docs.update` |
| `/api/v1/facilities/{facility_id}/documents/{id}` | DELETE | `facilities.docs.update` |
| `/api/v1/facilities/{facility_id}/documents/{id}` | PUT | `facilities.docs.update` |
| `/api/v1/facilities/{id}` | GET | — (no permission marker) |
| `/api/v1/facility-map/{id}/config` | GET | — (no permission marker) |
| `/api/v1/facility-map/{id}/config` | PUT | `flow.facility_map.update` |
| `/api/v1/feed/status` | GET | `tmu.program.read` |
| `/api/v1/flow/aircraft-profiles` | GET | `flow.aircraft_profiles.read` |
| `/api/v1/flow/aircraft-profiles/{kind}/{key}` | DELETE | `flow.aircraft_profiles.update` |
| `/api/v1/flow/aircraft-profiles/{kind}/{key}` | PUT | `flow.aircraft_profiles.update` |
| `/api/v1/flow/aircraft/{callsign}/route` | GET | — (no permission marker) |
| `/api/v1/flow/atc` | GET | — (no permission marker) |
| `/api/v1/flow/counts` | GET | — (no permission marker) |
| `/api/v1/flow/data-refresh` | POST | `flow.fca.update` |
| `/api/v1/flow/data-status` | GET | — (no permission marker) |
| `/api/v1/flow/facilities` | GET | — (no permission marker) |
| `/api/v1/flow/fcas` | GET | — (no permission marker) |
| `/api/v1/flow/fcas` | POST | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}` | DELETE | `flow.fca.delete` |
| `/api/v1/flow/fcas/{id}` | PUT | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/exclusions` | GET | `flow.fca.read` |
| `/api/v1/flow/fcas/{id}/exclusions/{callsign}` | DELETE | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/exclusions/{callsign}` | POST | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/order` | PUT | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/release/{callsign}` | DELETE | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/release/{callsign}` | POST | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/swap` | POST | `flow.fca.update` |
| `/api/v1/flow/fcas/{id}/traffic` | GET | — (no permission marker) |
| `/api/v1/flow/idst` | GET | `flow.fca.read` |
| `/api/v1/flow/resolve-routes` | POST | `stats.data.read` |
| `/api/v1/flow/route-coverage` | GET | — (no permission marker) |
| `/api/v1/flow/routes` | GET | — (no permission marker) |
| `/api/v1/flow/routes` | POST | `flow.route.update` |
| `/api/v1/flow/routes/{id}` | DELETE | `flow.route.delete` |
| `/api/v1/flow/routes/{id}` | PUT | `flow.route.update` |
| `/api/v1/flow/runway/{icao}` | GET | `flow.runway.read` |
| `/api/v1/flow/runway/{icao}` | PUT | `flow.runway.update` |
| `/api/v1/flow/runway/{icao}/configs` | GET | `flow.runway.read` |
| `/api/v1/flow/runway/{icao}/configs/{name}` | DELETE | `flow.runway.update` |
| `/api/v1/flow/runway/{icao}/configs/{name}` | PUT | `flow.runway.update` |
| `/api/v1/flow/traffic` | GET | — (no permission marker) |
| `/api/v1/flow/traffic/projected` | GET | — (no permission marker) |
| `/api/v1/flow/validate-fixes` | GET | `flow.fca.read` |
| `/api/v1/forecast/{icao}` | GET | `events.plan.read` |
| `/api/v1/integration/discord` | GET | `discord.config.read` |
| `/api/v1/integration/discord` | PUT | `discord.config.update` |
| `/api/v1/integration/discord/ace/{id}` | GET | `integration.jobs.update` |
| `/api/v1/integration/discord/ace/{id}/claim` | POST | `integration.jobs.update` |
| `/api/v1/integration/discord/advisory/{id}` | GET | `integration.jobs.update` |
| `/api/v1/integration/discord/availability/{id}` | POST | `integration.jobs.update` |
| `/api/v1/integration/discord/guilds/snapshot` | POST | `integration.jobs.update` |
| `/api/v1/integration/discord/refresh` | POST | `discord.config.update` |
| `/api/v1/integration/discord/thread-template` | GET | `discord.config.read` |
| `/api/v1/integration/discord/thread-template` | PUT | `discord.config.update` |
| `/api/v1/integration/discord/tmi/{id}` | GET | `integration.jobs.update` |
| `/api/v1/integration/jobs/lease` | POST | `integration.jobs.update` |
| `/api/v1/integration/jobs/{id}/ack` | POST | `integration.jobs.update` |
| `/api/v1/me` | GET | `auth.profile.read` |
| `/api/v1/me/ace-claims` | GET | `ace.requests.claim` |
| `/api/v1/me/discord` | GET | — (no permission marker) |
| `/api/v1/me/flight` | GET | — (no permission marker) |
| `/api/v1/me/preferences/{namespace}` | GET | `auth.profile.read` |
| `/api/v1/me/preferences/{namespace}` | PUT | `auth.profile.read` |
| `/api/v1/public/airports/{icao}` | GET | — (no permission marker) |
| `/api/v1/public/board` | GET | — (no permission marker) |
| `/api/v1/public/desktop/download/{platform}` | GET | — (no permission marker) |
| `/api/v1/public/flight/{callsign}` | GET | — (no permission marker) |
| `/api/v1/stats/airports/top` | GET | `stats.data.read` |
| `/api/v1/stats/airports/{icao}` | GET | `stats.data.read` |
| `/api/v1/stats/airports/{icao}/movements` | GET | `stats.data.read` |
| `/api/v1/stats/captures` | GET | `stats.data.read` |
| `/api/v1/stats/captures` | POST | `stats.capture.update` |
| `/api/v1/stats/captures/{id}` | DELETE | `stats.capture.delete` |
| `/api/v1/stats/captures/{id}/replay` | GET | `stats.data.read` |
| `/api/v1/stats/delays` | GET | `stats.data.read` |
| `/api/v1/stats/flights/{id}` | GET | `stats.data.read` |
| `/api/v1/stats/flights/{id}/track` | GET | `stats.data.read` |
| `/api/v1/stats/hist/atc` | GET | `stats.data.read` |
| `/api/v1/stats/hist/departures/{dep}` | GET | `stats.data.read` |
| `/api/v1/stats/hist/fcas` | GET | `stats.data.read` |
| `/api/v1/stats/hist/flow/{icao}` | GET | `stats.data.read` |
| `/api/v1/stats/hist/gdps` | GET | `stats.data.read` |
| `/api/v1/stats/hist/ground-stops` | GET | `stats.data.read` |
| `/api/v1/stats/hist/runway/{icao}` | GET | `stats.data.read` |
| `/api/v1/stats/hist/taxi/{icao}` | GET | `stats.data.read` |
| `/api/v1/stats/hist/tmis` | GET | `stats.data.read` |
| `/api/v1/stats/hist/traffic` | GET | `stats.data.read` |
| `/api/v1/stats/members/{cid}/flights` | GET | `stats.data.read` |
| `/api/v1/stats/network/history` | GET | `stats.data.read` |
| `/api/v1/stats/replay` | GET | `stats.data.read` |
| `/api/v1/stats/replay/positions` | GET | `stats.data.read` |
| `/api/v1/stats/storage-forecast` | GET | `system.jobs.read` |
| `/api/v1/stats/taxi/estimates` | GET | `stats.data.read` |
| `/api/v1/stats/taxi/observations` | GET | `stats.data.read` |
| `/api/v1/tmu/advisories` | GET | `tmu.adv.read` |
| `/api/v1/tmu/advisories` | POST | `tmu.adv.create` |
| `/api/v1/tmu/advisories/{id}` | DELETE | `tmu.adv.update` |
| `/api/v1/tmu/advisories/{id}` | GET | `tmu.adv.read` |
| `/api/v1/tmu/advisories/{id}` | PATCH | `tmu.adv.update` |
| `/api/v1/tmu/advisories/{id}/cancel` | POST | `tmu.adv.publish` |
| `/api/v1/tmu/advisories/{id}/publish` | POST | `tmu.adv.publish` |
| `/api/v1/tmu/cfr` | POST | `tmu.cfr.assign` |
| `/api/v1/tmu/cfr/{callsign}` | DELETE | `tmu.cfr.assign` |
| `/api/v1/tmu/demand` | GET | `tmu.program.read` |
| `/api/v1/tmu/departures/{dep}` | GET | `tmu.program.read` |
| `/api/v1/tmu/flow/{icao}` | GET | `tmu.program.read` |
| `/api/v1/tmu/flow/{icao}/aadc` | GET | `tmu.program.read` |
| `/api/v1/tmu/gdp` | GET | `tmu.gdp.read` |
| `/api/v1/tmu/gdp` | POST | `tmu.gdp.create` |
| `/api/v1/tmu/gdp/{id}` | DELETE | `tmu.gdp.delete` |
| `/api/v1/tmu/gdp/{id}` | PUT | `tmu.gdp.create` |
| `/api/v1/tmu/gdp/{id}/board` | GET | `tmu.gdp.read` |
| `/api/v1/tmu/gdp/{id}/cancel` | POST | `tmu.gdp.publish` |
| `/api/v1/tmu/gdp/{id}/compress` | POST | `tmu.gdp.publish` |
| `/api/v1/tmu/gdp/{id}/publish` | POST | `tmu.gdp.publish` |
| `/api/v1/tmu/gdp/{id}/slots/{callsign}` | DELETE | `tmu.gdp.publish` |
| `/api/v1/tmu/gdp/{id}/slots/{callsign}` | POST | `tmu.gdp.publish` |
| `/api/v1/tmu/ground-stops` | GET | `tmu.groundstop.read` |
| `/api/v1/tmu/ground-stops` | POST | `tmu.groundstop.create` |
| `/api/v1/tmu/ground-stops/{id}` | DELETE | `tmu.groundstop.delete` |
| `/api/v1/tmu/ground-stops/{id}/cancel` | POST | `tmu.groundstop.publish` |
| `/api/v1/tmu/ground-stops/{id}/publish` | POST | `tmu.groundstop.publish` |
| `/api/v1/tmu/programs` | GET | `tmu.program.read` |
| `/api/v1/tmu/programs/{icao}` | DELETE | `tmu.program.delete` |
| `/api/v1/tmu/programs/{icao}` | PUT | `tmu.program.update` |
| `/api/v1/tmu/taxi/{icao}` | GET | `tmu.program.read` |
| `/api/v1/tmu/tmis` | GET | `tmu.tmi.read` |
| `/api/v1/tmu/tmis` | POST | `tmu.tmi.create` |
| `/api/v1/tmu/tmis/{id}` | DELETE | `tmu.tmi.delete` |
| `/api/v1/tmu/tmis/{id}` | PATCH | `tmu.tmi.update` |
| `/api/v1/tmu/tmis/{id}/cancel` | POST | `tmu.tmi.publish` |
| `/api/v1/tmu/tmis/{id}/publish` | POST | `tmu.tmi.publish` |
| `/api/v1/users` | GET | `users.directory.read` |
| `/health` | GET | — (no permission marker) |
<!-- generated:permission-map:end -->

---
paths:
  - "backend/src/feed/**"
  - "backend/src/handlers/{flow,feed,gdp,runway,sector_demand,sector_demand_cache,flight_exclusions,aircraft_profiles,airport_configs,sector_limits,sector_consolidations,tmu,restriction_artcc,taxi_insights}.rs"
  - "backend/src/handlers/{flow_event_fca_visibility,sector_demand,sector_limits,sector_consolidations}_tests.rs"
  - "backend/src/repos/{flow,gdp,tmu,runway,aircraft_profiles,flight_exclusions,sector_limits,sector_consolidations,departure_runway,taxi_insights,airport_configs}.rs"
  - "backend/src/{tmi,advisory,state}.rs"
  - "fixtures/**"
  - "backend/src/auth/**"
  - "backend/src/scope_test_support.rs"
  - "backend/src/repos/{access,api_keys,service_accounts}.rs"
  - "backend/src/handlers/{access,access_reset,api_keys,service_accounts,auth}.rs"
  - "backend/src/handlers/{machine_actor,actor_ratchet,auth_annotation,service_account_grant}_tests.rs"
  - "crates/ois-core/src/**"
---
# Domain Lessons (not loaded at startup; read in full before changing or reviewing a feed, trajectory, metering, TMI or permission rule)

What the trajectory, metering, feed and permission rules got wrong, the rule that stops it recurring,
and the proof each fix holds: a commit and the test that pins it. Read before changing any of them.
Append an entry whenever a domain defect is resolved; an entry without a proof is a hypothesis, not a
lesson.

Every rule below holds for each trajectory caller and for a national-scope and a facility-scope
principal (`feed-and-trajectory.md` → "The paths every change must exercise").

---

## A workaround on one reader leaves the other readers wrong

The default profile's descent schedule put ~519 kt at cruise altitude while cruise was 440 kt, so the
cruise leg ramped between them. #355 capped `ground_speed_at` and deliberately not `time_between`, so
every enroute ETA stayed optimistic: a 1,200 nm leg implied 466.5 kt, an ETA 12.5 min early (#361).
Fix the samples every reader integrates, then delete the cap. Proof: commit 41b42790;
`the_cruise_leg_is_flat_at_cruise_speed` and `the_profile_never_exceeds_cruise_speed`
(`feed/trajectory.rs`).

## Size a spacing gap with the speed at the fix, not cruise

FCA metering sized MIT gaps with cruise groundspeed, but a metering fix such as CAMRN is crossed on
descent at ~280 kt, so a 20 MIT flow landed departures ~12 nm apart (#355). `eta_along_route` already
built the profile and discarded the speed. Proof: commits a3dec589, 80d643e2;
`eta_along_route_reports_the_descent_speed_at_a_low_crossing_fix` (`feed/predict.rs`) and
`mit_cross_speed_wiring_tests` (`handlers/flow.rs`), which covers an airborne and a ground pilot
after a fixture with no prefile let a revert of the prefile site (removed in #834) pass the suite.

## Space a release against where the others will actually cross

`mark_release` spaced a departure behind airborne arrivals' raw ETAs, but the ladder meters those
arrivals later when they owe spacing. With 120 s separation the release landed 60 s behind an
arrival's real crossing (#356). Meter the current sequence first, with the ladder's own
`manual_order`, and add `RELEASE_MARGIN_MS` on the RDY path only. Proof: commit 939c1c29;
`a_release_is_spaced_behind_an_arrivals_metered_time_not_its_eta` (`handlers/flow.rs`).

## A departure into a high field still climbs from its own altitude

Clamping the profile's start to `start_alt.max(arr_elev)` made a sea-level departure into DEN skip
~5,400 ft of climb, so its ETA ran early (#315). Proof: commit ed8a0450;
`a_departure_into_a_high_field_still_climbs_from_its_own_altitude` (`feed/trajectory.rs`).

## Resolve a shared input in one place, so no caller can drop it

Five call sites each looked up the arrival field elevation and passed it as a bare `f64`; any one
could drop it and time a sea-level descent. #335 pinned all five; #411 removed the shape:
`predict::profile_from_here` resolves the elevation itself and no caller passes it. Proof: commits
3bbe3ae3, 5b0d5406; `a_projected_arrival_descends_to_the_field_not_sea_level` and
`the_fix_table_terminates_at_the_arrival_field_elevation` (`handlers/flow.rs`).

## Every surface times an aircraft the same way

The arrival ladder, airport-flow demand and runway ETE timed aircraft on a great circle (×1.12 on the
ground) while FCA metering and the map used the resolved route, so one aircraft's ladder slot never
matched its map position (#63). `feed/predict.rs` is now the one service. Its first tests ran only the
straight-line fallback (empty nav), so reverting the route resolution left them green; they now run
against the bundled nav and a real routing. Proof: commits 176438fa, d071e002;
`arrival_and_crossing_eta_agree_on_the_resolved_route` (`feed/predict.rs`) and
`ladder_and_runway_ete_agree_on_the_resolved_route` (`feed/flow.rs`).

## A placeholder position is a position

The prefile branch of the per-fix table fed `(0.0, 0.0)` into route resolution; ground-route trimming
projected it onto the route, reading ~4,900 nm for KJFK→KDCA or collapsing the route (#225). Resolve
the departure like every other prefile caller and skip one that cannot be resolved. Proof: commit
9f72fceb; `a_prefile_is_timed_from_its_departure_and_matches_the_real_arrival_eta`
(`handlers/flow.rs`).

## Not measured is not zero

A `NULL` pushback meant both "did not push" and "push not detected", and only ~45 % of real pushes are
detected at the feed's cadence. Counting `NULL` as zero would have taught a hub a 0 s pushback; dropping
it made a no-tug field unrepresentable (#287). The observer now stores `0` only for a departure that
taxied off the stand with no push, and `NULL` for unmeasured. Proof: commits 616bef62, a7a6a7f0;
`a_missed_push_and_a_real_no_push_are_recorded_differently` (`feed/taxi_observations.rs`).

## A stationary aircraft's heading is its parking orientation

Runway matching for the ground allowance read live heading on aircraft still at the gate, so a parked
heading within ±30° of an unrelated runway pulled that runway's taxi samples into metering (found in
#181's review). Match on heading only once groundspeed reaches `TAXI_ROLL_GS_KT`. Proof: commit
595f3482; `resolve_ground_allowance_sec_ignores_a_coincidental_heading_match_while_stationary`
(`feed/flow.rs`).

## A predicate applied at nine sites is tested at the sites

Manual flight exclusions were applied at ~9 production sites with only the predicate tested:
replacing every production call with a constant left the suite at 432 passed (#342). The same review
found the write took no ARTCC scope, though one facility's exclusion hides the aircraft nationally.
Proof: commit 31cac565; `a_manually_excluded_callsign_is_dropped_from_live_traffic` and
`…_from_projected_traffic` (`handlers/flow.rs`),
`a_facility_grant_cannot_exclude_through_another_facilitys_fca` (`handlers/flight_exclusions.rs`).

## The coarse permission is not the facility's permission

Release and CFR writes (#626) and the four FCA writes (#636) checked only `RequirePermission`, so any
holder could write at any ARTCC, and a scoped machine credential inherited that reach. The unscoped
edit was also the bypass for the release gate: relabel another facility's FCA as your own, then
release on it. Check the stored and
the new ARTCC on every write. Proof: commits 0e2c5e6d, 24775c1c;
`a_scoped_holder_marks_a_release_only_in_its_artcc` (`handlers/machine_actor_tests.rs`),
`a_scoped_holder_can_neither_edit_nor_relabel_across_artccs` (`handlers/flow.rs`).

## One resolver answers scope and deny together

`access.v_effective_user_permissions` honored denies but dropped `artcc_id`, so a deny scoped to ZDC
revoked the permission nationally; `permission_scope` honored scope and never read a deny. `/me` could
contradict itself (#543). One resolver now composes both; a national deny removes the permission
everywhere, a scoped deny only at its scope. Proof: commit 54c874cd; `the_deny_rule_over_the_whole_matrix`
(`repos/access.rs`), `a_denied_reader_is_neither_national_nor_in_the_permission_tree`
(`handlers/auth.rs`).

## A delegation guard reads the delegator's scope and denies

The editor's no-escalation guard read the actor's permission names only, so a ZDC-scoped editor could
grant at ZNY or nationally and delegate a permission they had been denied. Saving a scope also deleted
its denies and re-inserted grants only, widening access with no guard noticing (#559). Proof: commit
deae8267; `a_zdc_editor_cannot_grant_at_another_artcc` and `a_zdc_editor_cannot_grant_nationally`
(`handlers/access.rs`), eleven adversarial tests with a positive control.

## A restriction checked when a grant is written is bypassed by every other path to it

The service-account denylist (`api_keys.*`, `service_accounts.*`) was checked only when a grant was
written, so a permission reached through an older or widened role still let an account mint
credentials; rotate handed any `service_accounts.update` holder the account's whole authority (#584).
Enforce at resolution. Proof: commit 33e6d65c; `a_forbidden_permission_reached_through_a_role_is_never_held`
and `an_admin_cannot_rotate_an_account_that_outranks_them` (`handlers/service_account_grant_tests.rs`).

## A role reconciled at sign-in outlives its removal

Migration 0098 backfilled every grant as `manual`, including the `SERVER_ADMIN` rows the env
reconciliation wrote, and revocation removed only `system` rows, so a CID taken out of
`OIS_SERVER_ADMIN_CID` kept the role for good. The first fix demoted at sign-in, which a live session or
API key never repeats (#805). Now startup, a 5-minute job and sign-in all demote. Proof: commits
e02dac22, 91f27cdf; `a_backfilled_admin_no_longer_configured_is_demoted_at_sign_in`
(`handlers/auth.rs`), `a_removed_admin_loses_admin_on_a_live_session_at_startup`
(`handlers/access_reset.rs`).

## A hidden record is hidden on every route

`list_fcas` hid unpublished and deleted FCAs while the public traffic route loaded through `get_fca`,
which applied neither filter, so an FCA's UUID read its metered traffic anonymously (#586). The order,
release and swap routes later skipped the event-FCA guard the edit routes had, and answered 403 where a
missing id answers 404 (#746). Proof: commits e1cc68c8, 2cafd030;
`an_anonymous_caller_sees_only_what_the_public_list_shows` (`handlers/flow.rs`),
`hidden_fcas_404_out_of_scope_like_a_missing_id` (`handlers/flow_event_fca_visibility_tests.rs`).

## A renamed logical key strands every deployed mapping

Renaming the NTML channel's logical name moved the lookup key without moving the
`integration.discord_channels` rows, so every guild would have resolved to `None` and TMIs would have
stopped reaching Discord with a 200 and no log (#436). Migration 0083 repoints the rows and skips a guild
holding both names; the publish now warns when nothing is mapped. Proof: commit b3283d01;
`the_ntml_channel_mapping_survives_the_rename` and
`a_guild_holding_both_channel_names_does_not_break_the_rename` (`handlers/tmu.rs`).

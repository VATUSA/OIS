//! Persistence for VATUSA member sync — member detail on `identity.users`, the mirrored
//! roles/visits tables, and the per-facility webhook secrets.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::auth::acl;
use crate::errors::ApiError;
use crate::feed::vatusa::VatusaMember;
use crate::models::{UserAccessBody, VatusaProfile, VatusaRoleEntry, VatusaRoleMappingBody};
use crate::repos::access::{self as access_repo, GrantSource};
use crate::repos::audit as audit_repo;

/// The member's VATUSA details (for their profile / `/me`), or `None` if never synced.
pub async fn fetch_profile(pool: &PgPool, cid: i64) -> Result<Option<VatusaProfile>, ApiError> {
    type Row = (
        Option<String>,
        Option<i32>,
        Option<bool>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    );
    let row = sqlx::query_as::<_, Row>(
        "select home_facility, rating_numeric, flag_home_controller, facility_join, vatusa_synced_at
         from identity.users where cid = $1",
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    let Some((home_facility, rating_numeric, home_controller, facility_join, synced_at)) = row
    else {
        return Ok(None);
    };
    if synced_at.is_none() {
        return Ok(None); // user exists but hasn't been synced from VATUSA yet
    }

    let roles = sqlx::query_as::<_, (String, String)>(
        "select facility, role from identity.vatusa_roles where cid = $1 order by facility, role",
    )
    .bind(cid)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let visits = sqlx::query_scalar::<_, String>(
        "select facility from identity.vatusa_visits where cid = $1 order by facility",
    )
    .bind(cid)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(Some(VatusaProfile {
        home_facility,
        rating_numeric,
        home_controller,
        facility_join,
        synced_at,
        roles: roles
            .into_iter()
            .map(|(facility, role)| VatusaRoleEntry { facility, role })
            .collect(),
        visits,
    }))
}

/// Update a member's details and fully replace their roles/visits, keyed on CID. Only touches
/// users we already have (the row is created at login); a missing CID updates zero rows.
pub async fn upsert_member(pool: &PgPool, m: &VatusaMember) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    sqlx::query(
        "update identity.users set
             full_name = coalesce(nullif($2, ''), full_name),
             home_facility = nullif($3, ''),
             rating = coalesce($4, rating),
             rating_numeric = $5,
             flag_home_controller = $6,
             facility_join = $7,
             vatusa_synced_at = now(),
             updated_at = now()
         where cid = $1",
    )
    .bind(m.cid)
    .bind(m.full_name())
    // Normalised as the pull's are: the roster grant (#730) joins it against org.facilities.
    .bind(normalise_facility(&m.facility))
    .bind(m.short_rating())
    .bind(m.rating)
    .bind(m.flag_homecontroller)
    .bind(m.facility_join_ts())
    .execute(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    // The update above holds this member's `identity.users` row lock until commit, so overlapping
    // syncs for one member (sign-in + webhook) run one after another from here on — the reconciler
    // below never races itself, and needs no lock of its own (#548).
    //
    // What the member's roles justified *before* this sync: the audit trail names the VATUSA role
    // that caused a removal, and after the rewrite below that role is gone from the table.
    let justified_before = desired_vatusa_grants(&mut tx, m.cid).await?;

    sqlx::query("delete from identity.vatusa_roles where cid = $1")
        .bind(m.cid)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for role in &m.roles {
        // Stored verbatim before #548; now it is joined against org.facilities and the mappings, so
        // " zdc " must arrive as "ZDC".
        let facility = normalise_facility(&role.facility);
        let role_name = role.role.trim().to_uppercase();
        if facility.is_empty() || role_name.is_empty() {
            continue;
        }
        let granted = role
            .created_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
        sqlx::query(
            "insert into identity.vatusa_roles (cid, facility, role, granted_at)
             values ($1, $2, $3, $4)
             on conflict (cid, facility, role) do nothing",
        )
        .bind(m.cid)
        .bind(&facility)
        .bind(&role_name)
        .bind(granted)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    sqlx::query("delete from identity.vatusa_visits where cid = $1")
        .bind(m.cid)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    for visit in &m.visiting_facilities {
        let facility = normalise_facility(&visit.facility);
        if facility.is_empty() {
            continue;
        }
        sqlx::query(
            "insert into identity.vatusa_visits (cid, facility) values ($1, $2)
             on conflict (cid, facility) do nothing",
        )
        .bind(m.cid)
        .bind(&facility)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    // VATUSA is authoritative for the Discord link — mirror it into external_sync_mappings (which the
    // bot resolves) whenever we have the OIS user row. Absent/blank ⇒ clear any stale mapping.
    let user_id = sqlx::query_scalar::<_, String>("select id from identity.users where cid = $1")
        .bind(m.cid)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    if let Some(user_id) = &user_id {
        let justified_now = desired_vatusa_grants(&mut tx, m.cid).await?;
        reconcile_vatusa_grants(&mut tx, user_id, m.cid, &justified_before, &justified_now).await?;
    }

    if let Some(user_id) = user_id {
        let discord_id = m
            .discord_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        match discord_id {
            Some(did) => {
                // One OIS user per Discord id: drop any other user's stale claim on it first.
                sqlx::query(
                    "delete from integration.external_sync_mappings \
                     where system_code = 'discord' and entity_type = 'user' \
                       and external_id = $1 and local_id <> $2",
                )
                .bind(did)
                .bind(&user_id)
                .execute(&mut *tx)
                .await
                .map_err(|_| ApiError::Internal)?;
                sqlx::query(
                    "insert into integration.external_sync_mappings \
                         (system_code, entity_type, local_id, external_id, metadata) \
                     values ('discord', 'user', $1, $2, '{\"source\":\"vatusa\"}'::jsonb) \
                     on conflict (system_code, entity_type, local_id) \
                     do update set external_id = excluded.external_id",
                )
                .bind(&user_id)
                .bind(did)
                .execute(&mut *tx)
                .await
                .map_err(|_| ApiError::Internal)?;
            }
            None => {
                sqlx::query(
                    "delete from integration.external_sync_mappings \
                     where system_code = 'discord' and entity_type = 'user' and local_id = $1",
                )
                .bind(&user_id)
                .execute(&mut *tx)
                .await
                .map_err(|_| ApiError::Internal)?;
            }
        }
    }

    tx.commit().await.map_err(|_| ApiError::Internal)
}

/// The group every rostered controller holds at their home ARTCC and each visiting ARTCC (#730): the
/// baseline operational set (migration 0126). Granted by the reconciler like a role mapping, with
/// `source = 'vatusa'`, so it comes and goes with the roster and never touches a hand-made grant.
pub const ROSTER_GROUP: &str = "CONTROLLER";

/// Group grants a member's VATUSA roles call for, keyed by `(group, scope)`, each with the VATUSA
/// roles that justify it (`DATM@ZDC`) — the audit trail names them.
type JustifiedGrants = BTreeMap<(String, Option<String>), Vec<String>>;

/// The grants VATUSA justifies for a member, from two sources:
///
/// - **Role mappings** (#548): `access.vatusa_role_mappings` applied to their VATUSA roles, scoped to
///   the facility the role is held at. A division role (`ZHQ`, not an ARTCC) is a **national** grant.
/// - **The roster** (#730): [`ROSTER_GROUP`] at their home ARTCC and at each visiting ARTCC. Never
///   national: a `ZHQ` home is not an ARTCC, so it grants nothing.
///
/// Any facility missing from `org.facilities` is skipped, so the `access.user_roles` FK is never hit.
async fn desired_vatusa_grants(
    tx: &mut Transaction<'_, Postgres>,
    cid: i64,
) -> Result<JustifiedGrants, ApiError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Vec<String>)>(
        r#"
        with justified(role_name, artcc_id, because) as (
            select m.role_name,
                   case when vr.facility = 'ZHQ' then null else f.id end,
                   vr.role || '@' || vr.facility
            from identity.vatusa_roles vr
            join access.vatusa_role_mappings m
              on m.vatusa_role = vr.role and (m.facility is null or m.facility = vr.facility)
            left join org.facilities f on f.id = vr.facility
            where vr.cid = $1 and (vr.facility = 'ZHQ' or f.id is not null)
            union all
            select $2, f.id, 'roster home ' || f.id
            from identity.users u
            join org.facilities f on f.id = u.home_facility
            where u.cid = $1
            union all
            select $2, f.id, 'roster visit ' || f.id
            from identity.vatusa_visits v
            join org.facilities f on f.id = v.facility
            where v.cid = $1
        )
        select role_name, artcc_id, array_agg(distinct because order by because)
        from justified
        group by 1, 2
        "#,
    )
    .bind(cid)
    .bind(ROSTER_GROUP)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(group, scope, because)| ((group, scope), because))
        .collect())
}

/// What reconciling would change for a member: the VATUSA-justified grants they don't hold as
/// `source = 'vatusa'` rows, and the `vatusa` rows nothing justifies any more. Shared by the reconcile
/// and the admin's Resync preview (#549), so the preview is exactly what a Resync applies.
type PendingChanges = (
    Vec<((String, Option<String>), Vec<String>)>,
    Vec<(String, Option<String>)>,
);

async fn pending_changes(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    justified_now: &JustifiedGrants,
) -> Result<PendingChanges, ApiError> {
    let held: BTreeSet<(String, Option<String>)> = sqlx::query_as(
        "select role_name, artcc_id from access.user_roles \
         where user_id = $1 and source = 'vatusa'",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?
    .into_iter()
    .collect();
    let grants = justified_now
        .iter()
        .filter(|(key, _)| !held.contains(*key))
        .map(|(key, because)| (key.clone(), because.clone()))
        .collect();
    let revokes = held
        .into_iter()
        .filter(|key| !justified_now.contains_key(key))
        .collect();
    Ok((grants, revokes))
}

async fn is_detached(tx: &mut Transaction<'_, Postgres>, user_id: &str) -> Result<bool, ApiError> {
    sqlx::query_scalar(
        "select vatusa_roles_detached_at is not null from identity.users where id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map(|d| d.unwrap_or(false))
    .map_err(|_| ApiError::Internal)
}

/// Take a member off VATUSA role sync because an admin edited their access by hand (#549). Records
/// who and when only the first time, and returns whether this call is the one that detached them —
/// so the caller audits the detach once, not on every later edit.
pub async fn detach_roles(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    by_user_id: &str,
) -> Result<bool, ApiError> {
    sqlx::query(
        "update identity.users set vatusa_roles_detached_at = now(), vatusa_roles_detached_by = $2 \
         where id = $1 and vatusa_roles_detached_at is null",
    )
    .bind(user_id)
    .bind(by_user_id)
    .execute(&mut **tx)
    .await
    .map(|r| r.rows_affected() > 0)
    .map_err(|_| ApiError::Internal)
}

/// A member's role-sync state for the admin view: when and by whom they were detached, or `None` while
/// synced.
pub async fn detached_state(
    pool: &PgPool,
    user_id: &str,
) -> Result<Option<(DateTime<Utc>, Option<String>)>, ApiError> {
    sqlx::query_as::<_, (Option<DateTime<Utc>>, Option<String>)>(
        "select u.vatusa_roles_detached_at, b.display_name \
         from identity.users u left join identity.users b on b.id = u.vatusa_roles_detached_by \
         where u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.and_then(|(at, by)| at.map(|at| (at, by))))
    .map_err(|_| ApiError::Internal)
}

/// What a Resync would change for the member: `(grants, revokes)` as `(group, scope)` pairs, from a
/// transaction that is rolled back — nothing is written (#549 AC3).
pub async fn preview_resync(
    pool: &PgPool,
    user_id: &str,
    cid: i64,
) -> Result<PendingChanges, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let justified_now = desired_vatusa_grants(&mut tx, cid).await?;
    let changes = pending_changes(&mut tx, user_id, &justified_now).await?;
    tx.rollback().await.map_err(|_| ApiError::Internal)?;
    Ok(changes)
}

/// Put a member back on VATUSA role sync and reconcile them now (#549 AC3), in the caller's
/// transaction. The reconcile audits its own changes as `VATUSA sync`, as any sync does.
pub async fn resync(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    cid: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "update identity.users set vatusa_roles_detached_at = null, vatusa_roles_detached_by = null \
         where id = $1",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    reconcile_member(tx, cid, &BTreeMap::new())
        .await
        .map(|_| ())
}

// --- Reset everyone's access to VATUSA (#795) ---

/// Groups a reset never removes, whatever their `source`. Migration 0098 backfilled every grant that
/// existed then as `manual`, the baseline group and `SERVER_ADMIN` included, so matching on `source`
/// alone would strip older members' baseline and the server admin's own role.
pub const RESET_KEEPS_GROUPS: [&str; 2] = [access_repo::BASELINE_ROLE, acl::SERVER_ADMIN_ROLE];

/// One stored grant row: a group membership or a direct permission, at a scope (`None` = national),
/// with whose row it is. `granted` is `false` for a direct deny and always `true` for a group.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GrantRow {
    pub is_group: bool,
    pub name: String,
    pub artcc_id: Option<String>,
    pub source: String,
    pub granted: bool,
}

impl GrantRow {
    fn label(&self) -> String {
        let what = match (self.is_group, self.granted) {
            (true, _) => format!("group {}", self.name),
            (false, true) => format!("permission {}", self.name),
            (false, false) => format!("deny {}", self.name),
        };
        format!("{what} {} ({})", scope_label(&self.artcc_id), self.source)
    }
}

/// What a reset changed, or would change, for one member.
#[derive(Debug)]
pub struct MemberReset {
    pub user_id: String,
    pub cid: Option<i64>,
    pub display_name: String,
    /// They were off VATUSA role sync and are back on it.
    pub reattached: bool,
    pub added: Vec<GrantRow>,
    pub removed: Vec<GrantRow>,
    pub before: UserAccessBody,
    pub after: UserAccessBody,
}

impl MemberReset {
    /// The audit reason: the admin's reason, then every row the reset added or removed. The snapshots
    /// either side show access, not provenance or denies, so the list is what makes the entry an undo
    /// trail.
    fn audit_reason(&self, reason: &str) -> String {
        let mut changes: Vec<String> = self
            .removed
            .iter()
            .map(|row| format!("removed {}", row.label()))
            .chain(
                self.added
                    .iter()
                    .map(|row| format!("added {}", row.label())),
            )
            .collect();
        if self.reattached {
            changes.push("re-attached to VATUSA role sync".to_string());
        }
        format!("Reset to VATUSA: {reason} ({})", changes.join("; "))
    }
}

async fn grant_rows(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<BTreeSet<GrantRow>, ApiError> {
    let rows = sqlx::query_as::<_, (bool, String, Option<String>, String, bool)>(
        "select true, role_name, artcc_id, source, true from access.user_roles where user_id = $1 \
         union all \
         select false, permission_name, artcc_id, source, granted from access.user_permissions \
         where user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(is_group, name, artcc_id, source, granted)| GrantRow {
            is_group,
            name,
            artcc_id,
            source,
            granted,
        })
        .collect())
}

/// Reset one member to exactly their `system` grants plus what VATUSA justifies, in the caller's
/// transaction: put them back on role sync, delete every `manual` group and direct-permission row
/// (allow or deny) except [`RESET_KEEPS_GROUPS`], and reconcile their `vatusa` grants against their
/// stored VATUSA roles. Writes no audit; the caller audits, or rolls back for a dry run.
///
/// Returns `None` when nothing changed. Takes the member's `identity.users` row lock, the lock a sync
/// holds, so a concurrent sync for the same member waits.
pub async fn reset_member(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<Option<MemberReset>, ApiError> {
    let Some((cid, display_name, detached)) = sqlx::query_as::<_, (Option<i64>, String, bool)>(
        "select cid, display_name, vatusa_roles_detached_at is not null from identity.users \
             where id = $1 for update",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?
    else {
        return Ok(None);
    };
    let rows_before = grant_rows(tx, user_id).await?;
    let before = access_snapshot(tx, user_id, cid.unwrap_or_default()).await?;

    if detached {
        sqlx::query(
            "update identity.users \
             set vatusa_roles_detached_at = null, vatusa_roles_detached_by = null where id = $1",
        )
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    sqlx::query(
        "delete from access.user_roles \
         where user_id = $1 and source = 'manual' and role_name <> all($2)",
    )
    .bind(user_id)
    .bind(RESET_KEEPS_GROUPS)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from access.user_permissions where user_id = $1 and source = 'manual'")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    if let Some(cid) = cid {
        let justified_now = desired_vatusa_grants(tx, cid).await?;
        let (grants, revokes) = pending_changes(tx, user_id, &justified_now).await?;
        for ((group, scope), _) in &grants {
            access_repo::set_user_role_scoped(
                tx,
                user_id,
                group,
                true,
                scope.as_deref(),
                GrantSource::Vatusa,
            )
            .await?;
        }
        for (group, scope) in &revokes {
            access_repo::set_user_role_scoped(
                tx,
                user_id,
                group,
                false,
                scope.as_deref(),
                GrantSource::Vatusa,
            )
            .await?;
        }
    }

    let rows_after = grant_rows(tx, user_id).await?;
    let removed: Vec<GrantRow> = rows_before.difference(&rows_after).cloned().collect();
    let added: Vec<GrantRow> = rows_after.difference(&rows_before).cloned().collect();
    if !detached && removed.is_empty() && added.is_empty() {
        return Ok(None);
    }
    let after = access_snapshot(tx, user_id, cid.unwrap_or_default()).await?;
    Ok(Some(MemberReset {
        user_id: user_id.to_string(),
        cid,
        display_name,
        reattached: detached,
        added,
        removed,
        before,
        after,
    }))
}

/// Whether a reset writes, and who it is audited as.
pub enum ResetMode<'a> {
    /// Every member's reset runs in a transaction that is rolled back: nothing is written.
    DryRun,
    Apply {
        actor_id: Option<String>,
        reason: &'a str,
        ip_address: Option<String>,
    },
}

/// The outcome of a reset over every member. `failure` is set when it stopped part-way: the members
/// in `changed` are reset (or, in a dry run, would be) and the rest are untouched.
pub struct ResetRun {
    /// Every member, examined or not.
    pub users_checked: usize,
    pub changed: Vec<MemberReset>,
    pub failure: Option<ApiError>,
}

/// The members a reset can change, in CID order: detached, holding a `manual` row other than a
/// [`RESET_KEEPS_GROUPS`] group, or holding `vatusa` group grants that differ from what their stored
/// VATUSA roles justify. Everyone else already holds exactly their `system` grants plus VATUSA's, so
/// [`reset_member`] would change nothing for them. The division pull seeds every rostered controller
/// into `identity.users` and reconciles every attached member, so most members are in none of these
/// groups, and a reset opens a transaction only for the members it touches.
///
/// "Justified" here is the set form of [`desired_vatusa_grants`], over every member at once. The
/// `vatusa` drift is one set-based diff (justified `FULL JOIN` held, keeping the rows with one side
/// missing), not a per-member comparison: a correlated subquery over the CTE rescans it once per
/// member, which took 31 s at 15,000 users. A full join needs a hashable condition, so a national
/// (null) scope is compared through `coalesce(artcc_id, '')`, as the unique index does (0098).
pub(crate) async fn reset_candidates(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar(
        r#"
        with justified(cid, role_name, artcc_id) as (
            select vr.cid, m.role_name, case when vr.facility = 'ZHQ' then null else f.id end
            from identity.vatusa_roles vr
            join access.vatusa_role_mappings m
              on m.vatusa_role = vr.role and (m.facility is null or m.facility = vr.facility)
            left join org.facilities f on f.id = vr.facility
            where vr.facility = 'ZHQ' or f.id is not null
            union
            select u.cid, $2, f.id
            from identity.users u join org.facilities f on f.id = u.home_facility
            union
            select v.cid, $2, f.id
            from identity.vatusa_visits v join org.facilities f on f.id = v.facility
        ),
        drift(user_id) as (
            select coalesce(j.user_id, h.user_id)
            from (select u.id as user_id, g.role_name, g.artcc_id
                  from justified g join identity.users u on u.cid = g.cid) j
            full join (select r.user_id, r.role_name, r.artcc_id from access.user_roles r
                       where r.source = 'vatusa') h
              on h.user_id = j.user_id and h.role_name = j.role_name
             and coalesce(h.artcc_id, '') = coalesce(j.artcc_id, '')
            where j.user_id is null or h.user_id is null
        )
        select u.id from identity.users u
        where u.vatusa_roles_detached_at is not null
           or exists (select 1 from access.user_roles r
                      where r.user_id = u.id and r.source = 'manual' and r.role_name <> all($1))
           or exists (select 1 from access.user_permissions p
                      where p.user_id = u.id and p.source = 'manual')
           or u.id in (select user_id from drift)
        order by u.cid nulls last, u.id
        "#,
    )
    .bind(RESET_KEEPS_GROUPS)
    .bind(ROSTER_GROUP)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Reset every member (#795) a reset can change ([`reset_candidates`]), one transaction each, in CID
/// order. A member's reset, its audit entry and its re-attach commit together, so a failure part-way
/// leaves no member half-reset; it stops the run and is returned beside the members already reset. A
/// member with no change gets no audit entry.
pub async fn reset_all(pool: &PgPool, mode: &ResetMode<'_>) -> ResetRun {
    let mut run = ResetRun {
        users_checked: 0,
        changed: Vec::new(),
        failure: None,
    };
    let total: Result<i64, _> = sqlx::query_scalar("select count(*) from identity.users")
        .fetch_one(pool)
        .await;
    let (total, users) = match (total, reset_candidates(pool).await) {
        (Ok(total), Ok(users)) => (total, users),
        (Err(_), _) | (_, Err(_)) => {
            run.failure = Some(ApiError::Internal);
            return run;
        }
    };
    run.users_checked = total as usize;
    for user_id in &users {
        match reset_one(pool, user_id, mode).await {
            Ok(change) => run.changed.extend(change),
            Err(e) => {
                tracing::error!(user_id, error = %e, "access reset to VATUSA failed for a member");
                run.failure = Some(e);
                break;
            }
        }
    }
    run
}

async fn reset_one(
    pool: &PgPool,
    user_id: &str,
    mode: &ResetMode<'_>,
) -> Result<Option<MemberReset>, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let change = reset_member(&mut tx, user_id).await?;
    match mode {
        ResetMode::DryRun => tx.rollback().await.map_err(|_| ApiError::Internal)?,
        ResetMode::Apply {
            actor_id,
            reason,
            ip_address,
        } => {
            if let Some(change) = &change {
                audit_repo::record_audit(
                    &mut *tx,
                    audit_repo::AuditEntry {
                        actor_id: actor_id.clone(),
                        action: "UPDATE".to_string(),
                        resource_type: "USER_ACCESS".to_string(),
                        resource_id: Some(change.user_id.clone()),
                        artcc_id: None,
                        reason: Some(change.audit_reason(reason)),
                        before_state: serde_json::to_value(&change.before).ok(),
                        after_state: serde_json::to_value(&change.after).ok(),
                        ip_address: ip_address.clone(),
                    },
                )
                .await?;
            }
            tx.commit().await.map_err(|_| ApiError::Internal)?;
        }
    }
    Ok(change)
}

/// Make the member's `source = 'vatusa'` group grants equal what their VATUSA roles justify. Compares
/// against the rows actually held rather than the previous sync's view, so a mapping edited between
/// syncs, or a sync that failed half-way, converges on the next run.
///
/// Every write goes through `set_user_role_scoped(…, GrantSource::Vatusa)`, which only ever touches
/// `vatusa` rows — a hand-made grant of the same group at the same scope is a separate row (0098) and
/// survives a demotion.
///
/// A sync that changes anything is audited **exactly as an admin edit is** (#546 AC6): one `UPDATE` on
/// `USER_ACCESS`, keyed on the member, with the full access snapshot either side — so one query finds
/// a controller's whole access history, by hand or by sync. The actor is `VATUSA sync` and the reason
/// names each change and the VATUSA role behind it (#548 AC6). A sync that changes nothing writes
/// nothing.
async fn reconcile_vatusa_grants(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    cid: i64,
    justified_before: &JustifiedGrants,
    justified_now: &JustifiedGrants,
) -> Result<bool, ApiError> {
    // A hand-managed member is off role sync until a Resync (#549). Every reconcile — sign-in, the
    // division pull, a mapping edit — comes through here, so this one check covers them all.
    if is_detached(tx, user_id).await? {
        return Ok(false);
    }
    let (grants, revokes) = pending_changes(tx, user_id, justified_now).await?;
    if grants.is_empty() && revokes.is_empty() {
        return Ok(false);
    }

    let before = access_snapshot(tx, user_id, cid).await?;
    let mut changes = Vec::with_capacity(grants.len() + revokes.len());
    for ((group, scope), because) in &grants {
        access_repo::set_user_role_scoped(
            tx,
            user_id,
            group,
            true,
            scope.as_deref(),
            GrantSource::Vatusa,
        )
        .await?;
        changes.push(format!(
            "granted {group} {} (holds {})",
            scope_label(scope),
            because.join(", ")
        ));
    }
    for key @ (group, scope) in &revokes {
        access_repo::set_user_role_scoped(
            tx,
            user_id,
            group,
            false,
            scope.as_deref(),
            GrantSource::Vatusa,
        )
        .await?;
        // Caused by a role the member no longer holds — or, when nothing justified it even before
        // this sync, by its mapping having been removed.
        let why = match justified_before.get(key) {
            Some(because) => format!("no longer holds {}", because.join(", ")),
            None => "no mapped VATUSA role supports it".to_string(),
        };
        changes.push(format!("revoked {group} {} ({why})", scope_label(scope)));
    }
    let after = access_snapshot(tx, user_id, cid).await?;

    audit_repo::record_audit(
        &mut **tx,
        audit_repo::AuditEntry {
            actor_id: Some(VATUSA_SYNC_ACTOR.to_string()),
            action: "UPDATE".to_string(),
            resource_type: "USER_ACCESS".to_string(),
            resource_id: Some(user_id.to_string()),
            artcc_id: None,
            reason: Some(format!("VATUSA sync: {}", changes.join("; "))),
            before_state: serde_json::to_value(&before).ok(),
            after_state: serde_json::to_value(&after).ok(),
            ip_address: None,
        },
    )
    .await?;
    Ok(true)
}

// --- Role → group mappings (#548) ---

/// The VATUSA roles confirmed in the division pull (`GET /v3/division/controllers`, VATUSA's
/// `acl_user_role` table), for documentation and tests — not a filter: the editor offers whatever
/// [`fetch_known_vatusa_roles`] actually sees. They are the **long** form; VATUSA's per-facility
/// endpoint lists the same grants under short codes (`EC`, `INS`, `WM`, `FACCBT`, …) that the sync never
/// receives, so a mapping on a short code would match nobody (#699). There is no assistant role:
/// VATUSA's `AEC` is "holds `EVENT_COORDINATOR` but isn't the facility's point of contact", which a
/// mapping can't express, so OIS's `AEC` group stays hand-assigned.
pub const DOCUMENTED_VATUSA_ROLES: &[&str] = &[
    "DIVISION_TECH_TEAM",
    "EVENT_COORDINATOR",
    "FACILITY_ACADEMY_EDITOR",
    "INSTRUCTOR",
    "WEB_MAINTAINER",
];

/// Whether `role` (already trimmed and uppercased) can name a VATUSA role: `A–Z`, `0–9` and `_`, at
/// most 64 characters. The real vocabulary is long-form with underscores (`FACILITY_ACADEMY_EDITOR` is
/// 23), which the earlier alphanumeric, 16-character rule refused (#699).
pub fn is_valid_vatusa_role(role: &str) -> bool {
    !role.is_empty()
        && role.len() <= 64
        && role
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

type MappingRow = (i64, String, Option<String>, String, DateTime<Utc>, i64);

fn mapping_body(
    (id, vatusa_role, facility, role_name, created_at, holders): MappingRow,
) -> VatusaRoleMappingBody {
    VatusaRoleMappingBody {
        id,
        vatusa_role,
        facility,
        role_name,
        created_at,
        holders,
    }
}

/// `holders` is how many synced members the mapping matches today, so the editor can say when one
/// grants nobody (#699).
const MAPPING_SELECT: &str = "select m.id, m.vatusa_role, m.facility, m.role_name, m.created_at, \
     (select count(distinct r.cid) from identity.vatusa_roles r \
      where r.role = m.vatusa_role and (m.facility is null or r.facility = m.facility)) \
     from access.vatusa_role_mappings m";

pub async fn fetch_role_mappings(pool: &PgPool) -> Result<Vec<VatusaRoleMappingBody>, ApiError> {
    let rows = sqlx::query_as::<_, MappingRow>(&format!(
        "{MAPPING_SELECT} order by m.role_name, m.vatusa_role, m.facility nulls first"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().map(mapping_body).collect())
}

pub async fn fetch_role_mapping(
    pool: &PgPool,
    id: i64,
) -> Result<Option<VatusaRoleMappingBody>, ApiError> {
    let row = sqlx::query_as::<_, MappingRow>(&format!("{MAPPING_SELECT} where m.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(row.map(mapping_body))
}

/// The VATUSA roles seen in synced members, for the mapping editor's picker.
pub async fn fetch_known_vatusa_roles(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar("select distinct role from identity.vatusa_roles order by role")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Insert a mapping, or `Conflict` if the same one exists. Race-free: the unique index decides.
pub async fn create_role_mapping(
    tx: &mut Transaction<'_, Postgres>,
    vatusa_role: &str,
    facility: Option<&str>,
    role_name: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "insert into access.vatusa_role_mappings (vatusa_role, facility, role_name) \
         values ($1, $2, $3) \
         on conflict (vatusa_role, (coalesce(facility, '')), role_name) do nothing \
         returning id",
    )
    .bind(vatusa_role)
    .bind(facility)
    .bind(role_name)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?
    .ok_or(ApiError::Conflict)
}

pub async fn delete_role_mapping(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
) -> Result<(), ApiError> {
    sqlx::query("delete from access.vatusa_role_mappings where id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Re-reconcile every member a mapping change can affect — those whose stored VATUSA roles include
/// `vatusa_role` (at `facility`, when the mapping has one) — so the change takes effect now rather
/// than at each member's next sync. Uses the stored roles; no VATUSA call. Returns how many.
pub async fn reconcile_members_holding(
    tx: &mut Transaction<'_, Postgres>,
    vatusa_role: &str,
    facility: Option<&str>,
) -> Result<usize, ApiError> {
    let cids: Vec<i64> = sqlx::query_scalar(
        "select distinct vr.cid from identity.vatusa_roles vr \
         join identity.users u on u.cid = vr.cid \
         where vr.role = $1 and ($2::text is null or vr.facility = $2) \
         order by vr.cid",
    )
    .bind(vatusa_role)
    .bind(facility)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    for &cid in &cids {
        reconcile_member_access(tx, cid).await?;
    }
    Ok(cids.len())
}

/// Reconcile one member from their stored VATUSA roles. Takes the member's `identity.users` row lock —
/// the lock a sync holds — so this and a concurrent sync for the same member run one after the other.
async fn reconcile_member_access(
    tx: &mut Transaction<'_, Postgres>,
    cid: i64,
) -> Result<(), ApiError> {
    // No "before": the member's roles didn't change, a mapping did — so a removal's reason is that no
    // mapping supports the grant any more, which is what the reconciler says when it has no prior view.
    reconcile_member(tx, cid, &BTreeMap::new())
        .await
        .map(|_| ())
}

/// Reconcile one member from their stored VATUSA roles, given what those roles justified before the
/// change being applied (so a removal's audit names the role that was lost). Takes the member's
/// `identity.users` row lock — the lock a sync holds — so concurrent writers for one member queue.
async fn reconcile_member(
    tx: &mut Transaction<'_, Postgres>,
    cid: i64,
    justified_before: &JustifiedGrants,
) -> Result<bool, ApiError> {
    let Some(user_id) =
        sqlx::query_scalar::<_, String>("select id from identity.users where cid = $1 for update")
            .bind(cid)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| ApiError::Internal)?
    else {
        return Ok(false);
    };
    let justified_now = desired_vatusa_grants(tx, cid).await?;
    reconcile_vatusa_grants(tx, &user_id, cid, justified_before, &justified_now).await
}

fn scope_label(scope: &Option<String>) -> String {
    scope
        .as_deref()
        .map_or_else(|| "nationally".to_string(), |artcc| format!("at {artcc}"))
}

/// The member's access as the user editor shows it, read inside the transaction so it sees the
/// sync's own writes — the same snapshot an admin edit records.
async fn access_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    cid: i64,
) -> Result<UserAccessBody, ApiError> {
    let grants = access_repo::fetch_user_direct_grants(&mut **tx, user_id).await?;
    let roles = access_repo::fetch_user_role_grants(&mut **tx, user_id).await?;
    let mut body = acl::user_access_body(user_id, cid, grants, roles)?;
    if body.server_admin {
        let catalog = access_repo::fetch_access_catalog_names(&mut **tx).await?;
        acl::apply_server_admin_catalog(&mut body, &catalog)?;
    }
    Ok(body)
}

/// The audit actor seeded by migration 0100.
const VATUSA_SYNC_ACTOR: &str = "vatusa-sync";

// --- Division pull (VATUSA/OIS#605) ---

/// A facility code as OIS stores it: trimmed, uppercased, and with v3's division-wide `*` stored as
/// `ZHQ` — the marker v2 used — so the role mapping's national case (0100) and its editor's
/// "ZHQ (division)" choice work whichever API a role arrived from.
pub fn normalise_facility(raw: &str) -> String {
    match raw.trim() {
        "*" => "ZHQ".to_string(),
        code => code.to_uppercase(),
    }
}

/// One controller from the division pull, already normalised.
#[derive(Debug, Clone)]
pub struct DivisionMember {
    pub cid: i64,
    pub display_name: String,
    pub rating_numeric: i32,
    pub rating_short: Option<String>,
    pub facility: String,
    pub facility_join: Option<DateTime<Utc>>,
    pub visits: Vec<String>,
    /// `(facility, role, granted_at)`.
    pub roles: Vec<(String, String, Option<DateTime<Utc>>)>,
}

/// Members with a stored VATUSA sync — the floor the pull's sanity check compares against.
pub async fn count_synced_members(pool: &PgPool) -> Result<i64, ApiError> {
    sqlx::query_scalar("select count(*) from identity.users where vatusa_synced_at is not null")
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// How many VATUSA role grants are stored — the role half of the pull's truncation check.
pub async fn count_stored_roles(pool: &PgPool) -> Result<i64, ApiError> {
    sqlx::query_scalar("select count(*) from identity.vatusa_roles")
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Apply one chunk of the division pull in one transaction, as a handful of bulk statements rather
/// than ~10 per member: seed or refresh the users, diff (not replace) their roles and visits, and
/// re-reconcile the VATUSA-mapped access (#548) of exactly the members whose roles changed. Returns
/// `(users seeded, members whose roles changed)`.
///
/// `members` must be sorted by CID: rows are locked in that order, which is also the order the role
/// mapping editor's re-reconcile locks them, so the two can never deadlock.
///
/// Deliberately untouched: names and email (sign-in owns them, from VATSIM Connect), the audit actor
/// (created at sign-in, so seeded strangers don't get one), and **the Discord mapping** — v3 doesn't
/// carry `discord_id`, and absence must never read as "cleared" (sign-in refreshes it over v2).
pub async fn apply_division_chunk(
    pool: &PgPool,
    members: &[DivisionMember],
) -> Result<(usize, usize), ApiError> {
    let db = |_| ApiError::Internal;
    let cids: Vec<i64> = members.iter().map(|m| m.cid).collect();
    let mut tx = pool.begin().await.map_err(db)?;

    // What each member's VATUSA data justified before this pull, for every member and not only those
    // whose roles changed: the roster grant (#730) moves with their home facility and visits too. The
    // audit names what was lost from this.
    let mut before = BTreeMap::new();
    for &cid in &cids {
        before.insert(cid, desired_vatusa_grants(&mut tx, cid).await?);
    }

    let seeded: Vec<bool> = sqlx::query_scalar(
        r#"
        insert into identity.users as u
            (cid, full_name, display_name, rating, rating_numeric, home_facility,
             flag_home_controller, facility_join, vatusa_synced_at)
        select c.cid, c.name, c.name, c.short, c.numeric, nullif(c.facility, ''),
               exists (select 1 from org.facilities f where f.id = c.facility),
               c.joined, now()
        from unnest($1::bigint[], $2::text[], $3::text[], $4::int[], $5::text[], $6::timestamptz[])
             as c(cid, name, short, numeric, facility, joined)
        on conflict (cid) do update
        set rating = coalesce(excluded.rating, u.rating),
            rating_numeric = excluded.rating_numeric,
            home_facility = excluded.home_facility,
            -- v2 at sign-in carries VATUSA's own flag; the pull can only derive one. Deferring to an
            -- existing value keeps the profile from flipping between the two each day.
            flag_home_controller = coalesce(u.flag_home_controller, excluded.flag_home_controller),
            facility_join = coalesce(excluded.facility_join, u.facility_join),
            vatusa_synced_at = now(),
            updated_at = now()
        returning (xmax = 0)
        "#,
    )
    .bind(&cids)
    .bind(
        members
            .iter()
            .map(|m| m.display_name.clone())
            .collect::<Vec<_>>(),
    )
    .bind(
        members
            .iter()
            .map(|m| m.rating_short.clone())
            .collect::<Vec<_>>(),
    )
    .bind(members.iter().map(|m| m.rating_numeric).collect::<Vec<_>>())
    .bind(
        members
            .iter()
            .map(|m| m.facility.clone())
            .collect::<Vec<_>>(),
    )
    .bind(members.iter().map(|m| m.facility_join).collect::<Vec<_>>())
    .fetch_all(&mut *tx)
    .await
    .map_err(db)?;

    let roles: Vec<(i64, &str, &str, Option<DateTime<Utc>>)> = members
        .iter()
        .flat_map(|m| {
            m.roles
                .iter()
                .map(move |(f, r, at)| (m.cid, f.as_str(), r.as_str(), *at))
        })
        .collect();
    replace_roles(&mut tx, &cids, &roles).await?;

    let visits: Vec<(i64, &str)> = members
        .iter()
        .flat_map(|m| m.visits.iter().map(move |v| (m.cid, v.as_str())))
        .collect();
    replace_visits(&mut tx, &cids, &visits).await?;

    // Every member, once all their data is written, in CID order (the lock order). An unchanged
    // member writes and audits nothing; this also grants existing members on the first pull after a
    // new grant source ships, since nothing about them changes to trigger it.
    let mut moved = 0;
    for &cid in &cids {
        if reconcile_member(&mut tx, cid, &before[&cid]).await? {
            moved += 1;
        }
    }

    tx.commit().await.map_err(db)?;
    Ok((seeded.iter().filter(|s| **s).count(), moved))
}

/// Controllers who have left the division keep nothing VATUSA granted them: their stored roles, visits
/// and home facility go, and with them, through the reconciler, any VATUSA-mapped or roster access
/// (#730). "Departed" is anyone the pull doesn't list who still holds any of the three — a plain
/// controller holds no roles, so roles alone would never notice them leave. `present` is every CID in
/// the pull. Returns how many members were cleared. Only ever called after the pull's sanity floor.
pub async fn clear_departed(pool: &PgPool, present: &[i64]) -> Result<usize, ApiError> {
    let db = |_| ApiError::Internal;
    let departed: Vec<i64> = sqlx::query_scalar(
        "select cid from ( \
             select cid from identity.vatusa_roles \
             union select cid from identity.vatusa_visits \
             union select cid from identity.users where home_facility is not null \
         ) held where cid <> all($1) order by cid",
    )
    .bind(present)
    .fetch_all(pool)
    .await
    .map_err(db)?;
    for chunk in departed.chunks(500) {
        let mut tx = pool.begin().await.map_err(db)?;
        let mut before = BTreeMap::new();
        for &cid in chunk {
            before.insert(cid, desired_vatusa_grants(&mut tx, cid).await?);
        }
        replace_roles(&mut tx, chunk, &[]).await?;
        replace_visits(&mut tx, chunk, &[]).await?;
        sqlx::query("update identity.users set home_facility = null where cid = any($1)")
            .bind(chunk)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        for &cid in chunk {
            reconcile_member(&mut tx, cid, &before[&cid]).await?;
        }
        tx.commit().await.map_err(db)?;
    }
    Ok(departed.len())
}

/// Make the stored roles of `cids` exactly `roles`, deleting what's gone and inserting what's new
/// rather than rewriting the table daily. Reconciling is the caller's, once every write has landed.
async fn replace_roles(
    tx: &mut Transaction<'_, Postgres>,
    cids: &[i64],
    roles: &[(i64, &str, &str, Option<DateTime<Utc>>)],
) -> Result<(), ApiError> {
    let db = |_| ApiError::Internal;
    let r_cid: Vec<i64> = roles.iter().map(|r| r.0).collect();
    let r_fac: Vec<&str> = roles.iter().map(|r| r.1).collect();
    let r_role: Vec<&str> = roles.iter().map(|r| r.2).collect();
    let r_at: Vec<Option<DateTime<Utc>>> = roles.iter().map(|r| r.3).collect();
    sqlx::query(
        r#"
        delete from identity.vatusa_roles r
        where r.cid = any($4)
          and not exists (
              select 1 from unnest($1::bigint[], $2::text[], $3::text[]) d(cid, facility, role)
              where d.cid = r.cid and d.facility = r.facility and d.role = r.role)
        "#,
    )
    .bind(&r_cid)
    .bind(&r_fac)
    .bind(&r_role)
    .bind(cids)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "insert into identity.vatusa_roles (cid, facility, role, granted_at) \
         select * from unnest($1::bigint[], $2::text[], $3::text[], $4::timestamptz[]) \
         on conflict (cid, facility, role) do nothing",
    )
    .bind(&r_cid)
    .bind(&r_fac)
    .bind(&r_role)
    .bind(&r_at)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Make the stored visits of `cids` exactly `visits` (delete what's gone, insert what's new).
async fn replace_visits(
    tx: &mut Transaction<'_, Postgres>,
    cids: &[i64],
    visits: &[(i64, &str)],
) -> Result<(), ApiError> {
    let db = |_| ApiError::Internal;
    let v_cid: Vec<i64> = visits.iter().map(|v| v.0).collect();
    let v_fac: Vec<&str> = visits.iter().map(|v| v.1).collect();
    sqlx::query(
        "delete from identity.vatusa_visits v where v.cid = any($3) and not exists ( \
             select 1 from unnest($1::bigint[], $2::text[]) d(cid, facility) \
             where d.cid = v.cid and d.facility = v.facility)",
    )
    .bind(&v_cid)
    .bind(&v_fac)
    .bind(cids)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "insert into identity.vatusa_visits (cid, facility) \
         select * from unnest($1::bigint[], $2::text[]) on conflict (cid, facility) do nothing",
    )
    .bind(&v_cid)
    .bind(&v_fac)
    .execute(&mut **tx)
    .await
    .map_err(db)?;
    Ok(())
}

// --- The division webhook (#605) ---

/// The stored division webhook: VATUSA's id for it (read back from its webhook list, since creating
/// one returns only the secret), our receiver URL, and the secret encrypted with `OIS_SECRET_KEY`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredWebhook {
    pub vatusa_id: Option<i64>,
    pub url: String,
    pub secret_ciphertext: Vec<u8>,
    pub key_version: i32,
}

pub async fn fetch_webhook(pool: &PgPool) -> Result<Option<StoredWebhook>, ApiError> {
    sqlx::query_as(
        "select vatusa_id, url, secret_ciphertext, key_version from identity.vatusa_webhook",
    )
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn store_webhook(pool: &PgPool, webhook: &StoredWebhook) -> Result<(), ApiError> {
    sqlx::query(
        "insert into identity.vatusa_webhook (vatusa_id, url, secret_ciphertext, key_version) \
         values ($1, $2, $3, $4) \
         on conflict (singleton) do update set vatusa_id = excluded.vatusa_id, url = excluded.url, \
             secret_ciphertext = excluded.secret_ciphertext, \
             key_version = excluded.key_version, created_at = now()",
    )
    .bind(webhook.vatusa_id)
    .bind(&webhook.url)
    .bind(&webhook.secret_ciphertext)
    .bind(webhook.key_version)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::{detach_roles, detached_state, preview_resync, resync, upsert_member};
    use crate::feed::vatusa::VatusaMember;

    const CID: i64 = 1_548_000;

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar(
            "insert into identity.users (full_name, display_name, cid) \
             values ('T', 'T', $1) returning id",
        )
        .bind(CID)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn map(pool: &PgPool, vatusa_role: &str, facility: Option<&str>, group: &str) {
        sqlx::query(
            "insert into access.vatusa_role_mappings (vatusa_role, facility, role_name) \
             values ($1, $2, $3)",
        )
        .bind(vatusa_role)
        .bind(facility)
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }

    /// A sync as VATUSA would deliver it — built through serde, the same path the HTTP fetch takes.
    async fn sync(pool: &PgPool, roles: &[(&str, &str)]) {
        let roles: Vec<_> = roles
            .iter()
            .map(|(role, facility)| serde_json::json!({ "role": role, "facility": facility }))
            .collect();
        let member: VatusaMember =
            serde_json::from_value(serde_json::json!({ "cid": CID, "roles": roles })).unwrap();
        upsert_member(pool, &member).await.unwrap();
    }

    async fn grants(pool: &PgPool, user: &str) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as(
            "select role_name, artcc_id, source from access.user_roles \
             where user_id = $1 order by role_name, artcc_id nulls first, source",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn vatusa(group: &str, scope: Option<&str>) -> (String, Option<String>, String) {
        (
            group.to_string(),
            scope.map(str::to_string),
            "vatusa".to_string(),
        )
    }

    /// (action, artcc_id, reason, actor_id) for every sync-written membership audit row.
    /// (action, resource_id, reason, actor_id) for every sync-written access audit entry — the same
    /// `USER_ACCESS` key an admin edit uses (#546 AC6).
    async fn sync_audits(pool: &PgPool) -> Vec<(String, Option<String>, String, String)> {
        sqlx::query_as(
            "select action, resource_id, reason, actor_id from access.audit_logs \
             where resource_type = 'USER_ACCESS' and actor_id = 'vatusa-sync' \
             order by created_at, id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// AC1: a member holding `DATM@ZDC` receives the mapped group, scoped to ZDC, owned by the sync.
    #[sqlx::test]
    async fn a_mapped_role_grants_the_group_at_its_facility(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("DATM", "ZDC")]).await;

        assert_eq!(grants(&pool, &user).await, vec![vatusa("EC", Some("ZDC"))]);
    }

    /// AC2: losing the VATUSA role removes exactly the grant it caused — not a hand-made grant of the
    /// same group at the same scope, and not another synced grant.
    #[sqlx::test]
    async fn losing_the_role_removes_only_its_grant(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;
        map(&pool, "INS", None, "NTMO").await;
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, artcc_id, source) \
             values ($1, 'EC', 'ZDC', 'manual')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        sync(&pool, &[("DATM", "ZDC"), ("INS", "ZDC")]).await;
        sync(&pool, &[("INS", "ZDC")]).await;

        assert_eq!(
            grants(&pool, &user).await,
            vec![
                (
                    "EC".to_string(),
                    Some("ZDC".to_string()),
                    "manual".to_string()
                ),
                vatusa("NTMO", Some("ZDC")),
            ]
        );
    }

    /// The sync owns only its own rows, so "what is already held" means held **by the sync**. A
    /// hand-made grant of the mapped group at the same scope must not stop the sync from writing its
    /// own row — or removing the manual one later would take the member's access with it until the
    /// next sync — and a manual grant no mapping supports is not the sync's to revoke, so a sync that
    /// changes nothing records nothing. `losing_the_role_removes_only_its_grant` has this fixture but
    /// asserts only the end state, where reading every source looks the same.
    #[sqlx::test]
    async fn a_manual_grant_neither_stands_in_for_the_sync_nor_is_revoked_by_it(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;
        for (role, artcc) in [("EC", Some("ZDC")), ("NTMO", None)] {
            sqlx::query(
                "insert into access.user_roles (user_id, role_name, artcc_id, source) \
                 values ($1, $2, $3, 'manual')",
            )
            .bind(&user)
            .bind(role)
            .bind(artcc)
            .execute(&pool)
            .await
            .unwrap();
        }

        sync(&pool, &[("DATM", "ZDC")]).await;
        let held = grants(&pool, &user).await;
        assert!(
            held.contains(&vatusa("EC", Some("ZDC"))),
            "the sync writes its own EC@ZDC beside the manual one: {held:?}"
        );
        assert!(
            held.contains(&("NTMO".to_string(), None, "manual".to_string())),
            "and leaves the unmapped manual grant alone: {held:?}"
        );

        let audits = |pool: PgPool, user: String| async move {
            sqlx::query_scalar::<_, i64>(
                "select count(*) from access.audit_logs \
                 where resource_type = 'USER_ACCESS' and resource_id = $1",
            )
            .bind(&user)
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        let before = audits(pool.clone(), user.clone()).await;
        sync(&pool, &[("DATM", "ZDC")]).await;
        assert_eq!(
            audits(pool.clone(), user.clone()).await,
            before,
            "a repeat sync that changes nothing records nothing — not a revoke of the manual NTMO"
        );
    }

    /// AC4: a division role (`ZHQ`, not an ARTCC) is a national grant; a facility OIS doesn't know is
    /// skipped — neither can reach the `access.user_roles` FK.
    #[sqlx::test]
    async fn division_roles_are_national_and_unknown_facilities_are_skipped(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "WM", None, "VATUSA_STAFF").await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("WM", "ZHQ"), ("DATM", "ZZZ")]).await;

        assert_eq!(
            grants(&pool, &user).await,
            vec![vatusa("VATUSA_STAFF", None)]
        );
    }

    /// The national decision for `ZHQ` must not depend on `ZHQ` being absent from `org.facilities`:
    /// without the explicit case, adding it as a facility one day would silently turn every division
    /// grant into a ZHQ-scoped one. (Today the left join alone yields NULL, which hid this.)
    #[sqlx::test]
    async fn division_roles_stay_national_even_if_zhq_becomes_a_facility(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "WM", None, "VATUSA_STAFF").await;
        sqlx::query("insert into org.facilities (id, name) values ('ZHQ', 'VATUSA HQ')")
            .execute(&pool)
            .await
            .unwrap();

        sync(&pool, &[("WM", "ZHQ")]).await;

        assert_eq!(
            grants(&pool, &user).await,
            vec![vatusa("VATUSA_STAFF", None)]
        );
    }

    /// AC6: every sync that changes access is audited exactly as an admin edit is — `UPDATE` on
    /// `USER_ACCESS`, keyed on the member, with the access snapshot either side — by the `VATUSA sync`
    /// actor, naming the VATUSA role behind each change. That includes a removal, whose role is already
    /// gone from `identity.vatusa_roles` by the time it is written.
    #[sqlx::test]
    async fn every_change_is_audited_naming_the_vatusa_role(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("DATM", "ZDC")]).await;
        sync(&pool, &[]).await;

        let audits = sync_audits(&pool).await;
        assert_eq!(audits.len(), 2, "one per changing sync: {audits:?}");
        for (action, resource, _, actor) in &audits {
            assert_eq!(
                (action.as_str(), resource.as_deref(), actor.as_str()),
                ("UPDATE", Some(user.as_str()), "vatusa-sync")
            );
        }
        assert_eq!(
            audits[0].2,
            "VATUSA sync: granted EC at ZDC (holds DATM@ZDC)"
        );
        assert_eq!(
            audits[1].2,
            "VATUSA sync: revoked EC at ZDC (no longer holds DATM@ZDC)"
        );

        // The snapshots are the user editor's: the grant appears in the ZDC scope's roles after it.
        let (before, after): (serde_json::Value, serde_json::Value) = sqlx::query_as(
            "select before_state, after_state from access.audit_logs \
             where resource_type = 'USER_ACCESS' and actor_id = 'vatusa-sync' \
             order by created_at, id limit 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let zdc_roles = |state: &serde_json::Value| {
            state["scopes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["artcc_id"] == "ZDC")
                .map(|s| s["role_names"].clone())
        };
        assert_eq!(zdc_roles(&before), None);
        assert_eq!(zdc_roles(&after), Some(serde_json::json!(["EC"])));
    }

    /// A mapping pinned to a facility applies only there; one without a facility applies anywhere.
    #[sqlx::test]
    async fn a_facility_specific_mapping_applies_only_at_that_facility(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "ATM", Some("ZDC"), "EC").await;

        sync(&pool, &[("ATM", "ZNY"), ("ATM", "ZDC")]).await;

        assert_eq!(grants(&pool, &user).await, vec![vatusa("EC", Some("ZDC"))]);
    }

    /// The facility used to be stored verbatim; it is now joined against `org.facilities` and the
    /// mappings, so whitespace and case must not decide whether someone gets access.
    #[sqlx::test]
    async fn facilities_are_normalised_on_ingest(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("datm", " zdc ")]).await;

        assert_eq!(grants(&pool, &user).await, vec![vatusa("EC", Some("ZDC"))]);
        let stored: (String, String) =
            sqlx::query_as("select role, facility from identity.vatusa_roles where cid = $1")
                .bind(CID)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, ("DATM".to_string(), "ZDC".to_string()));
    }

    /// Re-syncing an unchanged member writes nothing and audits nothing — the six-hourly job must not
    /// fill the dossier with no-op churn.
    #[sqlx::test]
    async fn a_repeat_sync_changes_nothing(pool: PgPool) {
        seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("DATM", "ZDC")]).await;
        sync(&pool, &[("DATM", "ZDC")]).await;

        assert_eq!(sync_audits(&pool).await.len(), 1);
    }

    /// Deleting a mapping revokes what it granted on the member's next sync, and the audit says why.
    #[sqlx::test]
    async fn a_removed_mapping_revokes_on_the_next_sync(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;
        sync(&pool, &[("DATM", "ZDC")]).await;

        sqlx::query("delete from access.vatusa_role_mappings")
            .execute(&pool)
            .await
            .unwrap();
        sync(&pool, &[("DATM", "ZDC")]).await;

        assert!(grants(&pool, &user).await.is_empty());
        let audits = sync_audits(&pool).await;
        assert!(
            audits[1].2.contains("no mapped VATUSA role supports it"),
            "{audits:?}"
        );
    }

    // ---- #699: real role names, default mappings --------------------------------------------------

    async fn seeded(pool: &PgPool) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as(
            "select vatusa_role, facility, role_name from access.vatusa_role_mappings \
             order by vatusa_role",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// The owner's decision on #699, shipped by 0124: VATUSA's event coordinators are OIS's EC at their
    /// own ARTCC, and the division tech team is national VATUSA staff. Nothing else.
    #[sqlx::test]
    async fn the_default_mappings_are_seeded(pool: PgPool) {
        assert_eq!(
            seeded(&pool).await,
            vec![
                (
                    "DIVISION_TECH_TEAM".into(),
                    Some("ZHQ".into()),
                    "VATUSA_STAFF".into()
                ),
                ("EVENT_COORDINATOR".into(), None, "EC".into()),
            ]
        );
    }

    /// A default mapping on a role the sync never sends would grant nobody, silently — the failure
    /// #699 found. Every seeded role is one the division pull is known to send.
    #[sqlx::test]
    async fn seeded_mappings_name_documented_roles(pool: PgPool) {
        for (role, _, _) in seeded(&pool).await {
            assert!(
                super::DOCUMENTED_VATUSA_ROLES.contains(&role.as_str()),
                "{role} is not a documented VATUSA role"
            );
        }
    }

    /// The real vocabulary is long-form with underscores; the old rule (alphanumeric, ≤ 16) refused
    /// four of the five roles VATUSA actually sends.
    #[test]
    fn every_documented_role_passes_validation_and_junk_does_not() {
        for role in super::DOCUMENTED_VATUSA_ROLES {
            assert!(super::is_valid_vatusa_role(role), "{role} is refused");
        }
        for bad in ["", "EC-1", "EC 1", "ec", &"A".repeat(65)] {
            assert!(!super::is_valid_vatusa_role(bad), "{bad:?} is accepted");
        }
        assert!(super::is_valid_vatusa_role(&"A".repeat(64)));
    }

    /// A role nothing maps (here a real one) is stored, grants nothing, and is offered to the editor so
    /// an admin can map it.
    #[sqlx::test]
    async fn an_unmapped_role_is_stored_offered_and_grants_nothing(pool: PgPool) {
        let user = seed_user(&pool).await;
        sync(&pool, &[("FACILITY_ACADEMY_EDITOR", "ZDC")]).await;
        assert!(grants(&pool, &user).await.is_empty());
        assert!(
            super::fetch_known_vatusa_roles(&pool)
                .await
                .unwrap()
                .contains(&"FACILITY_ACADEMY_EDITOR".to_string())
        );
    }

    /// VATUSA sends a division-wide role with facility `*`. End to end through the ingest path, that is
    /// a national grant (artcc_id null) — not one scoped to a facility called `*` or `ZHQ`.
    #[sqlx::test]
    async fn a_star_division_role_is_a_national_grant(pool: PgPool) {
        let user = seed_user(&pool).await;
        sync(&pool, &[("DIVISION_TECH_TEAM", "*")]).await;
        assert_eq!(
            grants(&pool, &user).await,
            vec![vatusa("VATUSA_STAFF", None)]
        );
    }

    #[sqlx::test]
    async fn an_event_coordinator_is_ec_at_their_own_artcc(pool: PgPool) {
        let user = seed_user(&pool).await;
        sync(&pool, &[("EVENT_COORDINATOR", "ZDC")]).await;
        assert_eq!(grants(&pool, &user).await, vec![vatusa("EC", Some("ZDC"))]);
    }

    /// The editor's "grants nobody" warning reads this count.
    #[sqlx::test]
    async fn holders_counts_the_members_a_mapping_matches(pool: PgPool) {
        seed_user(&pool).await;
        map(&pool, "INSTRUCTOR", Some("ZLA"), "EC").await;
        sync(
            &pool,
            &[("EVENT_COORDINATOR", "ZDC"), ("INSTRUCTOR", "ZDC")],
        )
        .await;
        let holders: std::collections::BTreeMap<String, i64> = super::fetch_role_mappings(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|m| (m.vatusa_role, m.holders))
            .collect();
        assert_eq!(holders["EVENT_COORDINATOR"], 1, "any facility: matches ZDC");
        assert_eq!(
            holders["INSTRUCTOR"], 0,
            "held at ZDC, mapped at ZLA: matches nobody"
        );
        assert_eq!(holders["DIVISION_TECH_TEAM"], 0);
    }

    async fn detach(pool: &PgPool, user: &str) {
        let mut tx = pool.begin().await.unwrap();
        assert!(detach_roles(&mut tx, user, user).await.unwrap());
        tx.commit().await.unwrap();
    }

    // ---- #730: the roster grant ---------------------------------------------------------------

    /// Sign-in's path: a member with this home facility and these visits, and no VATUSA roles.
    async fn sign_in_as(pool: &PgPool, home: &str, visits: &[&str]) {
        let visits: Vec<_> = visits
            .iter()
            .map(|f| serde_json::json!({ "facility": f }))
            .collect();
        let member: VatusaMember = serde_json::from_value(serde_json::json!({
            "cid": CID, "facility": home, "visiting_facilities": visits, "roles": [],
        }))
        .unwrap();
        upsert_member(pool, &member).await.unwrap();
    }

    /// The division pull's path, for the same member. Returns how many members' access it moved.
    async fn pull(pool: &PgPool, home: &str, visits: &[&str]) -> usize {
        let member = super::DivisionMember {
            cid: CID,
            display_name: "T".into(),
            rating_numeric: 5,
            rating_short: None,
            facility: home.into(),
            facility_join: None,
            visits: visits.iter().map(|v| v.to_string()).collect(),
            roles: vec![],
        };
        super::apply_division_chunk(pool, &[member])
            .await
            .unwrap()
            .1
    }

    fn controller(artcc: &str) -> (String, Option<String>, String) {
        vatusa("CONTROLLER", Some(artcc))
    }

    /// AC1: at home and at every visiting ARTCC, on sign-in, with no manual step.
    #[sqlx::test]
    async fn a_rostered_controller_is_controller_at_home_and_each_visit(pool: PgPool) {
        let user = seed_user(&pool).await;
        sign_in_as(&pool, " zdc ", &["ZTL", "zny"]).await;
        assert_eq!(
            grants(&pool, &user).await,
            [controller("ZDC"), controller("ZNY"), controller("ZTL")],
            "normalised like the pull's, and one grant per facility"
        );
    }

    /// AC7, and the facilities that aren't ARTCCs: no home and no visits holds nothing beyond USER; a
    /// division (`ZHQ`) home or an unknown facility grants nothing, never a national CONTROLLER.
    #[sqlx::test]
    async fn no_rostered_artcc_grants_nothing(pool: PgPool) {
        let user = seed_user(&pool).await;
        sign_in_as(&pool, "", &[]).await;
        assert!(grants(&pool, &user).await.is_empty());
        sign_in_as(&pool, "ZHQ", &["ZZZ"]).await;
        assert!(grants(&pool, &user).await.is_empty());
    }

    /// AC3 through the pull: a transfer moves the grant, and the pull reports it moved (#644's nudge).
    #[sqlx::test]
    async fn a_transfer_moves_the_grant_on_the_next_pull(pool: PgPool) {
        let user = seed_user(&pool).await;
        assert_eq!(pull(&pool, "ZDC", &[]).await, 1);
        assert_eq!(grants(&pool, &user).await, [controller("ZDC")]);
        assert_eq!(pull(&pool, "ZTL", &[]).await, 1);
        assert_eq!(grants(&pool, &user).await, [controller("ZTL")]);
        assert_eq!(
            pull(&pool, "ZTL", &[]).await,
            0,
            "an unchanged member moves nothing"
        );
    }

    /// AC4 through the pull — a visit-only change, which reconciled nothing before #730.
    #[sqlx::test]
    async fn dropping_a_visit_removes_its_grant(pool: PgPool) {
        let user = seed_user(&pool).await;
        pull(&pool, "ZDC", &["ZTL"]).await;
        assert_eq!(
            grants(&pool, &user).await,
            [controller("ZDC"), controller("ZTL")]
        );
        assert_eq!(pull(&pool, "ZDC", &[]).await, 1);
        assert_eq!(grants(&pool, &user).await, [controller("ZDC")]);
    }

    /// A controller who leaves the division holds no VATUSA role, only a home and visits — they must
    /// still count as departed, lose both, and lose the grant.
    #[sqlx::test]
    async fn a_departed_controller_loses_the_grant(pool: PgPool) {
        let user = seed_user(&pool).await;
        let home = || async {
            sqlx::query_scalar::<_, Option<String>>(
                "select home_facility from identity.users where id = $1",
            )
            .bind(&user)
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        // A home and nothing else — the plainest controller, and the one roles alone never noticed.
        pull(&pool, "ZDC", &[]).await;
        assert_eq!(super::clear_departed(&pool, &[]).await.unwrap(), 1);
        assert!(grants(&pool, &user).await.is_empty());
        assert_eq!(home().await, None);
        // A visit and nothing else.
        pull(&pool, "", &["ZTL"]).await;
        assert_eq!(grants(&pool, &user).await, [controller("ZTL")]);
        assert_eq!(super::clear_departed(&pool, &[]).await.unwrap(), 1);
        assert!(grants(&pool, &user).await.is_empty());
    }

    /// AC5: a hand-made CONTROLLER grant — at a facility the roster never names, and at the very
    /// facility the sync grants and later revokes — survives every reconcile.
    #[sqlx::test]
    async fn a_hand_made_controller_grant_survives_every_reconcile(pool: PgPool) {
        let user = seed_user(&pool).await;
        let mut tx = pool.begin().await.unwrap();
        for at in ["ZNY", "ZDC"] {
            crate::repos::access::set_user_role_scoped(
                &mut tx,
                &user,
                "CONTROLLER",
                true,
                Some(at),
                crate::repos::access::GrantSource::Manual,
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        pull(&pool, "ZDC", &[]).await;
        pull(&pool, "ZTL", &[]).await;
        super::clear_departed(&pool, &[]).await.unwrap();
        let manual: Vec<Option<String>> = sqlx::query_scalar(
            "select artcc_id from access.user_roles \
             where user_id = $1 and role_name = 'CONTROLLER' and source = 'manual' order by 1",
        )
        .bind(&user)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(manual, [Some("ZDC".to_string()), Some("ZNY".to_string())]);
    }

    /// #549: a detached member is off sync for the roster grant too.
    #[sqlx::test]
    async fn a_detached_member_is_not_granted_controller(pool: PgPool) {
        let user = seed_user(&pool).await;
        detach(&pool, &user).await;
        sign_in_as(&pool, "ZDC", &["ZTL"]).await;
        pull(&pool, "ZDC", &["ZTL"]).await;
        assert!(grants(&pool, &user).await.is_empty());
    }

    /// #549 AC1 + AC4: once detached, a sync leaves the member's groups alone — though their VATUSA
    /// roles changed — while their identity details and stored VATUSA roles keep syncing.
    #[sqlx::test]
    async fn a_detached_member_keeps_their_groups_while_their_details_still_sync(pool: PgPool) {
        let user = seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;
        map(&pool, "TMU", None, "AEC").await;
        sync(&pool, &[("DATM", "ZDC")]).await;
        assert_eq!(grants(&pool, &user).await, [vatusa("EC", Some("ZDC"))]);

        detach(&pool, &user).await;
        let member: VatusaMember = serde_json::from_value(serde_json::json!({
            "cid": CID, "fname": "New", "lname": "Name", "facility": "ZNY",
            "roles": [{ "role": "TMU", "facility": "ZDC" }],
        }))
        .unwrap();
        upsert_member(&pool, &member).await.unwrap();

        assert_eq!(
            grants(&pool, &user).await,
            [vatusa("EC", Some("ZDC"))],
            "no AEC granted, no EC revoked"
        );
        let (name, facility): (String, Option<String>) =
            sqlx::query_as("select full_name, home_facility from identity.users where id = $1")
                .bind(&user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (name.as_str(), facility.as_deref()),
            ("New Name", Some("ZNY"))
        );
        let stored: Vec<String> =
            sqlx::query_scalar("select role from identity.vatusa_roles where cid = $1")
                .bind(CID)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(stored, ["TMU"], "the VATUSA roles themselves still sync");

        // The preview is what a Resync would do, from the roles stored while detached…
        let (add, remove) = preview_resync(&pool, &user, CID).await.unwrap();
        let add: Vec<_> = add.into_iter().map(|(key, _)| key).collect();
        // The member's home is now ZNY, so a Resync also grants the roster group there (#730); the
        // detach above kept it from being granted until then.
        assert_eq!(
            add,
            [
                ("AEC".to_string(), Some("ZDC".to_string())),
                ("CONTROLLER".to_string(), Some("ZNY".to_string()))
            ]
        );
        assert_eq!(remove, [("EC".to_string(), Some("ZDC".to_string()))]);
        // …and it wrote nothing.
        assert_eq!(grants(&pool, &user).await, [vatusa("EC", Some("ZDC"))]);

        let mut tx = pool.begin().await.unwrap();
        resync(&mut tx, &user, CID).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            grants(&pool, &user).await,
            [
                vatusa("AEC", Some("ZDC")),
                vatusa("CONTROLLER", Some("ZNY"))
            ],
            "the Resync applies exactly the preview, roster grant included (#730)"
        );
        assert!(detached_state(&pool, &user).await.unwrap().is_none());
    }

    /// Who and when are the first detach's: a second hand edit doesn't move them, and says so.
    #[sqlx::test]
    async fn only_the_first_edit_detaches(pool: PgPool) {
        let user = seed_user(&pool).await;
        detach(&pool, &user).await;
        let first = detached_state(&pool, &user).await.unwrap().unwrap().0;
        let mut tx = pool.begin().await.unwrap();
        assert!(!detach_roles(&mut tx, &user, &user).await.unwrap());
        tx.commit().await.unwrap();
        assert_eq!(
            detached_state(&pool, &user).await.unwrap().unwrap().0,
            first
        );
    }
}

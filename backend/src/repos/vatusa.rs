//! Persistence for VATUSA member sync — member detail on `identity.users`, the mirrored
//! roles/visits tables, and the per-facility webhook secrets.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::errors::ApiError;
use crate::feed::vatusa::VatusaMember;
use crate::models::{VatusaProfile, VatusaRoleEntry, VatusaRoleMappingBody};
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
    .bind(&m.facility)
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
        let facility = role.facility.trim().to_uppercase();
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
        if visit.facility.is_empty() {
            continue;
        }
        sqlx::query(
            "insert into identity.vatusa_visits (cid, facility) values ($1, $2)
             on conflict (cid, facility) do nothing",
        )
        .bind(m.cid)
        .bind(&visit.facility)
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

/// Group grants a member's VATUSA roles call for, keyed by `(group, scope)`, each with the VATUSA
/// roles that justify it (`DATM@ZDC`) — the audit trail names them.
type JustifiedGrants = BTreeMap<(String, Option<String>), Vec<String>>;

/// The grants `access.vatusa_role_mappings` derives from the member's current VATUSA roles (#548).
///
/// Scope is the facility the VATUSA role is held at, with two cases decided in the query so the
/// `access.user_roles` FK can never be hit: a division role (`ZHQ`, not an ARTCC) is a **national**
/// grant, and any other facility missing from `org.facilities` is skipped.
async fn desired_vatusa_grants(
    tx: &mut Transaction<'_, Postgres>,
    cid: i64,
) -> Result<JustifiedGrants, ApiError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Vec<String>)>(
        r#"
        select m.role_name,
               case when vr.facility = 'ZHQ' then null else f.id end as artcc_id,
               array_agg(distinct vr.role || '@' || vr.facility
                         order by vr.role || '@' || vr.facility) as because
        from identity.vatusa_roles vr
        join access.vatusa_role_mappings m
          on m.vatusa_role = vr.role and (m.facility is null or m.facility = vr.facility)
        left join org.facilities f on f.id = vr.facility
        where vr.cid = $1 and (vr.facility = 'ZHQ' or f.id is not null)
        group by 1, 2
        "#,
    )
    .bind(cid)
    .fetch_all(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(group, scope, because)| ((group, scope), because))
        .collect())
}

/// Make the member's `source = 'vatusa'` group grants equal what their VATUSA roles justify, auditing
/// every change. Compares against the rows actually held rather than the previous sync's view, so a
/// mapping edited between syncs, or a sync that failed half-way, converges on the next run.
///
/// Every write goes through `set_user_role_scoped(…, GrantSource::Vatusa)`, which only ever touches
/// `vatusa` rows — a hand-made grant of the same group at the same scope is a separate row (0098) and
/// survives a demotion.
async fn reconcile_vatusa_grants(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    cid: i64,
    justified_before: &JustifiedGrants,
    justified_now: &JustifiedGrants,
) -> Result<(), ApiError> {
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

    for ((group, scope), because) in justified_now {
        if held.contains(&(group.clone(), scope.clone())) {
            continue;
        }
        access_repo::set_user_role_scoped(
            tx,
            user_id,
            group,
            true,
            scope.as_deref(),
            GrantSource::Vatusa,
        )
        .await?;
        let reason = format!("VATUSA sync: holds {}", because.join(", "));
        audit_sync_change(tx, "GRANT", group, scope, cid, reason).await?;
    }

    for (group, scope) in &held {
        if justified_now.contains_key(&(group.clone(), scope.clone())) {
            continue;
        }
        access_repo::set_user_role_scoped(
            tx,
            user_id,
            group,
            false,
            scope.as_deref(),
            GrantSource::Vatusa,
        )
        .await?;
        // A removal is caused by a role the member no longer holds — or, when nothing justified it
        // even before this sync, by its mapping having been removed.
        let reason = match justified_before.get(&(group.clone(), scope.clone())) {
            Some(because) => format!("VATUSA sync: no longer holds {}", because.join(", ")),
            None => "VATUSA sync: no mapped VATUSA role supports it".to_string(),
        };
        audit_sync_change(tx, "REVOKE", group, scope, cid, reason).await?;
    }
    Ok(())
}

/// The audit actor seeded by migration 0100.
const VATUSA_SYNC_ACTOR: &str = "vatusa-sync";

/// Same shape as a hand-made membership change (`handlers::access::change_membership`), so sync
/// grants appear beside manual ones under the same `ACCESS_GROUP_MEMBER` dossier filter.
async fn audit_sync_change(
    tx: &mut Transaction<'_, Postgres>,
    action: &str,
    group: &str,
    scope: &Option<String>,
    cid: i64,
    reason: String,
) -> Result<(), ApiError> {
    audit_repo::record_audit(
        &mut **tx,
        audit_repo::AuditEntry {
            actor_id: Some(VATUSA_SYNC_ACTOR.to_string()),
            action: action.to_string(),
            resource_type: "ACCESS_GROUP_MEMBER".to_string(),
            resource_id: Some(format!("{group}:{cid}")),
            artcc_id: scope.clone(),
            reason: Some(reason),
            before_state: None,
            after_state: None,
            ip_address: None,
        },
    )
    .await
}

// --- Role → group mappings (#548) ---

type MappingRow = (i64, String, Option<String>, String, DateTime<Utc>);

fn mapping_body(
    (id, vatusa_role, facility, role_name, created_at): MappingRow,
) -> VatusaRoleMappingBody {
    VatusaRoleMappingBody {
        id,
        vatusa_role,
        facility,
        role_name,
        created_at,
    }
}

const MAPPING_SELECT: &str =
    "select id, vatusa_role, facility, role_name, created_at from access.vatusa_role_mappings";

pub async fn fetch_role_mappings(pool: &PgPool) -> Result<Vec<VatusaRoleMappingBody>, ApiError> {
    let rows = sqlx::query_as::<_, MappingRow>(&format!(
        "{MAPPING_SELECT} order by role_name, vatusa_role, facility nulls first"
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
    let row = sqlx::query_as::<_, MappingRow>(&format!("{MAPPING_SELECT} where id = $1"))
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
    let Some(user_id) =
        sqlx::query_scalar::<_, String>("select id from identity.users where cid = $1 for update")
            .bind(cid)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| ApiError::Internal)?
    else {
        return Ok(());
    };
    let justified_now = desired_vatusa_grants(tx, cid).await?;
    // No "before": the member's roles didn't change, a mapping did — so a removal's reason is that no
    // mapping supports the grant any more, which is what the reconciler says when it has no prior view.
    reconcile_vatusa_grants(tx, &user_id, cid, &BTreeMap::new(), &justified_now).await
}

/// CIDs from the given set that we actually have a user row for.
pub async fn known_cids(pool: &PgPool, cids: &[i64]) -> Result<Vec<i64>, ApiError> {
    if cids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar::<_, i64>("select cid from identity.users where cid = any($1)")
        .bind(cids)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// The least-recently-synced members (never-synced first), for reconciliation.
pub async fn stale_member_cids(pool: &PgPool, limit: i64) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "select cid from identity.users
         where cid is not null
         order by vatusa_synced_at asc nulls first
         limit $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Active ARTCC ids, used to decide which facilities to register webhooks for.
pub async fn active_facilities(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select id from org.facilities where active = true order by id")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Facilities that already have a registered webhook.
pub async fn registered_facilities(pool: &PgPool) -> Result<HashSet<String>, ApiError> {
    let rows = sqlx::query_scalar::<_, String>("select facility from identity.vatusa_webhooks")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

pub async fn upsert_webhook(
    pool: &PgPool,
    facility: &str,
    webhook_id: i64,
    secret: &str,
    url: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into identity.vatusa_webhooks (facility, webhook_id, secret, url)
         values ($1, $2, $3, $4)
         on conflict (facility) do update
         set webhook_id = excluded.webhook_id,
             secret = excluded.secret,
             url = excluded.url,
             updated_at = now()",
    )
    .bind(facility)
    .bind(webhook_id)
    .bind(secret)
    .bind(url)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|_| ApiError::Internal)
}

/// The signing secret for a facility's webhook, used to verify inbound deliveries.
pub async fn webhook_secret(pool: &PgPool, facility: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select secret from identity.vatusa_webhooks where facility = $1",
    )
    .bind(facility)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::upsert_member;
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
    async fn sync_audits(pool: &PgPool) -> Vec<(String, Option<String>, String, String)> {
        sqlx::query_as(
            "select action, artcc_id, reason, actor_id from access.audit_logs \
             where resource_type = 'ACCESS_GROUP_MEMBER' and actor_id = 'vatusa-sync' \
             order by created_at, action",
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

    /// AC6: every sync-driven change is audited by the `vatusa-sync` actor, naming the VATUSA role that
    /// caused it — including a removal, whose role is already gone from `identity.vatusa_roles`.
    #[sqlx::test]
    async fn every_change_is_audited_naming_the_vatusa_role(pool: PgPool) {
        seed_user(&pool).await;
        map(&pool, "DATM", None, "EC").await;

        sync(&pool, &[("DATM", "ZDC")]).await;
        sync(&pool, &[]).await;

        let audits = sync_audits(&pool).await;
        assert_eq!(audits.len(), 2, "one grant, one revoke: {audits:?}");
        let (action, scope, reason, actor) = &audits[0];
        assert_eq!(
            (action.as_str(), scope.as_deref(), actor.as_str()),
            ("GRANT", Some("ZDC"), "vatusa-sync")
        );
        assert!(reason.contains("DATM@ZDC"), "{reason}");
        let (action, _, reason, _) = &audits[1];
        assert_eq!(action, "REVOKE");
        assert!(reason.contains("no longer holds DATM@ZDC"), "{reason}");
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

    /// Mappings are seeded with nothing, so deploying this grants nobody anything (owner decision).
    #[sqlx::test]
    async fn no_mappings_are_seeded(pool: PgPool) {
        let count: i64 = sqlx::query_scalar("select count(*) from access.vatusa_role_mappings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

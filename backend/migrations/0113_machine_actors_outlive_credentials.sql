-- VATUSA/OIS#659: a machine's `access.actors` row outlives the credential it stood for.
--
-- A machine release names its writer by actor (`flow.fca_release.updated_by_actor`,
-- `tmu.issued_cfrs.issued_by_actor`, 0102), and release authority and IDST's "via <tool>" read that
-- actor's type and name (#585). The actor used to cascade from its API key (0045) or service account
-- (0003), so deleting the credential erased the actor, nulled the release's attribution, and turned a
-- machine's committed time into a person's.
--
-- Unlinking the actor instead keeps its `actor_type` and `display_name`: the release stays the
-- machine's, a committed time stands, and audit rows keep naming who wrote them. An unlinked actor is
-- never reused, because the actor lookups match on the credential column. Users are unchanged.

alter table access.actors drop constraint actors_api_key_id_fkey;
alter table access.actors
    add constraint actors_api_key_id_fkey
    foreign key (api_key_id) references access.api_keys(id) on delete set null;

alter table access.actors drop constraint actors_service_account_id_fkey;
alter table access.actors
    add constraint actors_service_account_id_fkey
    foreign key (service_account_id) references access.service_accounts(id) on delete set null;

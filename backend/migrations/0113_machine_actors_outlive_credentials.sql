-- @formatter:off
-- VATUSA/OIS#659: a machine's actor outlives its credential.
--
-- A release or CFR names its writer through `access.actors` (#583, #585). The actor row used to
-- cascade from its API key or service account, so deleting the credential erased the actor, nulled
-- the release's `updated_by_actor`/`issued_by_actor`, and turned a machine's committed time into what
-- looked like a person's: the IDST lost its "via …" provenance and the release read as
-- `held_by_person`. A committed time stands (#585), and so must the record of who committed it. The
-- actor now survives with its `display_name` and `actor_type`; only the link to the deleted credential
-- is cleared. User actors are unchanged.

alter table access.actors
    drop constraint if exists actors_api_key_id_fkey,
    add constraint actors_api_key_id_fkey foreign key (api_key_id)
        references access.api_keys(id) on delete set null;

alter table access.actors
    drop constraint if exists actors_service_account_id_fkey,
    add constraint actors_service_account_id_fkey foreign key (service_account_id)
        references access.service_accounts(id) on delete set null;

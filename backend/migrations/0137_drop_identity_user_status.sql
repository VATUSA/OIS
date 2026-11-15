-- @formatter:off
-- VATUSA/OIS#944: identity.users.status (0002) offered ACTIVE/INACTIVE/SUSPENDED, but no code ever
-- set or enforced it, so it read as a suspension mechanism that did not exist. Drop the column and its
-- index; the inline check constraint goes with the column. This supersedes the 0094 comment that
-- describes the column as present. Suspension, if it is ever wanted, is its own feature.
--
-- The stored values are discarded and cannot be recovered.

drop index identity.idx_identity_users_status;

alter table identity.users drop column status;

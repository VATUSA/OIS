-- Collapse direct permission grants that a held group already gives (VATUSA/OIS#550).
--
-- Presets expanded into concrete rows: applying "VATUSA Admin" wrote ~80 `access.user_permissions`
-- rows per user. Now that groups carry real permission sets (#544) and are editable (#545, #546),
-- those rows duplicate what the user's groups grant. Leaving them is not harmless: they are invisible
-- in the group UI, they never change when the group changes, and so they drift -- remove a permission
-- from a group and every user who once had a preset applied silently keeps it.
--
-- This generalises `0094_seed_role_permissions.sql`'s delete, which collapsed only the five `USER`
-- baseline permissions, and follows the same guard.
--
-- ## A row is removed only if it is redundant -- every one of these must hold
--
-- 1. `granted is true`. **A deny is never removed**: deleting one would *widen* access.
-- 2. The user holds a group whose `role_permissions` include that permission.
-- 3. That membership's scope **covers** the row's. A national membership covers a row at any scope; a
--    facility membership covers only a row at that same facility. A **national direct grant under a
--    facility-only membership is not redundant** -- it grants more -- and stays. In SQL that falls out
--    of `ur.artcc_id = up.artcc_id` being NULL, not true, when the row is national and the
--    membership is not.
-- 4. The covering group is not `SERVER_ADMIN`. It is env-bootstrapped and removed automatically when
--    the env flag goes (`repos/access.rs`), so a grant folded into it would vanish with no admin
--    action. Today it has no `role_permissions` at all -- its access comes from the view's cross
--    join -- so condition 2 already excludes it; this says so rather than relying on the absence of
--    rows.
--
-- Anything else -- a bespoke grant no group covers -- is untouched.
--
-- ## Why this cannot change anyone's effective access
--
-- Every removed row is a grant the same user already receives, at a scope at least as wide, from a
-- membership that stays. Denies are never touched, so whatever they narrowed they still narrow. The
-- test `the_cleanup_changes_no_one_s_effective_permissions` asserts this through the real resolver,
-- before and after, rather than trusting this argument.

delete from access.user_permissions up
where up.granted is true
  and exists (
      select 1
      from access.user_roles ur
      join access.role_permissions rp on rp.role_name = ur.role_name
      where ur.user_id = up.user_id
        and rp.permission_name = up.permission_name
        and ur.role_name <> 'SERVER_ADMIN'
        and (ur.artcc_id is null or ur.artcc_id = up.artcc_id)
  );

-- @formatter:off
-- Deleting a saved replay (#432).
--
-- Saved captures pin full-fidelity positions forever: `CAPTURE_GUARD` keeps every row inside an
-- 'open' or 'saved' window out of `downsample_positions()`, so a mis-scoped capture is storage that
-- can never be given back. Deletion marks the capture 'discarded' — already a permitted status — which
-- drops it out of that guard, so compaction reclaims the space on its next pass and the row survives
-- for audit.
--
-- Separate from `stats.capture.update` deliberately: saving a window and destroying one someone else
-- saved are different levels of trust.

insert into access.permissions (name, description) values
    ('stats.capture.delete', 'Delete a saved capture window, releasing its pinned position data')
on conflict (name) do nothing;

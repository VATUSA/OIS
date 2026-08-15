-- @formatter:off
-- Flow map routes are now defined by a filed-route STRING (e.g. "RBV Q430 BYRDD J48 MOL
-- FLASK OZZZI2") that the nav engine resolves to a track on read — not a hand-drawn polyline.
-- Add the route text plus optional departure/arrival airports (which improve SID/STAR and
-- preferred-route resolution). The old `points` column is left in place but unused.

alter table flow.route
    add column if not exists route text not null default '',
    add column if not exists dep text not null default '',
    add column if not exists arr text not null default '';

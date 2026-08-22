# Roles & permissions

What you can do in OIS is governed by **permissions**, granted through **roles** and scoped per-ARTCC.

## How it works

- Permissions are fine-grained and path-based (`segments.action` — e.g. `flow.fca.update`).
- They're bundled into **roles** (for example a TMU or events role), which are assigned to users — optionally **scoped to a specific ARTCC** so a role only applies within that facility's airspace.
- A national-level grant applies everywhere; an ARTCC-scoped grant applies only there. For example, `flow.facility_map.update` (edit a facility map's aircraft coloring) is typically granted **scoped to one ARTCC**, so a controller can recolor only their own facility's map.

## VATUSA roles vs. OIS access

Your [VATUSA roles](/introduction/profile) (ATM, INS, …) describe your real-world facility position. Your **OIS access** is what you can do in OIS. They're related but managed separately — having a VATUSA staff position doesn't automatically grant OIS permissions unless an administrator maps it.

## Managing access

Administrators manage grants in the **access control** editor. It lists **all users in a browsable, paginated table** (search by name or CID when you know who you want), and for each user shows their roles and permissions at the national level and per ARTCC. Every change is recorded with a **reason** to the user's audit log.

## Server admin

A small set of CIDs configured on the server hold **SERVER_ADMIN**, which grants everything. This is set in deployment configuration, not through the UI.

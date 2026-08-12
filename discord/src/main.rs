//! OIS Discord bot — Phase 4 scaffold.
//!
//! Responsibilities (see `docs/features/discord-integration.md`):
//!   - poll `integration.outbound_jobs` via the backend API and execute them
//!     (auto-create event threads, post TMI/ADV embeds, post ACE request embeds)
//!   - handle interactions (claim buttons, slash commands) by calling back into the
//!     backend as a service account
//!   - VATSIM<->Discord account linking

fn main() {
    println!("ois-discord: scaffold — see docs/features/discord-integration.md");
}

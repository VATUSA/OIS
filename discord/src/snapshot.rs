use std::sync::Arc;

use ois_client::{GuildChannelSnap, GuildRoleSnap, GuildSnap, OisClient};
use serenity::all::{ChannelType, Http};

/// Convert a serenity `ChannelType` to the short kind string stored in the snapshot.
fn channel_kind(k: ChannelType) -> &'static str {
    match k {
        ChannelType::Text => "text",
        ChannelType::Voice => "voice",
        ChannelType::Category => "category",
        ChannelType::News => "announcement",
        ChannelType::Forum => "forum",
        ChannelType::Stage => "stage",
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread => {
            "thread"
        }
        _ => "other",
    }
}

/// Pull every guild the bot is in (its channels + roles) via REST and push the full snapshot to the
/// backend, so the admin config can offer dropdowns.
pub(crate) async fn snapshot_and_push(http: &Arc<Http>, api: &OisClient) -> Result<(), String> {
    let guilds = http
        .get_guilds(None, None)
        .await
        .map_err(|e| format!("get_guilds: {e}"))?;
    let mut out = Vec::with_capacity(guilds.len());
    for gi in guilds {
        let gid = gi.id;
        let channels = gid
            .channels(http)
            .await
            .map_err(|e| format!("channels({gid}): {e}"))?;
        let roles = gid
            .roles(http)
            .await
            .map_err(|e| format!("roles({gid}): {e}"))?;
        out.push(GuildSnap {
            guild_id: gid.get().to_string(),
            name: gi.name.clone(),
            channels: channels
                .values()
                .map(|c| GuildChannelSnap {
                    id: c.id.get().to_string(),
                    name: c.name.clone(),
                    kind: channel_kind(c.kind).to_string(),
                    parent_id: c.parent_id.map(|p| p.get().to_string()),
                    position: c.position as i32,
                })
                .collect(),
            roles: roles
                .values()
                .map(|r| GuildRoleSnap {
                    id: r.id.get().to_string(),
                    name: r.name.clone(),
                    managed: r.managed,
                    position: r.position as i32,
                })
                .collect(),
        });
    }
    let count = out.len();
    api.push_guild_snapshot(out)
        .await
        .map_err(|e| format!("push: {e}"))?;
    tracing::info!(guilds = count, "pushed guild snapshot");
    Ok(())
}

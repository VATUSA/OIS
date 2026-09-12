use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, ChannelType, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage,
    CreateThread, Http, RoleId, UserId,
};

use crate::util::{channel, str_field, truncate};

const DIVIDER: &str = "───────────────────────────";

pub(crate) async fn create_event_thread(
    http: &Arc<Http>,
    p: &Value,
) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    let title = str_field(p, "event_title").unwrap_or("Event coordination");
    let thread_name = str_field(p, "thread_name").unwrap_or(title);
    let date_line = str_field(p, "date_line").unwrap_or("");

    let thread = channel
        .create_thread(
            http,
            CreateThread::new(truncate(thread_name, 100)).kind(ChannelType::PublicThread),
        )
        .await
        .map_err(|e| format!("create_thread failed: {e}"))?;

    // Facility lines: ping each facility's EC(s) — the OIS users holding the `EC` role scoped to that
    // ARTCC (from Access Control), by their linked Discord id. NTMO/DCC-Trainee stay config roles.
    let parse_role = |v: Option<&str>| v.and_then(|s| s.parse::<u64>().ok()).map(RoleId::new);
    let mut role_ids: Vec<RoleId> = Vec::new();
    let mut user_ids: Vec<UserId> = Vec::new();
    let mut facility_lines = String::new();
    if let Some(facs) = p.get("facilities").and_then(Value::as_array) {
        for f in facs {
            let id = f.get("id").and_then(Value::as_str).unwrap_or("?");
            let ecs: Vec<u64> = f
                .get("ec_user_ids")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|s| s.parse::<u64>().ok())
                        .collect()
                })
                .unwrap_or_default();
            let ping = if ecs.is_empty() {
                " _(no EC assigned)_".to_string()
            } else {
                ecs.iter().map(|u| format!(" <@{u}>")).collect()
            };
            for u in &ecs {
                user_ids.push(UserId::new(*u));
            }
            facility_lines.push_str(&format!("• **{id}**{ping}\n"));
        }
    }
    if facility_lines.is_empty() {
        facility_lines.push_str("_No facilities marked required/preferred yet._\n");
    }

    let ntmo = str_field(p, "ntmo_role_id");
    let dcc = str_field(p, "dcc_trainee_role_id");
    for r in [parse_role(ntmo), parse_role(dcc)].into_iter().flatten() {
        role_ids.push(r);
    }
    let ntmo_ping = ntmo
        .map(|r| format!("<@&{r}>"))
        .unwrap_or_else(|| "@NTMO".to_string());
    let dcc_ping = dcc
        .map(|r| format!("<@&{r}>"))
        .unwrap_or_else(|| "@DCC Trainee".to_string());

    let content = format!(
        "**{title} | Planning Thread**\n\
         {title} is on {date_line}\n\n\
         Review the following for your facility:\n\
         - TMU/TMI package\n\
         - Staffing\n\
         - Configs and AAR\n\n\
         {facility_lines}\n\
         Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and \
         fill out all appropriate areas of the staffing data.\n\
         {DIVIDER}\n\
         {ntmo_ping} please react with your availability to NOM for this event. {dcc_ping} please \
         react with your availability to shadow this event.\n\n\
         🟢 = Available\n🟡 = Partially available/unsure\n🔴 = Unavailable\n\
         {DIVIDER}"
    );

    let event_id = p
        .get("event_id")
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_default();
    let buttons = vec![
        CreateButton::new(format!("evtavail:green:{event_id}"))
            .emoji('🟢')
            .style(ButtonStyle::Success),
        CreateButton::new(format!("evtavail:yellow:{event_id}"))
            .emoji('🟡')
            .style(ButtonStyle::Secondary),
        CreateButton::new(format!("evtavail:red:{event_id}"))
            .emoji('🔴')
            .style(ButtonStyle::Danger),
    ];

    // Discord rejects duplicate ids in allowed_mentions (an EC can cover several facilities; roles
    // can repeat too).
    role_ids.sort();
    role_ids.dedup();
    user_ids.sort();
    user_ids.dedup();
    let message = CreateMessage::new()
        .content(content)
        .allowed_mentions(CreateAllowedMentions::new().roles(role_ids).users(user_ids))
        .components(vec![CreateActionRow::Buttons(buttons)]);
    thread
        .id
        .send_message(http, message)
        .await
        .map_err(|e| format!("thread send_message failed: {e}"))?;
    Ok(Some(json!({ "thread_id": thread.id.get().to_string() })))
}

use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, ChannelType, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage,
    CreateThread, Http, RoleId, UserId,
};

use crate::util::{channel, str_field, truncate};

/// Fallback body if the payload somehow lacks `thread_template` (e.g. mid-deploy) — identical text
/// to what was hard-coded here before the website-configured template existed.
const FALLBACK_TEMPLATE: &str = "**{{title}} | Planning Thread**\n\
    {{title}} is on {{date_line}}\n\n\
    Review the following for your facility:\n\
    - TMU/TMI package\n\
    - Staffing\n\
    - Configs and AAR\n\n\
    {{facility_lines}}\n\
    Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and fill \
    out all appropriate areas of the staffing data.\n\
    ───────────────────────────\n\
    {{ntmo_ping}} please react with your availability to NOM for this event. {{dcc_ping}} please \
    react with your availability to shadow this event.\n\n\
    🟢 = Available\n🟡 = Partially available/unsure\n🔴 = Unavailable\n\
    ───────────────────────────";

/// Substitute each `{{key}}` in `template` with its value, in a single left-to-right pass over the
/// *template* text only. A repeated sequential `.replace()` per key would re-scan already-substituted
/// values on every later pass — a value that happens to contain literal `{{other_key}}` text (e.g. an
/// event title of "Fly-in {{ntmo_ping}} Weekend") would then get that text rewritten into a real
/// mention by a later iteration, injecting an extra ping the template never asked for at that spot.
/// Scanning the source once and appending substituted values straight to the output — never feeding
/// them back through the scan — makes that class of re-injection structurally impossible. An unknown
/// or malformed `{{...}}` token (typo, or no closing `}}`) is left as literal text.
fn render_template(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        match after_open.find("}}") {
            Some(end) => {
                let key = &after_open[..end];
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after_open[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Cap the one-line-per-facility block specifically, rather than truncating the whole rendered
/// message: `{{ntmo_ping}}`/`{{dcc_ping}}` and the availability legend sit *after* this block in the
/// default template, so a blind tail-truncation of the final content on a long facility list would
/// silently drop the ping section entirely — the ping would never fire, with nothing logged.
/// Truncating this one growable piece up front keeps everything after it in the template intact.
const MAX_FACILITY_LINES_LEN: usize = 1000;

fn cap_facility_lines(facility_lines: String) -> String {
    if facility_lines.chars().count() <= MAX_FACILITY_LINES_LEN {
        return facility_lines;
    }
    format!(
        "{}\n_…and more facilities not shown here (see the event page)._\n",
        truncate(facility_lines.trim_end(), MAX_FACILITY_LINES_LEN)
    )
}

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
    let facility_lines = cap_facility_lines(facility_lines);

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

    let template = str_field(p, "thread_template").unwrap_or(FALLBACK_TEMPLATE);
    let content = render_template(
        template,
        &[
            ("title", title),
            ("date_line", date_line),
            ("facility_lines", &facility_lines),
            ("ntmo_ping", &ntmo_ping),
            ("dcc_ping", &dcc_ping),
        ],
    );
    // Discord rejects a message over 2000 chars outright; the template is capped on save, but the
    // per-event facility_lines block can still push a long template over the edge, so truncate the
    // final rendered content as a hard safety net rather than let send_message fail wholesale.
    let content = truncate(&content, 2000);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_template_substitutes_every_placeholder() {
        let out = render_template(
            "Hi {{name}}, your event is {{when}}.",
            &[("name", "Alex"), ("when", "Saturday")],
        );
        assert_eq!(out, "Hi Alex, your event is Saturday.");
    }

    /// The fallback template, rendered, must match exactly what the old hard-coded `format!` call
    /// produced for the same inputs — this is the regression check for replacing that call.
    #[test]
    fn fallback_template_matches_the_old_hard_coded_output() {
        let out = render_template(
            FALLBACK_TEMPLATE,
            &[
                ("title", "Test Event"),
                ("date_line", "Sat, Jan 1 · 1200z"),
                ("facility_lines", "• **ZDC** <@111>\n"),
                ("ntmo_ping", "<@&222>"),
                ("dcc_ping", "<@&333>"),
            ],
        );
        let expected = "**Test Event | Planning Thread**\n\
            Test Event is on Sat, Jan 1 · 1200z\n\n\
            Review the following for your facility:\n\
            - TMU/TMI package\n\
            - Staffing\n\
            - Configs and AAR\n\n\
            • **ZDC** <@111>\n\n\
            Attempt to coordinate as many plans (initiatives, reroutes, etc.) in a timely manner, and \
            fill out all appropriate areas of the staffing data.\n\
            ───────────────────────────\n\
            <@&222> please react with your availability to NOM for this event. <@&333> please \
            react with your availability to shadow this event.\n\n\
            🟢 = Available\n🟡 = Partially available/unsure\n🔴 = Unavailable\n\
            ───────────────────────────";
        assert_eq!(out, expected);
    }

    /// A value that happens to contain literal `{{other_key}}` text must not be re-scanned and
    /// substituted again — that would let an event title like "Fly-in {{ntmo_ping}} Weekend" inject
    /// an extra role ping the template's own placeholder slot didn't ask for at that position. The
    /// old sequential-`.replace()` implementation was vulnerable to exactly this; a single pass over
    /// the template text (never re-scanning already-substituted output) is not.
    #[test]
    fn a_substituted_value_containing_placeholder_syntax_is_not_re_substituted() {
        let out = render_template(
            "Title: {{title}} Ping: {{ntmo_ping}}",
            &[
                ("title", "Fly-in {{ntmo_ping}} Weekend"),
                ("ntmo_ping", "<@&222>"),
            ],
        );
        assert_eq!(out, "Title: Fly-in {{ntmo_ping}} Weekend Ping: <@&222>");
    }

    /// An unrecognized `{{...}}` token (a typo, or a placeholder that doesn't exist) is left as
    /// literal text rather than panicking or silently dropping it.
    #[test]
    fn unknown_placeholder_is_left_literal() {
        let out = render_template("Hi {{typo}}!", &[("name", "Alex")]);
        assert_eq!(out, "Hi {{typo}}!");
    }

    /// A `{{` with no matching closing `}}` doesn't panic on the slice arithmetic.
    #[test]
    fn unclosed_placeholder_does_not_panic() {
        let out = render_template("Hi {{name", &[("name", "Alex")]);
        assert_eq!(out, "Hi {{name");
    }

    #[test]
    fn cap_facility_lines_leaves_a_short_list_untouched() {
        let lines = "• **ZDC** <@111>\n• **ZNY** <@222>\n".to_string();
        assert_eq!(cap_facility_lines(lines.clone()), lines);
    }

    /// A long facility list is clipped rather than left to grow the final rendered content past
    /// Discord's 2000-char limit — the regression this guards is the ping section (which sits after
    /// `{{facility_lines}}` in the template) getting silently truncated away along with the overflow.
    #[test]
    fn cap_facility_lines_clips_an_oversized_list_and_notes_the_clip() {
        let one_line = "• **ZDC** <@111>\n";
        let lines: String = one_line.repeat(100); // well over MAX_FACILITY_LINES_LEN
        let capped = cap_facility_lines(lines);
        assert!(capped.chars().count() < one_line.len() * 100);
        assert!(capped.contains("…and more facilities not shown here"));
    }

    /// End-to-end regression check for the actual bug: with an oversized facility list rendered into
    /// the real fallback template and then run through the same final 2000-char safety-net truncate
    /// `create_event_thread` applies, the ping section (which sits *after* `{{facility_lines}}`) must
    /// still survive. Before `cap_facility_lines` existed, a large enough facility list pushed the
    /// total rendered length past 2000 chars and the blind tail-truncate silently cut the pings off.
    #[test]
    fn ping_section_survives_an_oversized_facility_list_after_final_truncation() {
        let huge_facility_lines = "• **ZDC** <@111111111111111111>\n".repeat(100); // ~3300 chars raw
        let capped = cap_facility_lines(huge_facility_lines);
        let rendered = render_template(
            FALLBACK_TEMPLATE,
            &[
                ("title", "Fall Fly-In"),
                ("date_line", "Sat, Jan 1 · 1200z"),
                ("facility_lines", &capped),
                ("ntmo_ping", "<@&222>"),
                ("dcc_ping", "<@&333>"),
            ],
        );
        let final_content = truncate(&rendered, 2000);
        assert!(final_content.chars().count() <= 2000);
        assert!(
            final_content.contains("<@&222>") && final_content.contains("<@&333>"),
            "ping section was cut off by final truncation: {final_content:?}"
        );
    }
}

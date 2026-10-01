use std::sync::Arc;

use serde_json::{Value, json};
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateAllowedMentions, CreateButton, CreateMessage, Http,
};

use crate::util::{ADV_STRUCTURED_PREFIX, channel, str_field};

/// Discord rejects a message over this outright. The fence costs 8 more characters
/// (```` ```\n ```` + ```` \n``` ````), which [`fenced_chunks`] accounts for.
const MESSAGE_LIMIT: usize = 2000;

/// The rendered advisory document from a job payload.
///
/// A hard error, not a placeholder, for the same reason `tmi::ntml_row` is: the document is the
/// backend's to produce (`advisory::render_reroute` / `render_cancellation`), and posting a stand-in
/// would *succeed*, so `ack_job` would mark it `succeeded` — a wrong document in the channel,
/// indistinguishable from a right one.
fn document(p: &Value) -> Result<&str, String> {
    str_field(p, "document").ok_or_else(|| "payload carries no rendered document".to_string())
}

/// Replaces characters that would escape the code block the document is about to be wrapped in.
///
/// Unlike an NTML row, newlines are **meaningful here** — the document is multi-line and its
/// alignment is its content — so only the fence character is neutralised. An advisory's free-text
/// lines (`REMARKS`, `ASSOCIATED RESTRICTIONS`, `MODIFICATIONS`) are typed by a controller and
/// filtered nowhere, so a backtick run would otherwise close the fence early and render the rest of
/// the document as live Discord markdown.
fn fence_safe(document: &str) -> String {
    document.replace('`', "'")
}

/// Splits a document into code-block-wrapped messages, each under Discord's limit.
///
/// **Whole lines only.** The document is a fixed-width document whose alignment carries meaning — a
/// route table's `ORIG`/`DEST`/`ROUTE` columns line up per line — so a split inside a line silently
/// corrupts it. That is also why `util::truncate` is not used here: the event-thread job can afford to
/// clip its tail, and an advisory cannot.
///
/// Each chunk is fenced independently, so every message is a valid block on its own rather than the
/// first opening a fence that a later one closes.
///
/// A single line longer than the limit is emitted in a chunk of its own and left over-long rather than
/// broken. Discord will reject it, the job will retry and park as `failed`, and the reason will name
/// the advisory — which is the right outcome: a line that long means the renderer produced something
/// no split could have made postable, and quietly mangling it would hide that.
fn fenced_chunks(document: &str, limit: usize) -> Vec<String> {
    const FENCE: usize = "```\n\n```".len();
    let budget = limit.saturating_sub(FENCE);
    let safe = fence_safe(document);

    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in safe.lines() {
        // +1 for the newline joining it to what is already there.
        let added = if current.is_empty() {
            line.len()
        } else {
            current.len() + 1 + line.len()
        };
        if !current.is_empty() && added > budget {
            out.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out.into_iter().map(|c| format!("```\n{c}\n```")).collect()
}

pub(crate) async fn post_advisory(http: &Arc<Http>, p: &Value) -> Result<Option<Value>, String> {
    let channel = channel(p)?;
    // The backend assembles the whole document; the bot fences and splits it and decides nothing
    // about its content.
    let document = document(p)?;
    let advisory_id = str_field(p, "advisory_id");

    let chunks = fenced_chunks(document, MESSAGE_LIMIT);
    let last = chunks.len() - 1;
    let mut first_id: Option<String> = None;
    for (i, content) in chunks.iter().enumerate() {
        // The button goes on the last message only: it is the one a reader ends at, and one button
        // per chunk would be noise on a long document.
        let message = build_message(content, advisory_id.filter(|_| i == last));
        let sent = channel
            .send_message(http, message)
            .await
            .map_err(|e| format!("send_message failed: {e}"))?;
        if first_id.is_none() {
            first_id = Some(sent.id.get().to_string());
        }
    }
    Ok(Some(json!({ "message_id": first_id })))
}

/// The message as it will be sent. Split out from [`post_advisory`] so the hardening below can be
/// asserted — it is otherwise only reachable through a live Discord connection.
fn build_message(content: &str, advisory_id: Option<&str>) -> CreateMessage {
    let mut message = CreateMessage::new()
        .content(content)
        // An advisory's remarks and impacting-condition lines are free text a controller typed, and
        // plain message content IS parsed for mentions by default — so an accidental "@everyone" in
        // an advisory would mass-ping the channel without this.
        .allowed_mentions(CreateAllowedMentions::new());
    if let Some(advisory_id) = advisory_id {
        let button = CreateButton::new(format!("{ADV_STRUCTURED_PREFIX}{advisory_id}"))
            .label("View structured")
            .style(ButtonStyle::Secondary);
        message = message.components(vec![CreateActionRow::Buttons(vec![button])]);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(chunks: &[String]) -> Vec<String> {
        chunks
            .iter()
            .flat_map(|c| {
                c.trim_start_matches("```\n")
                    .trim_end_matches("\n```")
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn a_short_document_is_one_fenced_block() {
        let chunks = fenced_chunks("LINE ONE\nLINE TWO", MESSAGE_LIMIT);
        assert_eq!(chunks, vec!["```\nLINE ONE\nLINE TWO\n```".to_string()]);
    }

    /// A backtick run in free text would otherwise close the fence early and render the rest of the
    /// document as live markdown.
    #[test]
    fn a_backtick_cannot_break_out_of_the_block() {
        let chunks = fenced_chunks("REMARKS: ```see below", MESSAGE_LIMIT);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].matches("```").count(), 2, "{}", chunks[0]);
    }

    /// Newlines are content here, unlike an NTML row — the document's alignment is its meaning.
    #[test]
    fn newlines_survive() {
        assert!(fenced_chunks("A\nB\nC", MESSAGE_LIMIT)[0].contains("A\nB\nC"));
    }

    // --- splitting at the boundary (VATUSA/OIS#459 AC2) ---

    /// A document that fits exactly must not split: an off-by-one here posts a stray second message
    /// containing nothing but a fence.
    #[test]
    fn a_document_exactly_on_the_limit_stays_one_message() {
        const FENCE: usize = "```\n\n```".len();
        let doc = "X".repeat(MESSAGE_LIMIT - FENCE);
        let chunks = fenced_chunks(&doc, MESSAGE_LIMIT);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), MESSAGE_LIMIT);
    }

    #[test]
    fn one_character_over_splits_into_two() {
        const FENCE: usize = "```\n\n```".len();
        // Two lines that together exceed the budget by one.
        let half = (MESSAGE_LIMIT - FENCE) / 2;
        let doc = format!("{}\n{}", "A".repeat(half), "B".repeat(half + 1));
        let chunks = fenced_chunks(&doc, MESSAGE_LIMIT);
        assert_eq!(chunks.len(), 2, "should have split");
        for c in &chunks {
            assert!(
                c.len() <= MESSAGE_LIMIT,
                "chunk over the limit: {}",
                c.len()
            );
        }
    }

    /// The property that matters: splitting never divides a line, and loses none.
    #[test]
    fn splitting_preserves_every_line_whole_and_in_order() {
        let doc: String = (0..200)
            .map(|i| format!("ORIG{i:03}      DEST      >SOME ROUTE SEGMENT {i:03}<"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = fenced_chunks(&doc, MESSAGE_LIMIT);

        assert!(chunks.len() > 1, "this document should have split");
        assert_eq!(
            lines_of(&chunks),
            doc.lines().collect::<Vec<_>>(),
            "a line was divided, dropped or reordered"
        );
        for c in &chunks {
            assert!(c.len() <= MESSAGE_LIMIT);
            assert_eq!(c.matches("```").count(), 2, "each chunk fences itself");
        }
    }

    /// An over-long single line is left intact rather than broken — see [`fenced_chunks`].
    #[test]
    fn an_unsplittable_line_is_not_mangled() {
        let doc = "Z".repeat(MESSAGE_LIMIT * 2);
        let chunks = fenced_chunks(&doc, MESSAGE_LIMIT);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains(&doc));
    }

    // --- the payload contract ---

    #[test]
    fn a_missing_document_fails_rather_than_posting_a_placeholder() {
        let err = document(&json!({"channel_id": "1"})).unwrap_err();
        assert!(err.contains("no rendered document"), "{err}");
    }

    /// Copied from the TMI job: plain message content is parsed for mentions by default, so an
    /// advisory's free text could otherwise mass-ping the channel.
    #[test]
    fn an_at_everyone_in_an_advisory_cannot_ping_the_channel() {
        let content = "```\nREMARKS: @everyone see this\n```";
        let value = serde_json::to_value(build_message(content, None)).unwrap();

        let parse = value
            .get("allowed_mentions")
            .and_then(|m| m.get("parse"))
            .and_then(Value::as_array)
            .expect("allowed_mentions.parse must be set");
        assert!(parse.is_empty(), "nothing may be parsed as a mention");
        // Not vacuous: the text really does still contain the trigger.
        assert!(
            value
                .get("content")
                .and_then(Value::as_str)
                .unwrap()
                .contains("@everyone"),
            "the test would pass for the wrong reason if the content were sanitised instead"
        );
    }

    #[test]
    fn the_button_is_attached_only_with_an_advisory_id() {
        let with = serde_json::to_value(build_message("x", Some("adv-1"))).unwrap();
        assert!(
            with.get("components")
                .is_some_and(|c| !c.as_array().unwrap().is_empty())
        );

        let without = serde_json::to_value(build_message("x", None)).unwrap();
        assert!(
            without
                .get("components")
                .is_none_or(|c| c.as_array().is_none_or(|a| a.is_empty()))
        );
    }
}

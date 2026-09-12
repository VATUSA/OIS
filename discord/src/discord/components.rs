use serenity::all::{
    ButtonStyle, CreateActionRow, CreateButton, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};

use crate::util::ACE_CLAIM_PREFIX;

/// The "Claim" button on the ACE request embed. Shared by the initial post (0/N) and the
/// claim/release notify (re-adds the button while slots remain).
pub(crate) fn claim_button(request_id: &str) -> CreateButton {
    CreateButton::new(format!("{ACE_CLAIM_PREFIX}{request_id}"))
        .label("Claim")
        .style(ButtonStyle::Primary)
}

/// The claim picker: Start + End dropdowns (Zulu HHMM) and a Claim button. All three custom_ids carry
/// the current `(start, end)` so any change re-renders with the state intact; the button enables once
/// both are chosen.
pub(crate) fn claim_components(
    request_id: &str,
    options: &[String],
    start: &str,
    end: &str,
) -> Vec<CreateActionRow> {
    let menu = |tag: char, placeholder: &str, chosen: &str| {
        let opts: Vec<CreateSelectMenuOption> = options
            .iter()
            .map(|o| {
                CreateSelectMenuOption::new(format!("{o}z"), o.clone())
                    .default_selection(o == chosen)
            })
            .collect();
        CreateActionRow::SelectMenu(
            CreateSelectMenu::new(
                format!("ace{tag}:{request_id}:{start}:{end}"),
                CreateSelectMenuKind::String { options: opts },
            )
            .placeholder(placeholder),
        )
    };
    let ready = start != "-" && end != "-";
    let confirm = CreateButton::new(format!("aceG:{request_id}:{start}:{end}"))
        .label("Claim slot")
        .style(ButtonStyle::Success)
        .disabled(!ready);
    vec![
        menu('S', "Start time (Zulu)", start),
        menu('E', "End time (Zulu)", end),
        CreateActionRow::Buttons(vec![confirm]),
    ]
}

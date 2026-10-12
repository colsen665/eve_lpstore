//! The filters dialog: its rows, the notes under them, and the name suggestions.

use super::*;

/// The width of the form's label column. `pub(crate)` so a test can check that a field's name fits
/// it, which is what lets the market fields be grouped without a layout change.
pub(crate) const LABEL_WIDTH: usize = 14;

pub(super) const VALUE_WIDTH: usize = 26;

/// The filter form, shown in place of the results while it is open. Each field has a row, and the
/// space under the rows holds the notes for the focused field.
pub(super) fn draw_form(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let block = Block::bordered().title(" Filters ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let mut lines: Vec<Line> = FormField::all()
        .iter()
        .map(|&field| form_row(app, field, sdata, width))
        .collect();
    let room = usize::from(inner.height).saturating_sub(lines.len());
    lines.extend(
        form_notes(app, sdata, width, room)
            .into_iter()
            .map(|(text, style)| Line::from(Span::styled(text, style))),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The notes under the field rows: the general error, the focused field's error or else its
/// valid values, and the matches for a name field. Notes beyond `room` are left out.
fn form_notes(app: &App, sdata: &StaticData, width: usize, room: usize) -> Vec<(String, Style)> {
    let red = Style::new().red();
    let dark_gray = Style::new().dark_gray();
    let focus = app.form.focus;
    let error = app.form.error_for(focus);
    let mut notes: Vec<(String, Style)> = Vec::new();
    if let Some(error) = app.form.general_error() {
        notes.extend(wrap_text(error, width).into_iter().map(|row| (row, red)));
    }
    if let Some(error) = &error {
        notes.extend(wrap_text(error, width).into_iter().map(|row| (row, red)));
    }
    if let Some(matches) = suggestions(app, sdata) {
        notes.extend(
            wrap_text(&matches, width)
                .into_iter()
                .map(|row| (row, dark_gray)),
        );
    }
    if error.is_none() {
        let text = hint(focus, sdata);
        notes.extend(
            wrap_text(&text, width)
                .into_iter()
                .map(|row| (row, dark_gray)),
        );
    }
    fit(notes, room, width)
}

fn form_row(app: &App, field: FormField, sdata: &StaticData, width: usize) -> Line<'static> {
    let focused = app.form.focus == field;
    let label_style = if focused {
        Style::new().bold().yellow()
    } else {
        Style::new()
    };
    let value = if field == FormField::Expire {
        if app.form.expire() {
            "[x] yes".to_string()
        } else {
            "[ ] no".to_string()
        }
    } else if focused {
        let mut text = tail(app.form.text(field), VALUE_WIDTH - 1);
        text.push('_');
        text
    } else {
        truncate(app.form.text(field), VALUE_WIDTH - 1)
    };
    let detail_width = width.saturating_sub(LABEL_WIDTH + VALUE_WIDTH + 2);
    let (detail, style) = match app.form.error_for(field) {
        Some(error) => (error, Style::new().red()),
        // Only the first line belongs in the row, which is one line tall. The rest is shown under
        // the focused field, where there is room for it.
        None => (hint_line(field, sdata), Style::new().dark_gray()),
    };
    Line::from(vec![
        Span::styled(
            format!("{:<width$}", field.label(), width = LABEL_WIDTH),
            label_style,
        ),
        Span::raw(format!("{:<width$}", value, width = VALUE_WIDTH)),
        Span::raw("  "),
        Span::styled(truncate(&detail, detail_width), style),
    ])
}

/// The first line of [`hint`], for the row's detail column, which is one line tall.
pub(super) fn hint_line(field: FormField, sdata: &StaticData) -> String {
    hint(field, sdata)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The valid values of a field, shown beside it when it has no error.
pub(super) fn hint(field: FormField, sdata: &StaticData) -> String {
    match field {
        FormField::Text(Field::Lpstore) => {
            "IDs or names, comma-separated. Blank for all.".to_string()
        }
        // Two lines: what the field picks, then how to write it. Kept to what fits the hint column at
        // the minimum terminal width, so the second line is not wrapped away.
        FormField::Text(Field::Region) => {
            "Prices come from its market orders.\nID or name. Blank for The Forge.".to_string()
        }
        FormField::Text(Field::Security) => {
            // Two lines like the market fields: the first has to fit the row's detail column at the
            // minimum terminal width.
            "0.5 exact, 0.5..0.7 for a range.\n0.5.. is at least, ..0.5 is at most. Blank for any."
                .to_string()
        }
        // Two lines like the market fields: what the filter keeps, then how to write it. The names
        // of the two pairs are close enough to mix up, so each line says which side of it this is.
        FormField::Text(Field::AgentRegion) => {
            "Keeps agents standing in it.\nID or name. Blank for any region.".to_string()
        }
        FormField::Text(Field::AgentSystem) => {
            "Keeps agents in this system.\nID or name. Blank for any system.".to_string()
        }
        FormField::Text(Field::AgentLevel) => "1 to 5. Blank for any.".to_string(),
        FormField::Text(Field::AgentDivision) => format!(
            "One of: {}. Blank for any.",
            sdata.division_names().join(", ")
        ),
        FormField::Text(Field::AgentType) => format!(
            "One of: {}. Blank for any.",
            sdata.agent_type_names().join(", ")
        ),
        FormField::Text(Field::Faction) => format!(
            "One of: {}. Blank for any.",
            sdata.faction_names().join(", ")
        ),
        FormField::Text(Field::System) => {
            "Prices use its NPC stations.\nName or ID. Blank for all systems.".to_string()
        }
        FormField::Text(Field::Depth) => "Offers per group, 1 or more. Blank for 10.".to_string(),
        FormField::Text(Field::CacheMinutes) => {
            "Minutes past ESI Expires.\nBlank: filtered corp count; 0 disables.".to_string()
        }
        FormField::Expire => {
            "Space toggles. Ignore cached responses and call the API again.".to_string()
        }
        // Set on the command line only, so the form never shows them.
        FormField::Text(Field::Tasks | Field::MaxInflight | Field::CacheDir) => String::new(),
    }
}

/// Up to five names that contain what is typed into the focused name field. None for the other
/// fields, and while a name field is empty.
pub(super) fn suggestions(app: &App, sdata: &StaticData) -> Option<String> {
    let query = app.form.name_query()?.to_lowercase();
    let mut names: Vec<String> = match app.form.focus {
        FormField::Text(Field::Lpstore) => sdata
            .corporation_ids()
            .map(|id| corp_name(sdata, id))
            .filter(|name| name.to_lowercase().contains(&query))
            .collect(),
        FormField::Text(Field::Region | Field::AgentRegion) => sdata
            .region_names()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect(),
        FormField::Text(Field::System | Field::AgentSystem) => sdata
            .system_names()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    names.sort();
    names.dedup();
    names.truncate(5);
    Some(if names.is_empty() {
        "No matches".to_string()
    } else {
        format!("Matches: {}", names.join(", "))
    })
}

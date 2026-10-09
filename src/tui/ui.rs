//! Drawing the `--tui` view. These functions read the app state and draw it, and they change
//! nothing. `draw` returns how the details pane was laid out, so the app can scroll it.

use super::app::{App, Focus, LogLine, Overlay, Stage};
use super::form::FormField;
use super::qr;
use crate::output;
use eve_lpstore::esi::ClientStats;
use eve_lpstore::format::{format_isk, format_number};
use eve_lpstore::report::Group;
use eve_lpstore::{OfferResult, StaticData};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use std::collections::VecDeque;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 20;

const LABEL_WIDTH: usize = 14;
const VALUE_WIDTH: usize = 26;

const DONATION_ADDRESS: &str = "bc1qyc55ddky2ehdnty2c48nvxxugjw2xdpxk956x7";

/// The offers pane is at least this many columns wide, including its borders. Narrower panes
/// leave out the volume column.
const WIDE_OFFERS: u16 = 66;

/// Draws the whole view. Returns the details pane's (total rows, rows on screen), or (0, 0)
/// when the results are not on screen.
pub fn draw(
    frame: &mut Frame,
    app: &App,
    sdata: &StaticData,
    stats: &ClientStats,
) -> (usize, usize) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let message = format!(
            "Terminal too small: need {}x{}, have {}x{}",
            MIN_WIDTH, MIN_HEIGHT, area.width, area.height
        );
        frame.render_widget(Paragraph::new(message).red(), area);
        return (0, 0);
    }

    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
    .areas(area);
    let corporations = if app.corporations == 1 {
        "corporation"
    } else {
        "corporations"
    };
    let mut summary = format!(
        "{} | {} {} | Filters: {}",
        app.region, app.corporations, corporations, app.filters
    );
    if app.stale {
        summary = format!("{} | {}", app.stage_text(), summary);
    }
    let inner_width = usize::from(header.width.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(truncate(&summary, inner_width))
            .block(Block::bordered().title(" LP store ")),
        header,
    );

    let layout = match app.overlay {
        Overlay::Log => {
            draw_log(frame, body, app);
            (0, 0)
        }
        Overlay::Filters => {
            draw_form(frame, body, app, sdata);
            (0, 0)
        }
        _ if app.report.is_some() => draw_results(frame, body, app, sdata),
        _ => {
            draw_progress(frame, body, app);
            (0, 0)
        }
    };
    if app.overlay == Overlay::Help {
        draw_help(frame, area);
    }
    if app.overlay == Overlay::Donate {
        draw_donate(frame, area);
    }
    draw_footer(frame, footer, stats, app.overlay == Overlay::Filters);
    layout
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    let [stage, fetch, pricing, log] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .areas(area);
    let style = if app.stage == Stage::Failed {
        Style::new().red()
    } else {
        Style::new()
    };
    frame.render_widget(Paragraph::new(app.stage_text()).style(style), stage);

    let ((fetch_done, fetch_total), (price_done, price_total)) = app.gauges();
    gauge(
        frame,
        fetch,
        "Fetching loyalty offers",
        fetch_done,
        fetch_total,
    );
    gauge(frame, pricing, "Pricing groups", price_done, price_total);

    let height = usize::from(log.height.saturating_sub(2));
    let lines = log_lines(&app.log, height, 0);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Recent log (l for all) ")),
        log,
    );
}

fn gauge(frame: &mut Frame, area: Rect, title: &str, done: usize, total: usize) {
    let (ratio, label) = if total == 0 {
        (0.0, "waiting".to_string())
    } else {
        (
            (done as f64 / total as f64).min(1.0),
            format!("{} / {}", done, total),
        )
    };
    let widget = Gauge::default()
        .block(Block::bordered().title(format!(" {} ", title)))
        .gauge_style(Style::new().fg(Color::Cyan))
        .ratio(ratio)
        .label(label);
    frame.render_widget(widget, area);
}

fn draw_log(frame: &mut Frame, area: Rect, app: &App) {
    let height = usize::from(area.height.saturating_sub(2));
    let lines = log_lines(&app.log, height, app.log_scroll);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Log (l or Esc to close) ")),
        area,
    );
}

/// The newest `height` lines, skipping the `scroll` newest. Errors are red.
fn log_lines(log: &VecDeque<LogLine>, height: usize, scroll: usize) -> Vec<Line<'static>> {
    let end = log.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    log.range(start..end)
        .map(|line| {
            let style = if line.is_error {
                Style::new().red()
            } else {
                Style::new()
            };
            Line::from(Span::styled(line.text.clone(), style))
        })
        .collect()
}

/// The filter form, shown in place of the results while it is open. Each field has a row, and the
/// space under the rows holds the notes for the focused field.
fn draw_form(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let block = Block::bordered().title(" Filters ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let mut lines: Vec<Line> = FormField::ALL
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

/// Keeps the first `room` notes. When some are cut, the last one kept ends with an ellipsis.
fn fit(mut notes: Vec<(String, Style)>, room: usize, width: usize) -> Vec<(String, Style)> {
    if notes.len() > room {
        notes.truncate(room);
        if let Some((text, _)) = notes.last_mut() {
            *text = truncate(&format!("{}…", text), width);
        }
    }
    notes
}

fn form_row(app: &App, field: FormField, sdata: &StaticData, width: usize) -> Line<'static> {
    let focused = app.form.focus == field;
    let label_style = if focused {
        Style::new().bold().yellow()
    } else {
        Style::new()
    };
    let value = if field == FormField::Expire {
        if app.form.expire {
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
        None => (hint(field, sdata), Style::new().dark_gray()),
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

/// The valid values of a field, shown beside it when it has no error.
fn hint(field: FormField, sdata: &StaticData) -> String {
    match field {
        FormField::Corporations => "IDs or names, comma-separated. Blank for all.".to_string(),
        FormField::Region => "ID or name. Blank for The Forge.".to_string(),
        FormField::Security => {
            "+0.5 at least, -0.5 at most, =0.5 exactly. Blank for any.".to_string()
        }
        FormField::AgentLevel => "1 to 5. Blank for any.".to_string(),
        FormField::Division => format!(
            "One of: {}. Blank for any.",
            sdata.division_names().join(", ")
        ),
        FormField::Type => format!(
            "One of: {}. Blank for any.",
            sdata.agent_type_names().join(", ")
        ),
        FormField::Faction => format!(
            "One of: {}. Blank for any.",
            sdata.faction_names().join(", ")
        ),
        FormField::System => "Name or ID. Blank for all systems.".to_string(),
        FormField::Depth => "Offers per group, 1 or more. Blank for 10.".to_string(),
        FormField::Expire => {
            "Space toggles. Ignore cached responses and call the API again.".to_string()
        }
    }
}

/// Up to five names that contain what is typed into the focused name field. None for the other
/// fields, and while a name field is empty.
fn suggestions(app: &App, sdata: &StaticData) -> Option<String> {
    let query = app.form.name_query()?.to_lowercase();
    let mut names: Vec<String> = match app.form.focus {
        FormField::Corporations => sdata
            .corporation_ids()
            .map(|id| corp_name(sdata, id))
            .filter(|name| name.to_lowercase().contains(&query))
            .collect(),
        FormField::Region => sdata
            .region_names()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect(),
        FormField::System => sdata
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

fn draw_results(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) -> (usize, usize) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).areas(area);
    let [offers, details] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Min(0)]).areas(right);
    draw_groups(frame, left, app, sdata);
    draw_offers(frame, offers, app, sdata);
    draw_details(frame, details, app, sdata)
}

/// A bordered pane. Its title says so when the results it shows are stale.
fn pane_block(name: &str, focused: bool, app: &App) -> Block<'static> {
    let title = if app.stale {
        format!(" {} (stale) ", name)
    } else {
        format!(" {} ", name)
    };
    let color = if focused {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    Block::bordered()
        .title(title)
        .border_style(Style::new().fg(color))
}

fn draw_groups(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let block = pane_block("Groups", app.focus == Focus::Groups, app);
    let groups = app.groups();
    if groups.is_empty() {
        frame.render_widget(
            Paragraph::new("No loyalty offers were found.").block(block),
            area,
        );
        return;
    }
    // Inside the borders, less the two columns of the highlight symbol.
    let names_width = usize::from(area.width.saturating_sub(4));
    let items: Vec<ListItem> = groups
        .iter()
        .map(|group| group_item(group, sdata, names_width))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(app.group));
    frame.render_stateful_widget(list, area, &mut state);
}

/// A group takes two lines: its ISK per LP and offer count, then its corporations.
fn group_item(group: &Group, sdata: &StaticData, names_width: usize) -> ListItem<'static> {
    let isk = group
        .results
        .first()
        .and_then(|result| result.isk_per_lp)
        .map(|value| format_number(value, 2))
        .unwrap_or_else(|| "n/a".to_string());
    let names: Vec<String> = group
        .corp_ids
        .iter()
        .map(|&id| corp_name(sdata, id))
        .collect();
    ListItem::new(vec![
        Line::from(format!("{} ISK/LP | {} offers", isk, group.offer_count)),
        Line::from(Span::styled(
            truncate(&names.join(", "), names_width),
            Style::new().dark_gray(),
        )),
    ])
}

/// Cuts `text` to `width` characters, ending a cut with an ellipsis.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{}…", kept)
}

/// The last `width` characters of `text`, with an ellipsis when the start is cut.
fn tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let kept: String = text.chars().skip(count - width.saturating_sub(1)).collect();
    format!("…{}", kept)
}

fn corp_name(sdata: &StaticData, corp_id: i32) -> String {
    sdata
        .corporation_name(corp_id)
        .cloned()
        .unwrap_or_else(|| corp_id.to_string())
}

fn item_name(sdata: &StaticData, type_id: i32) -> String {
    sdata
        .type_name(type_id)
        .cloned()
        .unwrap_or_else(|| format!("Type {}", type_id))
}

fn draw_offers(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let block = pane_block("Offers", app.focus == Focus::Offers, app);
    let Some(group) = app.selected_group() else {
        frame.render_widget(Paragraph::new("").block(block), area);
        return;
    };
    let wide = area.width >= WIDE_OFFERS;
    let rows: Vec<Row> = group
        .results
        .iter()
        .take(app.depth)
        .enumerate()
        .map(|(rank, result)| offer_row(rank, result, sdata, wide))
        .collect();
    let (titles, widths) = if wide {
        (
            vec!["#", "Item", "LP", "ISK/LP", "Vol ±10%", "Profit ±10%"],
            vec![
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(10),
                Constraint::Length(11),
            ],
        )
    } else {
        (
            vec!["#", "Item", "LP", "ISK/LP", "Profit ±10%"],
            vec![
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(11),
            ],
        )
    };
    let header = Row::new(titles).style(Style::new().bold());
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = TableState::default().with_selected(Some(app.offer));
    frame.render_stateful_widget(table, area, &mut state);
}

fn offer_row(rank: usize, result: &OfferResult, sdata: &StaticData, wide: bool) -> Row<'static> {
    let item = item_name(sdata, result.offer.type_id);
    Row::new(offer_cells(rank, result, item, wide))
}

/// The cells of an offer row. The volume within 10% of the best bid is left out when `wide` is
/// false, and the profit is the total on that volume.
fn offer_cells(rank: usize, result: &OfferResult, item: String, wide: bool) -> Vec<String> {
    let isk_per_lp = result
        .isk_per_lp
        .map(|value| format_number(value, 2))
        .unwrap_or_else(|| "n/a".to_string());
    let (volume, profit) = match result.within_10 {
        Some((volume, profit)) => (format_number(volume as f64, 0), short_isk(profit)),
        None => ("n/a".to_string(), "n/a".to_string()),
    };
    let mut cells = vec![
        (rank + 1).to_string(),
        item,
        format_number(result.offer.lp_cost as f64, 0),
        isk_per_lp,
    ];
    if wide {
        cells.push(volume);
    }
    cells.push(profit);
    cells
}

/// ISK in about eight characters: billions, millions, or thousands, with two decimals.
fn short_isk(value: f64) -> String {
    let magnitude = value.abs();
    let (scaled, suffix) = if magnitude >= 1e9 {
        (value / 1e9, "B")
    } else if magnitude >= 1e6 {
        (value / 1e6, "M")
    } else if magnitude >= 1e3 {
        (value / 1e3, "K")
    } else {
        return format_number(value, 2);
    };
    format!("{:.2}{}", scaled, suffix)
}

/// Draws the details pane, wrapped to its width, and returns (rows, rows on screen).
fn draw_details(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) -> (usize, usize) {
    let block = pane_block("Details", app.focus == Focus::Details, app);
    let width = usize::from(area.width.saturating_sub(2)).max(1);
    let visible = usize::from(area.height.saturating_sub(2));
    let lines: Vec<Line<'static>> = detail_entries(app, sdata)
        .iter()
        .flat_map(|(text, style)| wrap(text, width, *style))
        .collect();
    let rows = lines.len();
    let scroll = app.detail_scroll.min(rows.saturating_sub(visible));
    let scroll = u16::try_from(scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
    (rows, visible)
}

/// Splits `text` into rows of at most `width` characters, breaking at spaces where it can. A
/// word longer than a row is cut into pieces.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let len = word.chars().count();
        let joined = if current.is_empty() {
            len
        } else {
            current.chars().count() + 1 + len
        };
        if joined <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        let chars: Vec<char> = word.chars().collect();
        let mut pieces = chars.chunks(width).peekable();
        while let Some(piece) = pieces.next() {
            let piece: String = piece.iter().collect();
            if pieces.peek().is_some() {
                rows.push(piece);
            } else {
                current = piece;
            }
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

/// The rows of `wrap_text`, each styled.
fn wrap(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_text(text, width)
        .into_iter()
        .map(|row| Line::from(Span::styled(row, style)))
        .collect()
}

/// The text of the details pane, one entry per logical line. `wrap` turns each entry into one
/// or more screen rows.
fn detail_entries(app: &App, sdata: &StaticData) -> Vec<(String, Style)> {
    let plain = Style::new();
    let bold = Style::new().bold();
    let Some(group) = app.selected_group() else {
        return Vec::new();
    };
    let mut lines = vec![("Corporations".to_string(), bold)];
    for &id in &group.corp_ids {
        lines.push((output::corp_line(sdata, id, &app.agent_filter), plain));
    }
    lines.push((String::new(), plain));

    match app.selected_offer() {
        Some(result) => {
            lines.push((
                format!("Offer: {}", item_name(sdata, result.offer.type_id)),
                bold,
            ));
            lines.push((
                format!(
                    "LP cost: {} | Units per redemption: {}",
                    format_number(result.offer.lp_cost as f64, 0),
                    format_number(result.offer.quantity as f64, 0)
                ),
                plain,
            ));
            lines.push((
                format!(
                    "Reward: {} | Unit cost: {} | Profit per unit: {}",
                    optional(result.reward),
                    optional(result.unit_cost),
                    match (result.reward, result.unit_cost) {
                        (Some(reward), Some(unit_cost)) => format_number(reward - unit_cost, 2),
                        _ => "n/a".to_string(),
                    }
                ),
                plain,
            ));
            lines.push((
                format!(
                    "ISK/LP: {} | ISK cost: {} | Total ISK: {}",
                    optional(result.isk_per_lp),
                    format_number(result.breakdown.isk_cost, 2),
                    format_number(result.breakdown.total_isk, 2)
                ),
                plain,
            ));
            lines.push((
                match result.within_10 {
                    Some((volume, profit)) => format!(
                        "Volume within 10% of best bid: {}, total profit: {}",
                        format_number(volume as f64, 0),
                        format_isk(profit)
                    ),
                    None => "Volume within 10% of best bid: n/a, total profit: n/a".to_string(),
                },
                plain,
            ));
            lines.push(("Items:".to_string(), plain));
            for item in &result.breakdown.item_expenses {
                lines.push((
                    format!(
                        "  {}: {} x {} ISK = {} ISK",
                        item_name(sdata, item.type_id),
                        format_number(item.quantity as f64, 0),
                        format_number(item.best_price, 2),
                        format_number(item.total_cost, 2)
                    ),
                    plain,
                ));
            }
        }
        None => lines.push(("No offers were priced for this group.".to_string(), plain)),
    }
    lines
}

fn optional(value: Option<f64>) -> String {
    value
        .map(|value| format_number(value, 2))
        .unwrap_or_else(|| "n/a".to_string())
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let rows = [
        ("Tab", "switch between groups, offers, and details"),
        (
            "Up, Down, j, k",
            "move the selection, or scroll the details",
        ),
        ("PgUp, PgDn", "move a page at a time"),
        ("f", "edit the filters; Enter runs them, Esc cancels"),
        ("d", "show the donation address as a QR code"),
        ("l", "show or hide the full log"),
        ("?", "show or hide this help"),
        ("Esc", "close the log or this help"),
        ("q, Ctrl+C", "quit, stopping the run if it is still going"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(key, text)| Line::from(format!("{:<16}{}", key, text)))
        .collect();
    let popup = area.centered(
        Constraint::Length(70),
        Constraint::Length(lines.len() as u16 + 2),
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Keys ")),
        popup,
    );
}

/// The donation popup: the address as a QR code, with the address as text under it. When the
/// terminal is too small for the code, the popup shows only the address and a note.
fn draw_donate(frame: &mut Frame, area: Rect) {
    let grid = qr::modules(DONATION_ADDRESS);
    let code_rows = grid.len() / 2;
    let width = grid[0].len().max(DONATION_ADDRESS.len()) + 2;
    let height = code_rows + 4;
    let (popup, lines) = if usize::from(area.width) >= width && usize::from(area.height) >= height {
        let popup = area.centered(
            Constraint::Length(width as u16),
            Constraint::Length(height as u16),
        );
        let mut lines = qr_lines(&grid);
        lines.push(Line::from(DONATION_ADDRESS));
        lines.push(Line::from("Esc or d closes this").dark_gray());
        (popup, lines)
    } else {
        let popup = area.centered(
            Constraint::Length(width.min(usize::from(area.width)) as u16),
            Constraint::Length(4),
        );
        let inner = usize::from(popup.width.saturating_sub(2));
        let note = format!(
            "Enlarge the terminal to {}x{} for the QR code",
            width, height
        );
        let lines = vec![
            Line::from(truncate(DONATION_ADDRESS, inner)),
            Line::from(truncate(&note, inner)).dark_gray(),
        ];
        (popup, lines)
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(Block::bordered().title(" Donate ")),
        popup,
    );
}

/// The code as rows of half-block characters. Each character shows two module rows: the top
/// module is its foreground and the bottom module its background. Dark modules are black and
/// light modules white, so the code scans whatever colors the terminal uses.
fn qr_lines(grid: &[Vec<bool>]) -> Vec<Line<'static>> {
    let color = |dark: bool| {
        if dark {
            Color::Rgb(0, 0, 0)
        } else {
            Color::Rgb(255, 255, 255)
        }
    };
    grid.chunks(2)
        .map(|pair| {
            let spans: Vec<Span<'static>> = (0..pair[0].len())
                .map(|x| {
                    Span::styled(
                        "▀",
                        Style::new().fg(color(pair[0][x])).bg(color(pair[1][x])),
                    )
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

fn draw_footer(frame: &mut Frame, area: Rect, stats: &ClientStats, in_form: bool) {
    let keys = if in_form {
        "Tab/Shift+Tab field  Enter apply  Esc cancel  Ctrl+U clear  Ctrl+C quit"
    } else {
        "Tab panes  Up/Down move  f filters  d donate  l log  ? help  q quit"
    };
    let lines = vec![
        Line::from(format!(
            "In-flight: {} | Waiting: {} | Rate limit: {} | Errors: {}",
            stats.in_flight,
            stats.waiting,
            output::rate_limit_text(stats),
            stats.total_errors
        )),
        Line::from(format!(
            "Cache: {} hits, {} misses, {} writes",
            stats.cache_hits, stats.cache_misses, stats.cache_writes
        )),
        Line::from(keys).dark_gray(),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_lpstore::ExpenseBreakdown;
    use eve_lpstore::esi::loyalty::LoyaltyOffer;

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn wrap_breaks_at_spaces_and_cuts_long_words() {
        let rows = wrap("Ministry of Assessment (1000081): Turba", 20, Style::new());
        assert_eq!(
            texts(&rows),
            ["Ministry of", "Assessment", "(1000081): Turba"]
        );
        let cut = wrap("abcdefghij", 4, Style::new());
        assert_eq!(texts(&cut), ["abcd", "efgh", "ij"]);
        assert_eq!(texts(&wrap("", 10, Style::new())), [""]);
    }

    #[test]
    fn truncate_ends_a_cut_with_an_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    fn priced_offer(within_10: Option<(i64, f64)>) -> OfferResult {
        OfferResult {
            offer: LoyaltyOffer {
                offer_id: 1,
                type_id: 30486,
                quantity: 10,
                lp_cost: 1800,
                isk_cost: 1_200_000.0,
                required_items: Vec::new(),
            },
            breakdown: ExpenseBreakdown {
                offer_id: 1,
                lp_cost: 1800,
                isk_cost: 1_200_000.0,
                item_expenses: Vec::new(),
                total_isk: 1_649_000.0,
            },
            reward: Some(457_900.0),
            unit_cost: Some(164_900.0),
            isk_per_lp: Some(1627.78),
            within_10,
        }
    }

    #[test]
    fn short_isk_uses_one_suffix_and_two_decimals() {
        assert_eq!(short_isk(3_980_000_000.0), "3.98B");
        assert_eq!(short_isk(1_234_000.0), "1.23M");
        assert_eq!(short_isk(5_000.0), "5.00K");
        assert_eq!(short_isk(512.0), "512.00");
        assert_eq!(short_isk(-2_500_000.0), "-2.50M");
    }

    #[test]
    fn wide_offer_rows_show_the_volume_and_the_profit_within_10_percent() {
        let result = priced_offer(Some((2_474_600, 3_980_000_000.0)));
        let cells = offer_cells(0, &result, "Sisters Combat Scanner Probe".to_string(), true);
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[4], "2,474,600");
        assert_eq!(cells[5], "3.98B");
    }

    #[test]
    fn narrow_offer_rows_leave_out_the_volume() {
        let result = priced_offer(Some((2_474_600, 3_980_000_000.0)));
        let cells = offer_cells(0, &result, "Probe".to_string(), false);
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[4], "3.98B");
    }

    #[test]
    fn offers_without_a_reward_show_n_a() {
        let result = priced_offer(None);
        let cells = offer_cells(0, &result, "Probe".to_string(), true);
        assert_eq!(cells[4], "n/a");
        assert_eq!(cells[5], "n/a");
    }

    fn screen_text(width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw_donate(frame, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn the_donation_popup_shows_the_code_and_the_address_when_it_fits() {
        let text = screen_text(80, 24);
        assert!(text.contains(DONATION_ADDRESS));
        assert!(text.contains('▀'));
        assert!(!text.contains("Enlarge"));
    }

    #[test]
    fn the_donation_popup_falls_back_to_the_address_when_the_code_does_not_fit() {
        let text = screen_text(80, 20);
        assert!(text.contains(DONATION_ADDRESS));
        assert!(text.contains("Enlarge the terminal"));
    }

    #[test]
    fn tail_keeps_the_end_of_long_text() {
        assert_eq!(tail("abcdef", 4), "…def");
        assert_eq!(tail("abc", 4), "abc");
    }

    #[test]
    fn notes_past_the_room_are_cut_with_an_ellipsis() {
        let note = |text: &str| (text.to_string(), Style::new());
        let kept = fit(vec![note("one"), note("two"), note("three")], 2, 10);
        assert_eq!(kept, vec![note("one"), note("two…")]);
        assert_eq!(fit(vec![note("one")], 2, 10), vec![note("one")]);
        assert!(fit(vec![note("one")], 0, 10).is_empty());
    }
}

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
    Block, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use std::collections::VecDeque;

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 20;

/// The width of the form's label column. `pub(crate)` so a test can check that a field's name fits
/// it, which is what lets the market fields be grouped without a layout change.
pub(crate) const LABEL_WIDTH: usize = 14;
const VALUE_WIDTH: usize = 26;

const DONATION_ADDRESS: &str = "bc1qyc55ddky2ehdnty2c48nvxxugjw2xdpxk956x7";

/// The offers pane is at least this many columns wide, including its borders, for the wide layout.
/// Narrower panes leave out the imbalance and volume columns. The spread column shows in both.
///
/// This is measured, not derived: at 72 the last column's header is cut, and at 73 it is not. The
/// render tests below are what keep it honest.
const WIDE_OFFERS: u16 = 73;

/// The spread is the fourth offers column, after the rank, the item, and the ISK per LP.
const SPREAD_COLUMN: usize = 3;

/// The imbalance follows the spread, and only exists in the wide layout.
const IMBALANCE_COLUMN: usize = 4;

/// A spread up to this percentage means the market is tight enough to trust the best bid.
const SPREAD_TIGHT: f64 = 10.0;

/// A spread past this percentage means the best bid is probably a single stale order, and the
/// ISK per LP built on it is optimistic.
const SPREAD_WIDE: f64 = 50.0;

/// An imbalance at least this far from balanced is strong enough to colour. Closer to even than
/// this stays yellow, so a book that favours nobody is not reported as a win.
const IMBALANCE_STRONG: f64 = 25.0;

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
    // Before the first run there is no region and no corporation count to report, so the header
    // says that instead of leading with an empty field and a zero.
    let mut summary = if app.stage == Stage::Idle {
        format!("Not started | Filters: {}", app.filters)
    } else {
        format!(
            "{} | {} {} | Filters: {}",
            app.region, app.corporations, corporations, app.filters
        )
    };
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
    if app.overlay == Overlay::Export {
        draw_export(frame, area, app);
    }
    if app.overlay == Overlay::StaticData {
        draw_static_data(frame, area, app);
    }
    draw_footer(frame, footer, stats, app);
    layout
}

/// Where the static data files live, taken from one of them rather than written out again, so this
/// keeps saying the right thing when their location changes.
fn static_data_dir() -> String {
    StaticData::regions_cache_path()
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|| "the data directory".to_string())
}

/// The static data dialog, shown when the files are not on disk, or are and cannot be read.
/// Downloading fetches the whole EVE Online archive, so the view asks before spending the time on
/// it instead of finding out on a terminal that has gone quiet.
fn draw_static_data(frame: &mut Frame, area: Rect, app: &App) {
    let width = 64usize;
    let inner = width - 2;
    let mut lines = Vec::new();
    if app.downloading {
        lines.push(Line::from("Downloading the EVE Online static data…"));
    } else {
        lines.push(Line::from(
            "The EVE Online static data is needed to price loyalty offers.",
        ));
    }
    if let Some(reason) = &app.download_error {
        lines.push(Line::from(truncate(reason, inner)).red());
    }
    lines.push(Line::from(
        truncate(&format!("It is stored in {}.", static_data_dir()), inner).dark_gray(),
    ));
    let keys = if app.downloading {
        "Ctrl+C quits"
    } else {
        "Enter or d downloads it  Esc exits"
    };
    lines.push(Line::from(truncate(keys, inner)).dark_gray());
    let popup = area.centered(
        Constraint::Length(width as u16),
        Constraint::Length(lines.len() as u16 + 2),
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Static data ")),
        popup,
    );
}

/// The export prompt: where the view on screen would be written, and why the last attempt was
/// refused. The results stay behind it, so the name can be read against what is showing. A name
/// too long for the popup shows its end, which is where the letters being typed are.
fn draw_export(frame: &mut Frame, area: Rect, app: &App) {
    let width = 64usize;
    let inner = width - 2;
    let mut lines = vec![
        Line::from("Write the current view to".dark_gray()),
        Line::from(tail(&app.export_path, inner)).bold(),
    ];
    if let Some(error) = &app.export_error {
        lines.push(Line::from(truncate(error, inner)).red());
    }
    lines.push(Line::from(
        truncate(
            "Enter writes it  Esc cancels  Ctrl+U clears the name",
            inner,
        )
        .dark_gray(),
    ));
    let popup = area.centered(
        Constraint::Length(width as u16),
        Constraint::Length(lines.len() as u16 + 2),
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Export ")),
        popup,
    );
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    // Before a run starts there is nothing to gauge, so the two bars that would read "waiting" are
    // left out and the log gets their space.
    let idle = app.stage == Stage::Idle;
    let (stage_area, log_area) = if idle {
        let [stage, log] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        (stage, log)
    } else {
        let [stage, fetch, pricing, log] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);
        let ((fetch_done, fetch_total), (price_done, price_total)) = app.gauges();
        gauge(
            frame,
            fetch,
            "Fetching loyalty offers",
            fetch_done,
            fetch_total,
        );
        gauge(frame, pricing, "Pricing groups", price_done, price_total);
        (stage, log)
    };
    let style = if app.stage == Stage::Failed {
        Style::new().red()
    } else {
        Style::new()
    };
    frame.render_widget(Paragraph::new(app.stage_text()).style(style), stage_area);

    let height = usize::from(log_area.height.saturating_sub(2));
    let lines = log_lines(&app.log, height, 0);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Recent log (l for all) ")),
        log_area,
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

/// The valid values of a field, shown beside it when it has no error.
/// The first line of [`hint`], for the row's detail column, which is one line tall.
fn hint_line(field: FormField, sdata: &StaticData) -> String {
    hint(field, sdata)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string()
}

fn hint(field: FormField, sdata: &StaticData) -> String {
    match field {
        FormField::Corporations => "IDs or names, comma-separated. Blank for all.".to_string(),
        // Two lines: what the field picks, then how to write it. Kept to what fits the hint column at
        // the minimum terminal width, so the second line is not wrapped away.
        FormField::Region => {
            "Prices come from its market orders.\nID or name. Blank for The Forge.".to_string()
        }
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
        FormField::System => {
            "Prices use its NPC stations.\nName or ID. Blank for all systems.".to_string()
        }
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
        .map(|group| group_item(group, app.depth, sdata, names_width))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(app.group));
    frame.render_stateful_widget(list, area, &mut state);
}

/// A group takes two lines: its average ISK per LP over the offers it lists, its offer count,
/// then its corporations.
fn group_item(
    group: &Group,
    depth: usize,
    sdata: &StaticData,
    names_width: usize,
) -> ListItem<'static> {
    let isk = group
        .average_isk_per_lp(depth)
        .map(|value| format_number(value, 2))
        .unwrap_or_else(|| "n/a".to_string());
    let names: Vec<String> = group
        .corp_ids
        .iter()
        .map(|&id| corp_name(sdata, id))
        .collect();
    ListItem::new(vec![
        Line::from(format!("{} avg ISK/LP | {} offers", isk, group.offer_count)),
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
    let (titles, widths) = offer_columns(wide);
    let header = Row::new(titles).style(Style::new().bold());
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = TableState::default().with_selected(Some(app.offer));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The offers table's column titles and widths. The imbalance and volume columns only appear when
/// `wide`, and the spread shows in both. These have to agree in length with `offer_cells`.
fn offer_columns(wide: bool) -> (Vec<&'static str>, Vec<Constraint>) {
    if wide {
        (
            vec![
                "#",
                "Item",
                "ISK/LP",
                "Spread",
                "Imbal",
                "Vol ±10%",
                "Profit ±10%",
            ],
            vec![
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Length(6),
                Constraint::Length(10),
                Constraint::Length(11),
            ],
        )
    } else {
        (
            vec!["#", "Item", "ISK/LP", "Spread", "Profit ±10%"],
            vec![
                Constraint::Length(4),
                // A lower floor than the wide layout, so the fixed columns keep their width at an
                // 80-column terminal. Item is the only column that can shrink.
                Constraint::Min(10),
                Constraint::Length(12),
                Constraint::Length(8),
                Constraint::Length(11),
            ],
        )
    }
}

fn offer_row(rank: usize, result: &OfferResult, sdata: &StaticData, wide: bool) -> Row<'static> {
    let item = item_name(sdata, result.offer.type_id);
    let cells = offer_cells(rank, result, item, wide)
        .into_iter()
        .enumerate()
        .map(|(column, text)| {
            // The imbalance column only exists in the wide layout, where index 4 is the volume.
            let style = match column {
                SPREAD_COLUMN => spread_style(result.spread),
                IMBALANCE_COLUMN if wide => imbalance_style(result.imbalance),
                _ => Style::new(),
            };
            Cell::from(text).style(style)
        })
        .collect::<Vec<_>>();
    Row::new(cells)
}

/// The cells of an offer row. The imbalance and the volume within 10% of the best bid are left out
/// when `wide` is false, and the profit is the total on that volume.
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
        isk_per_lp,
        format_spread(result.spread),
    ];
    if wide {
        cells.push(format_imbalance(result.imbalance));
        cells.push(volume);
    }
    cells.push(profit);
    cells
}

/// The spread as a percentage. An illiquid reward can spread by orders of magnitude, so anything
/// past 999% is capped to hold the column's width.
fn format_spread(spread: Option<f64>) -> String {
    match spread {
        None => "n/a".to_string(),
        Some(value) if value > 999.0 => ">999%".to_string(),
        Some(value) => format!("{:.2}%", value),
    }
}

/// The colour of a spread. Green when the market is tight enough to trust the best bid, yellow as
/// it widens, and red when the best bid is probably a single stale order. Magenta marks an
/// inverted market. A missing side is dim rather than alarming: a reward nobody sells is not the
/// same thing as a bad market.
fn spread_style(spread: Option<f64>) -> Style {
    match spread {
        None => Style::new().dark_gray(),
        Some(value) if value < 0.0 => Style::new().magenta(),
        Some(value) if value <= SPREAD_TIGHT => Style::new().green(),
        Some(value) if value <= SPREAD_WIDE => Style::new().yellow(),
        Some(_) => Style::new().red(),
    }
}

/// The imbalance as a signed whole percentage. The sign always shows, so a bid-heavy book reads as
/// `+73%` rather than looking like a plain magnitude, and a value that rounds to zero shows
/// without a sign instead of as `-0%`.
fn format_imbalance(imbalance: Option<f64>) -> String {
    match imbalance.map(|value| value.round()) {
        None => "n/a".to_string(),
        Some(rounded) if rounded == 0.0 => "0%".to_string(),
        Some(rounded) if rounded > 0.0 => format!("+{:.0}%", rounded),
        Some(rounded) => format!("{:.0}%", rounded),
    }
}

/// The colour of an imbalance. Green when buyers outweigh sellers, which is what an LP holder
/// selling into the market wants, and red when a wall of sellers sits over thin bids. A book too
/// close to even to favour either side stays yellow, and a missing one is dim rather than
/// alarming, since a reward nobody sells is not the same thing as a bad market.
fn imbalance_style(imbalance: Option<f64>) -> Style {
    match imbalance {
        None => Style::new().dark_gray(),
        Some(value) if value >= IMBALANCE_STRONG => Style::new().green(),
        Some(value) if value <= -IMBALANCE_STRONG => Style::new().red(),
        Some(_) => Style::new().yellow(),
    }
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
    // A break in the source text starts a row of its own, so a hint can ask for two lines instead
    // of leaving that to the width. Each of those lines then wraps as usual.
    if let Some((first, rest)) = text.split_once('\n') {
        let mut rows = wrap_text(first, width);
        rows.extend(wrap_text(rest, width));
        return rows;
    }
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
                    "Best ask: {} | Spread: {}",
                    optional(result.best_ask),
                    format_spread(result.spread)
                ),
                spread_style(result.spread),
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
            lines.push((
                match result.book_depth {
                    Some((_, ask)) => format!(
                        "Volume within 10% of best ask: {} | Imbalance: {}",
                        format_number(ask as f64, 0),
                        format_imbalance(result.imbalance)
                    ),
                    None => "Volume within 10% of best ask: n/a | Imbalance: n/a".to_string(),
                },
                imbalance_style(result.imbalance),
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
        ("e", "write the view to a .csv or .json file"),
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

fn draw_footer(frame: &mut Frame, area: Rect, stats: &ClientStats, app: &App) {
    let overlay = app.overlay;
    let keys = match overlay {
        Overlay::Filters => {
            "Tab/Shift+Tab field  Enter apply  Esc cancel  Ctrl+U clear  Ctrl+C quit"
        }
        Overlay::Export => "Enter write  Esc cancel  Ctrl+U clear  Ctrl+C quit",
        Overlay::StaticData => {
            if app.downloading {
                "Downloading...  Ctrl+C quits"
            } else {
                "Enter or d download  Esc exit"
            }
        }
        _ => "Tab panes  Up/Down move  f filters  e export  d donate  l log  ? help  q quit",
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
    use crate::tui::app::RunInfo;
    use crate::tui::form::FilterForm;
    use eve_lpstore::ExpenseBreakdown;
    use eve_lpstore::esi::loyalty::LoyaltyOffer;
    use eve_lpstore::report::{LpStoreOptions, Report};
    use eve_lpstore::static_data::AgentFilter;

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
    fn a_line_break_in_the_source_starts_its_own_row() {
        // The market fields ask for two lines instead of leaving the split to the width.
        assert_eq!(wrap_text("one\ntwo", 20), ["one", "two"]);
        assert_eq!(wrap_text("one\ntwo", 3), ["one", "two"]);
        assert_eq!(wrap_text("a b\nc d", 3), ["a b", "c d"]);
        // A blank line stays a row of its own rather than vanishing.
        assert_eq!(wrap_text("a\n\nb", 10), ["a", "", "b"]);
    }

    #[test]
    fn truncate_ends_a_cut_with_an_ellipsis() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    fn priced_offer(
        spread: Option<f64>,
        imbalance: Option<f64>,
        within_10: Option<(i64, f64)>,
    ) -> OfferResult {
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
            // The ask that goes with the spread, so the fixture stays internally consistent.
            best_ask: spread.map(|s| 457_900.0 * (1.0 + s / 100.0)),
            spread,
            // A book of 100 units split to match the imbalance, for the same reason.
            book_depth: imbalance.map(|percent| {
                let bid = (100.0 + percent) / 2.0;
                (bid as i64, 100 - bid as i64)
            }),
            imbalance,
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
        let result = priced_offer(Some(12.5), Some(50.0), Some((2_474_600, 3_980_000_000.0)));
        let cells = offer_cells(0, &result, "Sisters Combat Scanner Probe".to_string(), true);
        assert_eq!(cells.len(), 7);
        assert_eq!(cells[3], "12.50%");
        assert_eq!(cells[4], "+50%");
        assert_eq!(cells[5], "2,474,600");
        assert_eq!(cells[6], "3.98B");
    }

    #[test]
    fn narrow_offer_rows_leave_out_the_imbalance_and_the_volume() {
        let result = priced_offer(Some(12.5), Some(50.0), Some((2_474_600, 3_980_000_000.0)));
        let cells = offer_cells(0, &result, "Probe".to_string(), false);
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[3], "12.50%");
        // The profit, not the imbalance, is what follows the spread in a narrow pane.
        assert_eq!(cells[4], "3.98B");
    }

    #[test]
    fn offers_without_a_reward_show_n_a() {
        let result = priced_offer(Some(12.5), Some(50.0), None);
        let cells = offer_cells(0, &result, "Probe".to_string(), true);
        assert_eq!(cells[3], "12.50%");
        assert_eq!(cells[4], "+50%");
        assert_eq!(cells[5], "n/a");
        assert_eq!(cells[6], "n/a");
    }

    #[test]
    fn the_spread_column_shows_n_a_without_an_ask() {
        let result = priced_offer(None, None, Some((100, 1000.0)));
        let cells = offer_cells(0, &result, "Probe".to_string(), true);
        assert_eq!(cells[3], "n/a");
    }

    #[test]
    fn the_imbalance_column_shows_n_a_without_both_sides() {
        let result = priced_offer(Some(12.5), None, Some((100, 1000.0)));
        let cells = offer_cells(0, &result, "Probe".to_string(), true);
        assert_eq!(cells[4], "n/a");
    }

    #[test]
    fn the_imbalance_shows_its_sign_and_never_a_negative_zero() {
        assert_eq!(format_imbalance(Some(73.4)), "+73%");
        assert_eq!(format_imbalance(Some(-73.4)), "-73%");
        assert_eq!(format_imbalance(Some(100.0)), "+100%");
        assert_eq!(format_imbalance(Some(-100.0)), "-100%");
        assert_eq!(format_imbalance(Some(0.0)), "0%");
        assert_eq!(format_imbalance(Some(-0.4)), "0%");
        assert_eq!(format_imbalance(None), "n/a");
    }

    #[test]
    fn the_imbalance_colour_follows_the_bands() {
        assert_eq!(imbalance_style(Some(100.0)), Style::new().green());
        assert_eq!(
            imbalance_style(Some(IMBALANCE_STRONG)),
            Style::new().green()
        );
        assert_eq!(imbalance_style(Some(10.0)), Style::new().yellow());
        assert_eq!(imbalance_style(Some(0.0)), Style::new().yellow());
        assert_eq!(imbalance_style(Some(-10.0)), Style::new().yellow());
        assert_eq!(imbalance_style(Some(-IMBALANCE_STRONG)), Style::new().red());
        assert_eq!(imbalance_style(Some(-100.0)), Style::new().red());
        assert_eq!(imbalance_style(None), Style::new().dark_gray());
    }

    #[test]
    fn a_huge_spread_is_capped_to_hold_the_column_width() {
        assert_eq!(format_spread(Some(12.5)), "12.50%");
        assert_eq!(format_spread(Some(-25.0)), "-25.00%");
        assert_eq!(format_spread(Some(-100.0)), "-100.00%");
        assert_eq!(format_spread(Some(999.0)), "999.00%");
        assert_eq!(format_spread(Some(999.5)), ">999%");
        assert_eq!(format_spread(Some(1234.5)), ">999%");
        assert_eq!(format_spread(None), "n/a");
    }

    #[test]
    fn the_spread_colour_follows_the_bands() {
        assert_eq!(spread_style(Some(0.0)), Style::new().green());
        assert_eq!(spread_style(Some(SPREAD_TIGHT)), Style::new().green());
        assert_eq!(spread_style(Some(10.1)), Style::new().yellow());
        assert_eq!(spread_style(Some(SPREAD_WIDE)), Style::new().yellow());
        assert_eq!(spread_style(Some(50.1)), Style::new().red());
        assert_eq!(spread_style(Some(-1.0)), Style::new().magenta());
        assert_eq!(spread_style(None), Style::new().dark_gray());
    }

    #[test]
    fn the_offer_columns_match_the_cells_in_both_layouts() {
        // The titles, the widths, and the cells are three parallel lists, and a row that
        // disagrees with the other two renders misaligned.
        let result = priced_offer(Some(12.5), Some(50.0), Some((100, 1000.0)));
        for wide in [true, false] {
            let (titles, widths) = offer_columns(wide);
            let cells = offer_cells(0, &result, "Probe".to_string(), wide);
            assert_eq!(titles.len(), cells.len(), "titles, wide {}", wide);
            assert_eq!(widths.len(), cells.len(), "widths, wide {}", wide);
        }
    }

    /// The whole screen after drawing `f`, as every cell's symbol in reading order.
    fn rendered(width: u16, height: u16, f: impl FnOnce(&mut Frame)) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(f).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// An app sitting on the results view with one group of `offers`. The static data is empty, so
    /// names fall back to their IDs, which is enough to test how the view is laid out.
    fn results_app(offers: Vec<OfferResult>, depth: usize) -> App {
        let info = RunInfo {
            region: "The Forge".to_string(),
            corporations: 1,
            filters: "none".to_string(),
            depth,
            agent_filter: AgentFilter::default(),
        };
        let mut app = App::new(LpStoreOptions::default());
        app.begin_run(info);
        app.finish(Report {
            groups: vec![Group {
                corp_ids: vec![1000002],
                offer_count: offers.len(),
                results: offers,
                errors: Vec::new(),
            }],
        });
        app
    }

    /// The whole `--tui` view rendered at `width` x `height`, as one string.
    fn view_text(app: &App, width: u16, height: u16) -> String {
        let sdata = StaticData::default();
        let stats = ClientStats::default();
        rendered(width, height, |frame| {
            draw(frame, app, &sdata, &stats);
        })
    }

    /// One offer with a known spread, imbalance, volume, and profit, so a test can look for the
    /// exact strings that have to survive the layout.
    fn one_priced_offer() -> Vec<OfferResult> {
        vec![priced_offer(
            Some(12.5),
            Some(50.0),
            Some((2_474_600, 3_980_000_000.0)),
        )]
    }

    #[test]
    fn the_wide_offers_table_shows_every_column_at_120x40() {
        let app = results_app(one_priced_offer(), 10);
        let text = view_text(&app, 120, 40);
        for title in [
            "Item",
            "ISK/LP",
            "Spread",
            "Imbal",
            "Vol ±10%",
            "Profit ±10%",
        ] {
            assert!(text.contains(title), "the column {title:?} did not render");
        }
        for value in ["12.50%", "+50%", "2,474,600", "3.98B"] {
            assert!(text.contains(value), "the value {value:?} did not render");
        }
    }

    #[test]
    fn the_narrow_offers_table_fits_the_smallest_terminal() {
        // The smallest terminal the view accepts has to show the last column in full. It did not
        // until `Item`'s floor dropped, and nothing caught it because no test rendered the results.
        let app = results_app(one_priced_offer(), 10);
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT + 4);
        assert!(
            text.contains("Spread") && text.contains("12.50%"),
            "the spread column shows in both layouts"
        );
        assert!(
            text.contains("Profit ±10%") && text.contains("3.98B"),
            "the last column was cut at {}x{}",
            MIN_WIDTH,
            MIN_HEIGHT + 4
        );
        assert!(!text.contains("Imbal"), "the imbalance is wide only");
        assert!(!text.contains("Vol ±10%"), "the volume is wide only");
    }

    #[test]
    fn the_wide_offers_table_survives_the_terminal_it_switches_on_at() {
        // The offers pane is 68% of the terminal, so this is about the narrowest terminal whose
        // pane reaches WIDE_OFFERS. Every column has to survive at the threshold, which is where an
        // over-wide one first cuts the last column's header off.
        let width = (f64::from(WIDE_OFFERS) / 0.68).ceil() as u16;
        let app = results_app(one_priced_offer(), 10);
        let text = view_text(&app, width, 24);
        assert!(
            text.contains("Imbal"),
            "the wide layout did not engage at {} columns",
            width
        );
        assert!(
            text.contains("Profit ±10%") && text.contains("3.98B"),
            "the last column was cut at {} columns",
            width
        );
    }

    #[test]
    fn no_terminal_width_cuts_the_last_offers_column() {
        // Both layouts were truncating "Profit ±10%" at their own minimum widths, and the gap
        // between them was not obvious from the column arithmetic. Every width the view accepts has
        // to show the last column in full, whichever layout it picks.
        for width in MIN_WIDTH..=160 {
            let app = results_app(one_priced_offer(), 10);
            let text = view_text(&app, width, MIN_HEIGHT + 4);
            assert!(
                text.contains("Profit ±10%") && text.contains("3.98B"),
                "the last offers column was cut at {} columns",
                width
            );
        }
    }

    #[test]
    fn the_export_prompt_shows_the_name_and_the_footer_hints_for_it() {
        let mut app = results_app(one_priced_offer(), 10);
        app.open_export();
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Export"), "the popup is titled");
        assert!(
            text.contains("lpstores-the-forge.csv"),
            "the suggested name shows: {}",
            text
        );
        assert!(
            text.contains("Enter write"),
            "the footer switches to the prompt's keys"
        );
        assert!(
            !text.contains("f filters"),
            "the view keys are hidden while the prompt owns the keyboard"
        );
    }

    #[test]
    fn a_refused_export_shows_why_beside_the_name() {
        let mut app = results_app(one_priced_offer(), 10);
        app.open_export();
        app.refuse_export("Use a file name ending in .csv or .json");
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Use a file name ending in .csv or .json"));
        assert!(
            text.contains("lpstores-the-forge.csv"),
            "the name is still there to correct"
        );
    }

    #[test]
    fn the_static_data_dialog_offers_the_download_and_says_where_it_goes() {
        let mut app = App::new(LpStoreOptions::default());
        app.open_static_data(None);
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Static data"), "the popup is titled");
        assert!(
            text.contains("Enter or d downloads it"),
            "the keys are on the dialog itself: {}",
            text
        );
        assert!(
            text.contains("stored in data"),
            "it says where the files would go"
        );
        assert!(
            text.contains("Enter or d download"),
            "the footer switches to the dialog's keys"
        );
        assert!(
            !text.contains("f filters"),
            "the view keys are hidden while it is open"
        );
    }

    #[test]
    fn the_static_data_dialog_shows_the_reason_when_the_files_cannot_be_read() {
        let mut app = App::new(LpStoreOptions::default());
        app.open_static_data(Some("data/regions.json is not valid JSON".to_string()));
        let text = view_text(&app, 120, 40);
        assert!(text.contains("data/regions.json is not valid JSON"));
        assert!(
            text.contains("Enter or d downloads it"),
            "retry is still offered"
        );
    }

    #[test]
    fn the_static_data_dialog_changes_while_it_downloads() {
        let mut app = App::new(LpStoreOptions::default());
        app.open_static_data(None);
        app.start_download();
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("Downloading the EVE Online static data"),
            "the dialog says something is happening"
        );
        assert!(
            !text.contains("Enter or d download"),
            "the download keys are gone once it started"
        );
        assert!(text.contains("Ctrl+C quits"), "the way out is still shown");
    }

    #[test]
    fn the_idle_view_says_nothing_has_started_behind_the_startup_dialog() {
        let mut app = App::new(LpStoreOptions::default());
        app.open_static_data(None);
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Not started"), "the header is honest");
        assert!(
            !text.contains("0 corporations"),
            "no corporation count is claimed before a run"
        );
        assert!(
            !text.contains("waiting"),
            "the gauges are left out rather than reading waiting before anything has started"
        );
        assert!(
            !text.contains("avg ISK/LP"),
            "no results are claimed before a run"
        );
    }

    #[test]
    fn the_startup_filters_dialog_shows_the_prefilled_defaults() {
        let mut app = App::new(LpStoreOptions::default());
        let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        app.show_filters(form);
        let text = view_text(&app, 120, 40);
        assert!(text.contains("The Forge"), "the region default is visible");
        assert!(
            text.contains("Corporations"),
            "the filter fields are there and empty"
        );
        assert!(
            text.contains("Enter apply"),
            "the footer shows the form's keys"
        );
    }

    #[test]
    fn a_note_about_the_saved_file_shows_on_the_startup_form() {
        // The form owns the keyboard and the log panel cannot be opened over it, so this is the only
        // place the reason could be seen before the first run.
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.set_note(Some(
            "filters.json is not valid JSON: expected value".to_string(),
        ));
        app.show_filters(form);
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("is not valid JSON"),
            "the reason is on screen: {}",
            text
        );
        assert!(
            text.contains("The Forge"),
            "the form still shows what it fell back to"
        );
    }

    #[test]
    fn the_market_hints_split_between_the_row_and_the_note_below() {
        // The row's detail column is one line tall, so an inline hint must show only its purpose.
        // Before this split the two lines collided into one mangled row reading
        // "Prices use its NPC stations.Name or ID…".
        for (focus, purpose) in [
            (FormField::Region, "Prices come from its market orders."),
            (FormField::System, "Prices use its NPC stations."),
        ] {
            let mut app = App::new(LpStoreOptions::default());
            let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
            form.focus = focus;
            app.show_filters(form);
            let text = view_text(&app, 120, 40);
            assert!(text.contains(purpose), "the row shows its purpose");
            assert!(
                !text.contains(&format!("{purpose}Name")),
                "the row glued two lines together: {focus:?}"
            );
        }
    }

    #[test]
    fn the_focused_market_field_shows_both_hint_lines_when_there_is_room() {
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.focus = FormField::System;
        app.show_filters(form);
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("Name or ID. Blank for all systems."),
            "the syntax line shows under the focused field"
        );
    }

    #[test]
    fn the_inline_hint_is_one_line_even_when_the_note_is_two() {
        assert_eq!(
            hint_line(FormField::Region, &StaticData::default()),
            "Prices come from its market orders."
        );
        assert_eq!(
            hint_line(FormField::System, &StaticData::default()),
            "Prices use its NPC stations."
        );
        // The market fields are the ones this change worded, and both fit the detail column at the
        // minimum terminal width so their purpose is never cut. Other hints are longer and already
        // truncate there, which is existing behaviour this test does not cover.
        let detail = usize::from(MIN_WIDTH - 2) - (LABEL_WIDTH + VALUE_WIDTH + 2);
        for field in [FormField::Region, FormField::System] {
            let line = hint_line(field, &StaticData::default());
            assert!(
                line.chars().count() <= detail,
                "{field:?} needs {} of {} columns: {:?}",
                line.chars().count(),
                detail,
                line
            );
        }
        // No inline hint may carry a break, whatever its length.
        for field in FormField::ALL {
            assert!(
                !hint_line(field, &StaticData::default()).contains('\n'),
                "{field:?} kept the break in the row"
            );
        }
    }

    #[test]
    fn the_grouped_dialog_renders_at_the_minimum_terminal_size() {
        let mut app = App::new(LpStoreOptions::default());
        app.show_filters(FilterForm::from_options_with_defaults(
            &app.applied,
            "The Forge",
            10,
        ));
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        for label in [
            "Corporations",
            "Security",
            "Agent level",
            "Division",
            "Agent type",
            "Faction",
            "Market region",
            "Market system",
            "Depth",
            "Expire cache",
        ] {
            assert!(text.contains(label), "{label:?} did not render");
        }
        assert!(
            !text.contains("stations.Name"),
            "a two-line hint was glued into one row"
        );
    }

    fn screen_text(width: u16, height: u16) -> String {
        rendered(width, height, |frame| draw_donate(frame, frame.area()))
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

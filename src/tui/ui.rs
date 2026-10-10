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
/// The smallest view the app draws. Twelve filter rows need room, and the notes under them carry
/// the hints, the valid values and every validation error: at 20 rows the filters dialog had two
/// rows of notes left, and adding the two agent place filters would have taken them to zero, which
/// is a form that cannot report a mistake. 24 leaves four.
pub const MIN_HEIGHT: u16 = 24;

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

/// What the last frame measured about the scrollable panes, fed back so the keys can bound the
/// scroll without the view holding the static data it renders from.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaneLayout {
    pub detail_rows: usize,
    pub detail_visible: usize,
    pub destinations_rows: usize,
    pub destinations_visible: usize,
}

/// Draws the whole view. Reports what each scrollable pane needs to know about itself.
pub fn draw(frame: &mut Frame, app: &App, sdata: &StaticData, stats: &ClientStats) -> PaneLayout {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let message = format!(
            "Terminal too small: need {}x{}, have {}x{}",
            MIN_WIDTH, MIN_HEIGHT, area.width, area.height
        );
        frame.render_widget(Paragraph::new(message).red(), area);
        return PaneLayout::default();
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
            PaneLayout::default()
        }
        Overlay::Filters => {
            draw_form(frame, body, app, sdata);
            PaneLayout::default()
        }
        Overlay::Destinations => draw_destinations(frame, body, app, sdata),
        _ if app.report.is_some() => draw_results(frame, body, app, sdata),
        _ => {
            draw_progress(frame, body, app);
            PaneLayout::default()
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
    // Last, so any overlay the user asked for wins; the run's own popup is not something they opened.
    if app.overlay == Overlay::None && (app.awaiting_run_summary || in_flight_run(app)) {
        draw_run_progress(frame, area, app);
    }
    draw_footer(frame, footer, stats, app);
    layout
}

/// Where the static data files live, for the dialog's text. A placeholder until the view is given
/// the real root, so the string is never empty and never wrong about a directory it has not seen.
pub(crate) const UNKNOWN_DATA_DIR: &str = "the data directory";

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
        truncate(&format!("It is stored in {}.", app.data_dir), inner).dark_gray(),
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

/// Whether a run is in flight over the results already on screen. `stale` says a report exists and a
/// newer run replaced it; the stage says that run has not finished.
fn in_flight_run(app: &App) -> bool {
    app.stale
        && matches!(
            app.stage,
            Stage::Starting | Stage::Fetching { .. } | Stage::Pricing { .. }
        )
}

/// The progress of a run that superseded the results on screen, shown over them. Before any results
/// exist the same numbers are drawn inline by `draw_progress`; once they do, `draw_results` takes the
/// body and those bars become unreachable, which left a re-run showing nothing but a header line and
/// a `(stale)` suffix on three pane titles.
///
/// The popup takes no key. It is a drawing decision, not a mode: the groups, offers and details panes
/// behind it still answer to Tab and the arrows, and any key at all acknowledges the end-of-run
/// summary, which is what `App::awaiting_run_summary` is for.
fn draw_run_progress(frame: &mut Frame, area: Rect, app: &App) {
    const WIDTH: u16 = 64;
    let summary = app.awaiting_run_summary;
    // Eight inner rows for the stage line and the two bars, plus one for the key line only when there
    // is something to acknowledge, plus the border.
    let popup = area.centered(
        Constraint::Length(WIDTH),
        Constraint::Length(if summary { 10 } else { 9 }),
    );
    frame.render_widget(Clear, popup);
    let block = Block::bordered().title(" Run ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let [stage, fetch, pricing, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(if summary { 1 } else { 0 }),
    ])
    .areas(inner);

    let stage_style = if app.stage == Stage::Failed {
        Style::new().red()
    } else {
        Style::new()
    };
    frame.render_widget(
        Paragraph::new(truncate(&app.stage_text(), usize::from(WIDTH) - 2)).style(stage_style),
        stage,
    );
    let ((fetch_done, fetch_total), (price_done, price_total)) = app.gauges();
    gauge(
        frame,
        fetch,
        "Fetching loyalty offers",
        fetch_done,
        fetch_total,
    );
    gauge(frame, pricing, "Pricing groups", price_done, price_total);
    if summary {
        frame.render_widget(Paragraph::new("any key closes this").dark_gray(), keys);
    }
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

/// One line per probability group: whole percents, since the counts are small and a decimal
/// suggests a precision the model does not have.
fn percent(part: f64) -> String {
    format!("{:.0}%", part * 100.0)
}

/// The destination table as plain text, one line per system and per destination. Split out from the
/// drawing so the wording is testable without a frame.
fn destination_rows(app: &App, sdata: &StaticData, width: usize) -> Vec<String> {
    let Some(group) = app.selected_group() else {
        return vec!["No group selected.".to_string()];
    };
    let mut rows = Vec::new();
    let scope = destination_scope(sdata.security_division_id(), app.agent_filter.division_id);
    if scope != Scope::Included {
        return vec![scope_note(sdata, scope, width)];
    }
    for corp_id in &group.corp_ids {
        rows.push(output::corporation_label(sdata, *corp_id));
        let systems = sdata.security_agent_systems(*corp_id, &app.agent_filter);
        if systems.is_empty() {
            rows.push("  no security agents match the filters".to_string());
            continue;
        }
        for (system_id, name, _) in systems {
            let Some(spread) = sdata.mission_spread(system_id) else {
                rows.push(format!("  {name} ({system_id})  no constellation data"));
                continue;
            };
            rows.extend(system_lines(system_id, &name, &spread, &|other| {
                sdata.system_name(other).cloned()
            }));
        }
    }
    rows
}

/// The lines for one agent system: a summary of the bands and the hop distances, then one line per
/// destination. Split out so the wording is testable against a hand-built spread without static
/// data. `name_of` resolves a destination id, which the caller reads from the archive.
fn system_lines(
    system_id: i32,
    name: &str,
    spread: &eve_lpstore::static_data::DestinationSpread,
    name_of: &dyn Fn(i32) -> Option<String>,
) -> Vec<String> {
    let bands = spread
        .bands
        .iter()
        .map(|(band, part)| format!("{}/{}", band_tenths(*band), percent(*part)))
        .collect::<Vec<_>>()
        .join(" ");
    let hops = spread
        .hops
        .iter()
        .map(|(distance, part)| format!("{distance}J/{}", percent(*part)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut rows = vec![format!("  {name} ({system_id})  route {bands}  {hops}")];
    for (dest_id, distance, route_band, own_band) in &spread.destinations {
        if *distance == 0 {
            // The agent system itself is a destination, but listing it under its own summary adds
            // nothing and repeats the header.
            continue;
        }
        let dest = name_of(*dest_id).unwrap_or_else(|| format!("system {dest_id}"));
        let note = if route_band == own_band {
            String::new()
        } else {
            // Only worth spelling out when they disagree: the route forces you lower than the
            // destination's own security.
            format!("  own {own}", own = band_tenths(*own_band))
        };
        rows.push(format!(
            "    {dest} {distance}J route {route}{note}",
            route = band_tenths(*route_band)
        ));
    }
    rows
}

/// The colour of a security band in the destination views: the asked-for purple at 0.0 and below,
/// red through lowsec, yellow at 0.5, green across the highsec middle and cyan at the top.
///
/// Ratatui's palette is ANSI and has no `Purple`, only `Magenta` and `LightMagenta`. `LightMagenta`
/// is what reads as purple on the usual terminal themes, and it is chosen over `Magenta` because the
/// offers table already spends `Magenta` on an inverted spread: two meanings in one colour two panes
/// apart is worse than two close shades with distinct meanings.
fn band_colour(tenths: i32) -> Color {
    match tenths {
        t if t <= 0 => Color::LightMagenta,
        t if t <= 4 => Color::Red,
        5 => Color::Yellow,
        t if t <= 8 => Color::Green,
        _ => Color::Cyan,
    }
}

/// The line broken into spans with its security bands coloured. Band labels are already written in
/// tenths by the formatters, so this reads the printed digits rather than converting a security
/// again, which is what keeps `(0.18)` — a system's true security, two decimals — plain while
/// `0.4/83%`, `route 0.4` and `own 0.5` are coloured.
fn coloured(text: &str, base: Style) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut at = 0;
    while at < chars.len() {
        let Some((tenths, next)) = read_band(&chars, at) else {
            plain.push(chars[at]);
            at += 1;
            continue;
        };
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut plain), base));
        }
        let words: String = chars[at..next].iter().collect();
        spans.push(Span::styled(
            words,
            base.patch(Style::new().fg(band_colour(tenths))),
        ));
        at = next;
    }
    if !plain.is_empty() || spans.is_empty() {
        spans.push(Span::styled(plain, base));
    }
    spans
}

/// A band label starting at `at`: an optional sign, one digit, a dot and one digit, with nothing
/// after it that would make the number longer. Returns its value in tenths and where it ends.
fn read_band(chars: &[char], at: usize) -> Option<(i32, usize)> {
    let mut i = at;
    let negative = chars.get(i) == Some(&'-');
    if negative {
        i += 1;
    }
    let ones = chars.get(i).and_then(|c| c.to_digit(10))?;
    i += 1;
    if chars.get(i) != Some(&'.') {
        return None;
    }
    i += 1;
    let tenths = chars.get(i).and_then(|c| c.to_digit(10))?;
    i += 1;
    if chars
        .get(i)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
    {
        return None;
    }
    let value = ones as i32 * 10 + tenths as i32;
    Some((if negative { -value } else { value }, i))
}

/// The agent systems a corporation's block shows, and how many it withholds. Data order is kept, so
/// the first rows are the same systems `corp_line` lists in the CLI report. Exactly at the cap shows
/// them all and says nothing about it, because there is nothing withheld.
fn cap_systems<T>(systems: Vec<T>) -> (Vec<T>, usize) {
    let hidden = systems.len().saturating_sub(output::MAX_SYSTEMS_SHOWN);
    (
        systems
            .into_iter()
            .take(output::MAX_SYSTEMS_SHOWN)
            .collect(),
        hidden,
    )
}

/// The line that stands in for the agent systems a corporation has more of than the pane shows.
fn more_systems_row(hidden: usize) -> String {
    let word = if hidden == 1 { "system" } else { "systems" };
    format!("  +{hidden} more {word}, press m for all of them")
}

/// The details-pane line for one agent system: its name and true security, then the band groups and
/// the hop groups. The destinations pane spells the same numbers with the system id and the word
/// `route`; here the row sits under its corporation, so the security stands in for the id.
fn summary_text(
    name: &str,
    true_sec: f64,
    spread: &eve_lpstore::static_data::DestinationSpread,
) -> String {
    let bands = spread
        .bands
        .iter()
        .map(|(band, part)| format!("{}/{}", band_tenths(*band), percent(*part)))
        .collect::<Vec<_>>()
        .join(", ");
    let hops = spread
        .hops
        .iter()
        .map(|(distance, part)| format!("{distance}J/{}", percent(*part)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("  {name} ({true_sec:.2}) {bands}  {hops}")
}

/// Tenths to the one-decimal label the panes and `--security` both use.
fn band_tenths(tenths: i32) -> String {
    format!("{:.1}", tenths as f64 / 10.0)
}

/// The mission destinations pane for the selected group. Probabilities are even shares of the
/// systems in the agent system's constellation, which is an assumption this tool makes rather than a
/// measurement of how missions are routed, and the band a destination lands in is the lowest
/// security on the safest way to reach it, which is why it can sit below the destination's own.
/// The pane's standing lines, kept above the scrolling body so the disclaimer cannot be scrolled
/// out of sight. Both are 74 characters or fewer, which is inside the 78 columns a border leaves at
/// the minimum supported width, so neither wraps and neither eats a row of the body.
const DESTINATION_NOTES: [&str; 2] = [
    "model, not measurement: security missions stay in the constellation",
    "bands are the lowest security on the safest gate route, not a system's own",
];

/// The one-line reason the systems are missing, for whichever view asks.
fn scope_note(sdata: &StaticData, scope: Scope, width: usize) -> String {
    match scope {
        Scope::Unknown => "the static data names no Security division".to_string(),
        Scope::Excluded(division) => {
            let name = sdata
                .division_name(division)
                .cloned()
                .unwrap_or_else(|| format!("Division {division}"));
            excluded_row(&name, width)
        }
        Scope::Included => String::new(),
    }
}

/// The agent systems the pane describes, and why any of them might be absent. The model covers
/// Security division agents only, so a run filtered to courier or mining agents has nothing to show
/// and says so instead of listing agents it was not asked to model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The static data names no Security division, so nothing can be put in scope.
    Unknown,
    /// The run asked for a different division, carrying its id.
    Excluded(i32),
    /// Security agents are in scope for this run.
    Included,
}

/// Which divisions the pane may describe, from the id the static data calls Security and the id the
/// run filtered on. Kept apart from the data so the decision is testable on its own: a run that
/// asked for courier agents has nothing in scope, and a run that named no division has everything
/// in scope that its other filters allow.
fn destination_scope(security: Option<i32>, asked: Option<i32>) -> Scope {
    let Some(security) = security else {
        return Scope::Unknown;
    };
    match asked {
        Some(division) if division != security => Scope::Excluded(division),
        _ => Scope::Included,
    }
}

/// The tail of the line shown when the run's division filter excludes the missions this model
/// describes. The longest division name in the archive is 28 characters, which with a fixed sentence
/// would overrun the pane at the minimum width, so the name is cut to whatever room is left.
const EXCLUDED_TAIL: &str = " agents are outside this model: security missions only";

fn excluded_row(division: &str, width: usize) -> String {
    let budget = width.saturating_sub(EXCLUDED_TAIL.chars().count());
    format!("{}{EXCLUDED_TAIL}", truncate(division, budget))
}

/// Which of the pane's rows are on screen. The window starts at the top of the table, so a pane that
/// opens with nothing scrolled shows the first corporation and its systems rather than the tail of
/// the list, and a scroll past the end is pulled back until the last row is on screen.
fn pane_window(rows: usize, scroll: usize, height: usize) -> (usize, usize) {
    if height == 0 {
        return (0, 0);
    }
    let start = scroll.min(rows.saturating_sub(height));
    (start, (start + height).min(rows))
}

/// The pane's rows, each given the style its indent asks for and cut to the pane's width. A
/// corporation names itself at column 0, a system indents two, a destination four, and the
/// indentation survives the cut so a wrapped row cannot be mistaken for the start of a new entry.
fn pane_body(rows: &[String], width: usize) -> Vec<(String, Style)> {
    rows.iter()
        .flat_map(|row| {
            let style = if row.starts_with("    ") {
                Style::new().dark_gray()
            } else if row.starts_with("  ") {
                Style::new()
            } else {
                Style::new().bold()
            };
            wrap_text(row, width)
                .into_iter()
                .map(move |part| (part, style))
        })
        .collect()
}

fn draw_destinations(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) -> PaneLayout {
    let block = pane_block("Mission destinations", false, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width).max(1);
    let rows = destination_rows(app, sdata, width);
    // Measured on the real archive a summary line runs to a median of 65 characters and a maximum of
    // 126, so at the 78 columns a border leaves at the minimum width some of them have to wrap. They
    // wrap here rather than being clipped, with the indent kept on every row, because a cut-off
    // percentage is worse than a second row.
    let body = pane_body(&rows, width);
    // The notes take the first lines, so the scrollable body starts below them, and the window opens
    // at the top of the table: the first rows are the corporation names and their systems.
    let height = usize::from(inner.height).saturating_sub(DESTINATION_NOTES.len());
    let (start, end) = pane_window(body.len(), app.destinations_scroll, height);
    let mut lines: Vec<Line<'static>> = DESTINATION_NOTES
        .iter()
        .map(|note| Line::from(Span::styled(*note, Style::new().italic().dark_gray())))
        .collect();
    lines.extend(
        body[start..end]
            .iter()
            .map(|(row, style)| Line::from(coloured(row, *style))),
    );
    // Sliced to the visible window already, so no `.scroll()` offset on top of that.
    frame.render_widget(Paragraph::new(lines), inner);
    PaneLayout {
        detail_rows: 0,
        detail_visible: 0,
        // The count the keys scroll against is the wrapped rows, not the logical lines.
        destinations_rows: body.len(),
        destinations_visible: height,
    }
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
            // Two lines like the market fields: the first has to fit the row's detail column at the
            // minimum terminal width.
            "0.5 exact, 0.5..0.7 for a range.\n0.5.. is at least, ..0.5 is at most. Blank for any."
                .to_string()
        }
        // Two lines like the market fields: what the filter keeps, then how to write it. The names
        // of the two pairs are close enough to mix up, so each line says which side of it this is.
        FormField::AgentRegion => {
            "Keeps agents standing in it.\nID or name. Blank for any region.".to_string()
        }
        FormField::AgentSystem => {
            "Keeps agents in this system.\nID or name. Blank for any system.".to_string()
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
        FormField::CacheMinutes => {
            "Minutes past ESI Expires.\nBlank: filtered corp count; 0 disables.".to_string()
        }
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
        FormField::System | FormField::AgentSystem => sdata
            .system_names()
            .filter(|name| name.to_lowercase().contains(&query))
            .cloned()
            .collect(),
        FormField::AgentRegion => sdata
            .region_names()
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

fn draw_results(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) -> PaneLayout {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).areas(area);
    let [offers, details] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Min(0)]).areas(right);
    draw_groups(frame, left, app, sdata);
    draw_offers(frame, offers, app, sdata);
    let (detail_rows, detail_visible) = draw_details(frame, details, app, sdata);
    PaneLayout {
        detail_rows,
        detail_visible,
        destinations_rows: 0,
        destinations_visible: 0,
    }
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
        .flat_map(|(text, style)| {
            wrap_text(text, width)
                .into_iter()
                .map(move |row| Line::from(coloured(&row, *style)))
        })
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
    // Leading spaces are indentation, not a word, and splitting on spaces would drop them: a
    // wrapped destination line would then start its second row at column 0 and read like a new
    // system rather than the continuation of one. The indent is taken off, the rest wrapped into
    // what is left of the width, and the indent put back on every row.
    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    if !indent.is_empty() {
        if width <= indent.chars().count() {
            // Nothing is left to wrap into. Hand back the line and let the pane clip it, which is
            // what it did before, rather than cutting one character per row. The view's own
            // minimum width makes this unreachable; the guard is so it stays that way.
            return vec![text.to_string()];
        }
        let room = width - indent.chars().count();
        return wrap_text(&text[indent.len()..], room)
            .into_iter()
            .map(|row| format!("{indent}{row}"))
            .collect();
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

/// The text of the details pane, one entry per logical line. `wrap` turns each entry into one
/// or more screen rows.
fn detail_entries(app: &App, sdata: &StaticData) -> Vec<(String, Style)> {
    let plain = Style::new();
    let bold = Style::new().bold();
    let Some(group) = app.selected_group() else {
        return Vec::new();
    };
    let mut lines = vec![("Corporations".to_string(), bold)];
    let scope = destination_scope(sdata.security_division_id(), app.agent_filter.division_id);
    if scope != Scope::Included {
        // The systems below are the ones the model describes, so an excluded run says why there are
        // none rather than leaving the corporation names standing alone. The `m` pane gives the same
        // reason at more length.
        lines.push((scope_note(sdata, scope, usize::from(MIN_WIDTH) - 2), plain));
        for &id in &group.corp_ids {
            lines.push((output::corporation_label(sdata, id), plain));
        }
        lines.push((String::new(), plain));
        return lines;
    }
    for &id in &group.corp_ids {
        lines.push((output::corporation_label(sdata, id), plain));
        let systems = sdata.security_agent_systems(id, &app.agent_filter);
        if systems.is_empty() {
            lines.push(("  no security agents match the filters".to_string(), plain));
            continue;
        }
        // A corporation can hold far more agent systems than the pane has rows for: measured on the
        // archive, 1 to 53 with a median of 11, and at 80 columns each summary wraps to two rows, so
        // the largest case pushed the offer breakdown 107 rows down. The systems past the cap are
        // named by count and pointed at the pane that lists them all.
        let (shown, hidden) = cap_systems(systems);
        for (system_id, name, true_sec) in shown {
            let text = match sdata.mission_spread(system_id) {
                Some(spread) => summary_text(&name, true_sec, &spread),
                None => format!("  {name} ({true_sec:.2})  no constellation data"),
            };
            lines.push((text, plain));
        }
        if hidden > 0 {
            lines.push((more_systems_row(hidden), plain));
        }
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
        (
            "m",
            "where missions from the selected group's agents can take you",
        ),
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
        Overlay::Destinations => "Up/Down scroll  Esc close  Ctrl+C quit",
        Overlay::StaticData => {
            if app.downloading {
                "Downloading...  Ctrl+C quits"
            } else {
                "Enter or d download  Esc exit"
            }
        }
        _ => {
            "Tab panes  Up/Down move  f filters  e export  m missions  d donate  l log  ? help  q quit"
        }
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
    use eve_lpstore::report::{Field, LpStoreOptions, OptionError, Report};
    use eve_lpstore::static_data::AgentFilter;

    #[test]
    fn wrapping_keeps_the_indent_on_every_row() {
        // Destination lines are indented under their corporation. Without this the continuation row
        // sat at column 0, which is where a new system would start.
        assert_eq!(
            wrap_text("  Bereye 0.5/12% 0.6/38%", 12),
            ["  Bereye", "  0.5/12%", "  0.6/38%"]
        );
        // A line with no indent is wrapped exactly as before.
        assert_eq!(wrap_text("one two three", 7), ["one two", "three"]);
        // An indent wider than the row still keeps its text rather than vanishing.
        assert_eq!(
            wrap_text("    deep", 3),
            ["    deep"],
            "clipped, not shredded"
        );
    }

    #[test]
    fn wrap_breaks_at_spaces_and_cuts_long_words() {
        assert_eq!(
            wrap_text("Ministry of Assessment (1000081): Turba", 20),
            ["Ministry of", "Assessment", "(1000081): Turba"]
        );
        assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("", 10), [""]);
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
            region_id: 10000002,
            agent_region_id: None,
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

    /// The whole `--tui` view rendered at `width` x `height`, as one string, on empty static data.
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
            text.contains("It is stored in the data directory."),
            "it names a place even before the real root is known"
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
    fn the_static_data_dialog_names_the_root_the_view_was_given() {
        // The whole point of the platform roots is that they are not `data/` any more, so the
        // dialog has to repeat what it was told rather than a guessed relative name.
        let mut app = App::new(LpStoreOptions::default());
        app.set_data_dir("/home/user/.local/share/eve_lpstore");
        app.open_static_data(None);
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("It is stored in /home/user/.local/share/eve_lpstore."),
            "the given root is not shown"
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
    fn the_focused_place_field_shows_both_hint_lines() {
        // The market fields ask for two lines, and the place fields do the same. This is what the
        // raised minimum height pays for: at the old size the second line had nowhere to go.
        for (focus, syntax) in [
            (FormField::AgentRegion, "ID or name. Blank for any region."),
            (FormField::AgentSystem, "ID or name. Blank for any system."),
        ] {
            let mut app = App::new(LpStoreOptions::default());
            let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
            form.focus = focus;
            app.show_filters(form);
            let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
            assert!(
                text.contains(syntax),
                "{focus:?} lost its syntax line: {text}"
            );
        }
    }

    #[test]
    fn the_cache_minutes_field_explains_the_blank_default_and_zero() {
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.focus = FormField::CacheMinutes;
        app.show_filters(form);
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        assert!(text.contains("Minutes past ESI Expires."), "{text}");
        assert!(
            text.contains("Blank: filtered corp count; 0 disables."),
            "the two existing defaults are explained: {text}"
        );
    }

    #[test]
    fn the_pane_words_a_collapsed_band_group() {
        // Sazdih from Bayuka: every route starts in the lowest system, so the route bands
        // collapse to one row while the destination lines still name Moh's own 0.5.
        let spread = eve_lpstore::static_data::DestinationSpread {
            constellation_id: 20000006,
            systems: 6,
            bands: vec![(4, 1.0)],
            own_bands: vec![(4, 5.0 / 6.0), (5, 1.0 / 6.0)],
            hops: vec![(0, 1.0 / 6.0), (1, 3.0 / 6.0), (2, 2.0 / 6.0)],
            destinations: vec![
                (30000039, 0, 4, 4),
                (30000038, 1, 4, 4),
                (30000040, 1, 4, 4),
                (30000041, 1, 4, 4),
                (30000042, 2, 4, 5),
                (30000043, 2, 4, 4),
            ],
        };
        let names = std::collections::HashMap::from([
            (30000039, "Bayuka"),
            (30000038, "Dooz"),
            (30000040, "Uzistoon"),
            (30000041, "Bairshir"),
            (30000042, "Moh"),
            (30000043, "Sari"),
        ]);
        let rows = system_lines(30000039, "Bayuka", &spread, &|id| {
            names.get(&id).map(|found| found.to_string())
        });
        assert_eq!(
            rows[0],
            "  Bayuka (30000039)  route 0.4/100%  0J/17% 1J/50% 2J/33%"
        );
        assert_eq!(
            rows[1], "    Dooz 1J route 0.4",
            "same band as its own, so no note"
        );
        let moh = rows.iter().find(|r| r.contains("Moh")).expect("Moh listed");
        assert_eq!(moh, "    Moh 2J route 0.4  own 0.5");
        assert_eq!(rows.len(), 6, "one summary plus five destinations");
        assert!(
            !rows.iter().any(|r| r.contains("Bayuka (30000039)  0J")),
            "the agent system is not repeated as a destination"
        );
    }

    #[test]
    fn the_pane_splits_bands_when_the_routes_allow_it() {
        let spread = eve_lpstore::static_data::DestinationSpread {
            constellation_id: 100,
            systems: 4,
            bands: vec![(5, 0.75), (6, 0.25)],
            own_bands: vec![(5, 0.75), (6, 0.25)],
            hops: vec![(0, 0.25), (1, 0.25), (2, 0.25), (3, 0.25)],
            destinations: vec![(1, 0, 5, 5), (2, 1, 5, 5), (3, 2, 5, 5), (4, 3, 5, 6)],
        };
        let rows = system_lines(1, "A", &spread, &|id| Some(format!("S{id}")));
        assert_eq!(
            rows[0],
            "  A (1)  route 0.5/75% 0.6/25%  0J/25% 1J/25% 2J/25% 3J/25%"
        );
        assert_eq!(rows.len(), 4, "one summary plus the three other systems");
        assert_eq!(rows[3], "    S4 3J route 0.5  own 0.6");
    }

    #[test]
    fn an_unknown_destination_id_still_gets_a_line() {
        let spread = eve_lpstore::static_data::DestinationSpread {
            constellation_id: 1,
            systems: 2,
            bands: vec![(4, 1.0)],
            own_bands: vec![(4, 1.0)],
            hops: vec![(0, 0.5), (1, 0.5)],
            destinations: vec![(9, 0, 4, 4), (99, 1, 4, 4)],
        };
        let rows = system_lines(9, "Here", &spread, &|_| None);
        assert_eq!(rows[1], "    system 99 1J route 0.4");
    }

    /// A spread with the numbers of the asked-for example, so the wording is pinned against it.
    fn two_band_spread() -> eve_lpstore::static_data::DestinationSpread {
        eve_lpstore::static_data::DestinationSpread {
            constellation_id: 20000300,
            systems: 2,
            bands: vec![(1, 0.5), (2, 0.5)],
            own_bands: vec![(1, 0.5), (2, 0.5)],
            hops: vec![(0, 0.5), (1, 0.5)],
            destinations: vec![(30003601, 0, 1, 1), (30003602, 1, 2, 2)],
        }
    }

    #[test]
    fn the_pane_opens_at_the_top_of_the_table() {
        // The bug this pins: the pane inherited the log's bottom-first window, so it opened showing
        // the tail of the list and Down moved it up. A corporation table reads top to bottom.
        assert_eq!(pane_window(60, 0, 10), (0, 10), "nothing scrolled");
        assert_eq!(pane_window(60, 5, 10), (5, 15), "scrolled down");
        assert_eq!(
            pane_window(60, 999, 10),
            (50, 60),
            "clamped to the last row"
        );
        assert_eq!(pane_window(4, 0, 10), (0, 4), "shorter than the pane");
        assert_eq!(
            pane_window(4, 2, 0),
            (0, 0),
            "a pane with no room shows nothing"
        );
    }

    #[test]
    fn a_long_summary_wraps_in_the_pane_instead_of_being_cut() {
        // The longest real summary measured 126 characters against the 78 columns a border leaves at
        // the minimum width, so this is the case the pane cannot show in one row.
        let long = format!(
            "  {}",
            [
                "Evenlongname (0.12)",
                "0.1/12%, 0.2/12%, 0.3/12%, 0.4/12%, 0.5/12%",
                "0J/12%, 1J/12%, 2J/12%, 3J/12%, 4J/12%, 5J/12%, 6J/12%, 7J/12%",
            ]
            .join("  ")
        );
        assert!(long.chars().count() > 100, "fixture is long enough");
        let dest = "    Deep 3J route 0.4";
        let body = pane_body(&[long.clone(), dest.to_string()], 78);
        assert!(
            body.len() >= 3,
            "the summary took more than one row: {body:?}"
        );
        for (part, _) in &body {
            assert!(part.chars().count() <= 78, "{part:?} overran the pane");
        }
        // Wrapping must lose no word and reorder none.
        let wrapped: Vec<&str> = body
            .iter()
            .flat_map(|(part, _)| part.split_whitespace())
            .collect();
        let wanted: Vec<&str> = long
            .split_whitespace()
            .chain(dest.split_whitespace())
            .collect();
        assert_eq!(wrapped, wanted, "wrapping lost or moved a word");
        assert!(
            body[0].0.starts_with("  Evenlongname"),
            "the first row keeps the indent"
        );
        assert!(
            body.iter().skip(1).all(|(part, _)| part.starts_with("  ")),
            "continuation rows are indented too: {body:?}"
        );
        assert_eq!(
            body.last().unwrap().0,
            dest,
            "the destination row is intact"
        );
        assert_eq!(body.last().unwrap().1, Style::new().dark_gray());
    }

    #[test]
    fn the_system_block_caps_at_what_the_cli_report_cuts_at() {
        // One constant governs both views, so a corporation cannot show ten systems in the report and
        // twelve in the pane. 53 is the largest count measured on the archive.
        let ids: Vec<i32> = (1..=53).collect();
        let (shown, hidden) = cap_systems(ids);
        assert_eq!(shown.len(), output::MAX_SYSTEMS_SHOWN);
        assert_eq!(hidden, 43);
        assert_eq!(
            shown,
            (1..=10).collect::<Vec<_>>(),
            "data order, the first ten"
        );
        let exactly = cap_systems((1..=10).collect::<Vec<i32>>());
        assert_eq!(
            (exactly.0.len(), exactly.1),
            (10, 0),
            "at the cap, nothing is withheld"
        );
        let few = cap_systems((1..=3).collect::<Vec<i32>>());
        assert_eq!((few.0.len(), few.1), (3, 0));
        assert_eq!(
            cap_systems(Vec::<i32>::new()).1,
            0,
            "and an empty list stays empty"
        );
    }

    #[test]
    fn the_overflow_line_counts_what_it_hides_and_names_the_way_to_see_it() {
        assert_eq!(
            more_systems_row(43),
            "  +43 more systems, press m for all of them"
        );
        assert_eq!(
            more_systems_row(1),
            "  +1 more system, press m for all of them"
        );
        // The largest count this can reach is 43, the biggest corporation measured on the archive
        // holding 53 systems, so the line stays well inside the 52 columns the details pane has at
        // the minimum size and never wraps onto a second row of its own.
        assert_eq!(more_systems_row(43).chars().count(), 43);
    }

    #[test]
    fn the_details_summary_is_one_indented_line_per_system() {
        assert_eq!(
            summary_text("Babirmoult", 0.1788, &two_band_spread()),
            "  Babirmoult (0.18) 0.1/50%, 0.2/50%  0J/50%, 1J/50%"
        );
        assert_eq!(
            summary_text("Annancale", 0.0941, &two_band_spread()),
            "  Annancale (0.09) 0.1/50%, 0.2/50%  0J/50%, 1J/50%"
        );
    }

    #[test]
    fn bands_are_coloured_at_the_asked_boundaries() {
        assert_eq!(band_colour(0), Color::LightMagenta);
        assert_eq!(band_colour(-1), Color::LightMagenta, "nullsec and below");
        assert_eq!(band_colour(1), Color::Red);
        assert_eq!(band_colour(4), Color::Red, "lowsec ends at 0.4");
        assert_eq!(band_colour(5), Color::Yellow);
        assert_eq!(band_colour(6), Color::Green);
        assert_eq!(band_colour(8), Color::Green);
        assert_eq!(band_colour(9), Color::Cyan);
        assert_eq!(band_colour(10), Color::Cyan, "1.0 is the top of the scale");
    }

    #[test]
    fn a_band_is_coloured_and_a_true_security_is_not() {
        // The system's own security is printed to two decimals and belongs to the agent, not to a
        // band, so it stays in the line's style while the band groups beside it are coloured.
        let spans = coloured("  Babirmoult (0.18) 0.1/50%, 0.2/50%", Style::new());
        let texts: Vec<String> = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(
            texts,
            ["  Babirmoult (0.18) ", "0.1", "/50%, ", "0.2", "/50%"]
        );
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[3].style.fg, Some(Color::Red));
        assert_eq!(
            spans[0].style.fg, None,
            "the uncoloured run keeps the line style"
        );

        let route = coloured("    Moh 2J route 0.4  own 0.5", Style::new().dark_gray());
        let values: Vec<Option<Color>> = route.iter().map(|s| s.style.fg).collect();
        assert_eq!(
            values,
            vec![
                Some(Color::DarkGray),
                Some(Color::Red),
                Some(Color::DarkGray),
                Some(Color::Yellow),
            ]
        );
        assert_eq!(
            route.iter().map(|s| s.content.as_ref()).collect::<String>(),
            "    Moh 2J route 0.4  own 0.5",
            "colouring must not lose a character"
        );
    }

    #[test]
    fn a_negative_band_is_read_as_a_band() {
        let spans = coloured("  9-ERCP (30001484)  route -0.1/100%", Style::new());
        assert_eq!(spans[1].content, "-0.1");
        assert_eq!(spans[1].style.fg, Some(Color::LightMagenta));
        // Hops and percents are not bands, and neither is a whole number written without a decimal.
        let plain = coloured("  0J/100% 1J/50% x 10 stations", Style::new());
        assert_eq!(plain.len(), 1, "nothing in {plain:?} should split");
    }

    #[test]
    fn the_details_pane_says_why_no_systems_are_listed() {
        // Empty static data has no division table, so nothing can be put in scope; the pane states
        // that rather than printing corporation names with nothing under them.
        let app = results_app(one_priced_offer(), 10);
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Corporations"), "the section is there");
        assert!(
            text.contains("the static data names no Security division"),
            "and says why it is empty: {text}"
        );
    }

    #[test]
    fn the_new_place_fields_still_leave_room_for_a_hint() {
        // The height this view accepts was raised when the form grew to twelve fields, so that the
        // notes under them — hints, valid values, and every validation error — keep a row. If the
        // form ever fills the dialog again, this fails rather than the error going unseen.
        let mut app = App::new(LpStoreOptions::default());
        let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        app.show_filters(form);
        for focus in [FormField::AgentRegion, FormField::AgentSystem] {
            app.form.focus = focus;
            let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
            let hint = hint_line(focus, &StaticData::default());
            assert!(
                text.contains(&hint),
                "{focus:?}'s hint was pushed out of a {}x{} dialog: {text}",
                MIN_WIDTH,
                MIN_HEIGHT
            );
        }
    }

    #[test]
    fn a_field_error_on_a_place_filter_is_visible_at_the_minimum_size() {
        let mut app = App::new(LpStoreOptions::default());
        let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        app.show_filters(form);
        app.form.focus = FormField::AgentRegion;
        app.form.set_errors(vec![OptionError {
            field: Field::AgentRegion,
            message: "Region 'Black Riss' not found".to_string(),
        }]);
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        assert!(
            text.contains("Region 'Black Riss' not found"),
            "the error is on screen, not cut by the field rows: {text}"
        );
    }

    #[test]
    fn the_place_fields_offer_name_matches_like_the_market_ones() {
        // The suggestions come from the static data, so an empty cache says so rather than nothing.
        for (field, typed) in [
            (FormField::AgentRegion, "bla"),
            (FormField::AgentSystem, "jit"),
        ] {
            let mut app = App::new(LpStoreOptions::default());
            let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
            form.focus = field;
            match field {
                FormField::AgentRegion => form.agent_region = typed.to_string(),
                _ => form.agent_system = typed.to_string(),
            }
            app.show_filters(form);
            let shown = suggestions(&app, &StaticData::default()).expect("a name field suggests");
            assert_eq!(shown, "No matches", "{field:?} on empty data: {shown}");
        }
    }

    /// A key press, for the handful of tests that need to press a key.
    fn press(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    /// Results on screen, then a second run started over them: the state a re-applied filter leaves.
    fn rerun_app(done: usize, total: usize) -> App {
        let mut app = results_app(one_priced_offer(), 10);
        app.start_run(RunInfo {
            region: "The Forge".to_string(),
            region_id: 10000002,
            agent_region_id: None,
            corporations: 12,
            filters: "--security 0.4..0.5".to_string(),
            depth: 10,
            agent_filter: AgentFilter::default(),
        });
        app.on_event(
            &eve_lpstore::report::Event::FetchProgress { done, total },
            None,
        );
        app
    }

    #[test]
    fn a_re_run_shows_the_progress_popup_over_the_results() {
        let app = rerun_app(3, 12);
        let text = view_text(&app, 120, 40);
        assert!(text.contains(" Run "), "the popup is titled: {text}");
        assert!(
            text.contains("Fetching loyalty offers: 3 / 12 corporations"),
            "the stage line is in it: {text}"
        );
        assert!(
            text.contains("Pricing groups") && text.contains("waiting"),
            "both bars show, the idle one reading waiting: {text}"
        );
    }

    #[test]
    fn the_first_run_keeps_its_bars_inline() {
        // No results behind it, so there is nothing to cover and the progress view is already the
        // whole screen. The popup is only for a run that superseded something.
        let mut app = App::new(LpStoreOptions::default());
        app.begin_run(RunInfo {
            region: "The Forge".to_string(),
            region_id: 10000002,
            agent_region_id: None,
            corporations: 3,
            filters: "none".to_string(),
            depth: 10,
            agent_filter: AgentFilter::default(),
        });
        app.on_event(
            &eve_lpstore::report::Event::FetchProgress { done: 1, total: 3 },
            None,
        );
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("Fetching loyalty offers: 1 / 3 corporations"),
            "the inline bar still reports progress: {text}"
        );
        assert!(!text.contains(" Run "), "and no popup covers anything");
    }

    #[test]
    fn the_end_of_a_re_run_waits_for_a_key() {
        let mut app = rerun_app(12, 12);
        app.finish(Report {
            groups: vec![Group {
                corp_ids: vec![1000002],
                offer_count: 1,
                results: one_priced_offer(),
                errors: Vec::new(),
            }],
        });
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("Finished: 1 groups priced") && text.contains("any key closes this"),
            "the summary is shown, and says how it goes away: {text}"
        );
        // The same state at the smallest size the view accepts. The summary popup is ten rows, so a
        // screen this short is where it would be cut first, and the key line is its last row.
        let small = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        assert!(
            small.contains("any key closes this"),
            "the summary fits {}x{}: {small}",
            MIN_WIDTH,
            MIN_HEIGHT
        );
        app.on_key(press(crossterm::event::KeyCode::Esc));
        let text = view_text(&app, 120, 40);
        assert!(
            !text.contains("any key closes this"),
            "one key was enough: {text}"
        );
        assert!(
            text.contains("Corporations"),
            "the fresh results are now the view"
        );
    }

    #[test]
    fn an_open_overlay_wins_over_the_progress_popup() {
        let mut app = rerun_app(3, 12);
        app.overlay = Overlay::Log;
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains(" Log ("),
            "the log the user asked for is showing"
        );
        assert!(
            !text.contains(" Run "),
            "the popup does not stack over it: {text}"
        );
    }

    #[test]
    fn a_stopped_re_run_reports_itself_in_the_popup() {
        let mut app = rerun_app(3, 12);
        app.fail();
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("The run stopped unexpectedly"),
            "the failure is on the popup: {text}"
        );
        assert!(text.contains("any key closes this"), "and it waits");
    }

    #[test]
    fn the_missions_notes_fit_the_narrowest_view() {
        // The notes sit above the body and must not wrap, or they eat the rows they describe.
        for note in DESTINATION_NOTES {
            assert!(
                note.len() + 2 <= usize::from(MIN_WIDTH),
                "{} wraps at the minimum width",
                note
            );
        }
    }

    #[test]
    fn the_missions_pane_states_that_it_is_a_model() {
        let app = missions_app();
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("model, not measurement"),
            "the pane says the numbers are a model: {text}"
        );
        assert!(
            text.contains("security missions stay in the constellation"),
            "and names the scope it assumes: {text}"
        );
        assert!(
            text.contains("safest gate route"),
            "and says what a band means"
        );
    }

    #[test]
    fn scrolling_keeps_the_missions_notes_on_screen() {
        let mut app = missions_app();
        app.destinations_scroll = 50;
        app.destinations_rows = 60;
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("model, not measurement"),
            "the disclaimer is not part of the scrolled body"
        );
    }

    /// The results view with the destinations pane open.
    fn missions_app() -> App {
        let mut app = results_app(one_priced_offer(), 10);
        app.overlay = Overlay::Destinations;
        app
    }

    #[test]
    fn probabilities_are_whole_percent() {
        assert_eq!(percent(1.0 / 6.0), "17%");
        assert_eq!(percent(5.0 / 6.0), "83%");
        assert_eq!(percent(1.0), "100%");
        assert_eq!(percent(0.0), "0%");
    }

    #[test]
    fn the_missions_pane_reports_what_it_cannot_show() {
        // An empty static data names no divisions at all, which is the state before the data has
        // been fetched, and the pane must say that rather than print nothing or guess an id.
        let mut app = results_app(one_priced_offer(), 10);
        app.overlay = Overlay::Destinations;
        let text = view_text(&app, 120, 40);
        assert!(text.contains("Mission destinations"), "the pane is titled");
        assert!(
            text.contains("the static data names no Security division"),
            "it says why there is nothing to list: {text}"
        );
    }

    #[test]
    fn the_scope_follows_the_division_the_run_asked_for() {
        assert_eq!(destination_scope(None, None), Scope::Unknown, "no data");
        assert_eq!(
            destination_scope(None, Some(22)),
            Scope::Unknown,
            "a division filter cannot conjure a division table"
        );
        assert_eq!(destination_scope(Some(24), None), Scope::Included);
        assert_eq!(destination_scope(Some(24), Some(24)), Scope::Included);
        assert_eq!(
            destination_scope(Some(24), Some(22)),
            Scope::Excluded(22),
            "Distribution is courier work, outside the model"
        );
    }

    #[test]
    fn an_excluded_run_is_told_which_division_it_asked_for() {
        assert_eq!(
            excluded_row("Distribution", 78),
            "Distribution agents are outside this model: security missions only"
        );
    }

    #[test]
    fn the_longest_division_name_still_fits_the_pane() {
        // "Industrialist - Entrepreneur" is 28 characters, and with a fixed sentence it would have overrun
        // the 78 columns a border leaves at the minimum width, where the line is cut without an
        // ellipsis because the pane does not wrap. The name gives way instead of the message.
        let width = usize::from(MIN_WIDTH) - 2;
        for division in [
            "Security",
            "Industrialist - Entrepreneur",
            "Soldier of Fortune",
            "a name far longer than any division in the archive at all",
        ] {
            let row = excluded_row(division, width);
            assert!(
                row.chars().count() <= width,
                "{row} overruns {width} columns"
            );
            assert!(
                row.ends_with("security missions only"),
                "the reason is never the part that gets cut: {row}"
            );
        }
        assert_eq!(
            excluded_row("Industrialist - Entrepreneur", width),
            "Industrialist - Entrepr… agents are outside this model: security missions only"
        );
    }

    #[test]
    fn the_missions_pane_names_a_group_only_after_a_selection() {
        let mut app = App::new(LpStoreOptions::default());
        app.overlay = Overlay::Destinations;
        let text = view_text(&app, 120, 40);
        assert!(
            text.contains("No group selected."),
            "empty view is explicit"
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
        // The fields these changes worded have to fit the detail column at the minimum terminal
        // width, so their extra wording goes on the second line. Other hints are longer and already
        // truncate there, which is existing behaviour this test does not cover.
        assert_eq!(
            hint_line(FormField::Security, &StaticData::default()),
            "0.5 exact, 0.5..0.7 for a range."
        );
        let detail = usize::from(MIN_WIDTH - 2) - (LABEL_WIDTH + VALUE_WIDTH + 2);
        for field in [
            FormField::Region,
            FormField::System,
            FormField::Security,
            FormField::AgentRegion,
            FormField::AgentSystem,
            FormField::CacheMinutes,
        ] {
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
            "Agent region",
            "Agent system",
            "Agent level",
            "Division",
            "Agent type",
            "Faction",
            "Market region",
            "Market system",
            "Depth",
            "Cache minutes",
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

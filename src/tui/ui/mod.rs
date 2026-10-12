//! Drawing the `--tui` view. These functions read the app state and draw it, and they change
//! nothing. `draw` returns how the details pane was laid out, so the app can scroll it.

use super::app::{App, ExportPrompt, Focus, LogLine, Overlay, Stage, StaticDataDialog};
use super::form::FormField;
use super::qr;
use crate::output;
use eve_lpstore::esi::ClientStats;
use eve_lpstore::format::{format_isk, format_number, format_optional};
use eve_lpstore::report::{Field, Group};
use eve_lpstore::{OfferResult, StaticData};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use std::collections::{HashSet, VecDeque};

mod destinations;
mod details;
mod filters;
mod offers;
mod popups;
mod progress;
#[cfg(test)]
mod tests;
mod text;

use destinations::*;
use details::*;
use filters::*;
use offers::*;
use popups::*;
use progress::*;
use text::*;
// The form's tests check that every label fits this column.
#[cfg(test)]
pub(crate) use filters::LABEL_WIDTH;
pub(crate) use popups::UNKNOWN_DATA_DIR;

pub const MIN_WIDTH: u16 = 80;

/// The smallest view the app draws. Twelve filter rows need room, and the notes under them carry
/// the hints, the valid values and every validation error: at 20 rows the filters dialog had two
/// rows of notes left, and adding the two agent place filters would have taken them to zero, which
/// is a form that cannot report a mistake. 24 leaves four.
pub const MIN_HEIGHT: u16 = 24;

/// What the last frame measured about the scrollable panes, fed back so the keys can bound the
/// scroll without the view holding the static data it renders from.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaneLayout {
    pub detail_rows: usize,
    pub detail_visible: usize,
    pub destinations_rows: usize,
    pub destinations_visible: usize,
    pub help_rows: usize,
    pub help_visible: usize,
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
    let corporations = if app.corporations() == 1 {
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
            app.region(),
            app.corporations(),
            corporations,
            app.filters
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

    let mut layout = match &app.overlay {
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
    // The footer before the popups, so one tall enough to reach it covers it, rather than the
    // footer being drawn over the popup's last rows.
    draw_footer(frame, footer, stats, app);
    match &app.overlay {
        Overlay::Help => {
            (layout.help_rows, layout.help_visible) = draw_help(frame, area, app.help_scroll);
        }
        Overlay::Donate => draw_donate(frame, area),
        Overlay::Export(prompt) => draw_export(frame, area, prompt),
        Overlay::StaticData(dialog) => draw_static_data(frame, area, dialog, &app.data_dir),
        _ => {}
    }
    // Last, so any overlay the user asked for wins; the run's own popup is not something they opened.
    if app.overlay == Overlay::None && (app.awaiting_run_summary || in_flight_run(app)) {
        draw_run_progress(frame, area, app);
    }
    layout
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
        ..PaneLayout::default()
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

fn draw_footer(frame: &mut Frame, area: Rect, stats: &ClientStats, app: &App) {
    let keys = match &app.overlay {
        Overlay::Filters => {
            "Tab/Shift+Tab field  Enter apply  Esc cancel  Ctrl+U clear  Ctrl+C quit"
        }
        Overlay::Export(_) => "Enter write  Esc cancel  Ctrl+U clear  Ctrl+C quit",
        Overlay::Destinations => "Up/Down scroll  Esc close  Ctrl+C quit",
        Overlay::StaticData(dialog) => {
            if dialog.downloading {
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
        // The errors come before the limits, so a narrow footer cuts the limits, not the errors.
        Line::from(format!(
            "In-flight: {} | Waiting: {} | Errors: {} | {}",
            stats.in_flight,
            stats.waiting,
            stats.total_errors,
            output::rate_limit_text(stats)
        )),
        Line::from(format!(
            "Cache: {} hits, {} misses, {} writes",
            stats.cache_hits, stats.cache_misses, stats.cache_writes
        )),
        Line::from(keys).dark_gray(),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

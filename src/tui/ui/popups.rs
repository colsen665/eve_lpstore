//! The popups drawn over the view: static data, run progress, export, help, and donate.

use super::*;

pub(super) const DONATION_ADDRESS: &str = "bc1qyc55ddky2ehdnty2c48nvxxugjw2xdpxk956x7";

/// The width of the popups that hold a line of text, such as a file name or a reason. Inside the
/// minimum terminal width, with room to spare on either side.
const POPUP_WIDTH: u16 = 64;

/// The text width inside a [`POPUP_WIDTH`] popup, between its borders.
const POPUP_TEXT: usize = POPUP_WIDTH as usize - 2;

/// Draws `lines` in a bordered popup titled `title`, centred in `area` and as tall as the lines.
pub(super) fn popup(frame: &mut Frame, area: Rect, title: &str, width: u16, lines: Vec<Line>) {
    let popup = area.centered(
        Constraint::Length(width),
        Constraint::Length(lines.len() as u16 + 2),
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title)),
        popup,
    );
}

/// Where the static data files live, for the dialog's text. A placeholder until the view is given
/// the real root, so the string is never empty and never wrong about a directory it has not seen.
pub(crate) const UNKNOWN_DATA_DIR: &str = "the data directory";

/// The static data dialog, shown when the files are not on disk, or are and cannot be read.
/// Downloading fetches the whole EVE Online archive, so the view asks before spending the time on
/// it instead of finding out on a terminal that has gone quiet.
pub(super) fn draw_static_data(
    frame: &mut Frame,
    area: Rect,
    dialog: &StaticDataDialog,
    data_dir: &str,
) {
    let inner = POPUP_TEXT;
    let mut lines = Vec::new();
    if dialog.downloading {
        lines.push(Line::from("Downloading the EVE Online static data…"));
    } else {
        lines.push(Line::from(
            "The EVE Online static data is needed to price loyalty offers.",
        ));
    }
    if let Some(reason) = &dialog.error {
        lines.push(Line::from(truncate(reason, inner)).red());
    }
    lines.push(Line::from(
        truncate(&format!("It is stored in {}.", data_dir), inner).dark_gray(),
    ));
    let keys = if dialog.downloading {
        "Ctrl+C quits"
    } else {
        "Enter or d downloads it  Esc exits"
    };
    lines.push(Line::from(truncate(keys, inner)).dark_gray());
    popup(frame, area, " Static data ", POPUP_WIDTH, lines);
}

/// Whether a run is in flight over the results already on screen. `stale` says a report exists and a
/// newer run replaced it; the stage says that run has not finished.
pub(super) fn in_flight_run(app: &App) -> bool {
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
pub(super) fn draw_run_progress(frame: &mut Frame, area: Rect, app: &App) {
    let summary = app.awaiting_run_summary;
    // Eight inner rows for the stage line and the two bars, plus one for the key line only when there
    // is something to acknowledge, plus the border.
    let popup = area.centered(
        Constraint::Length(POPUP_WIDTH),
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
        Paragraph::new(truncate(&app.stage_text(), POPUP_TEXT)).style(stage_style),
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
    gauge(frame, pricing, "Pricing offers", price_done, price_total);
    if summary {
        frame.render_widget(Paragraph::new("any key closes this").dark_gray(), keys);
    }
}

/// The export prompt: where the view on screen would be written, and why the last attempt was
/// refused. The results stay behind it, so the name can be read against what is showing. A name
/// too long for the popup shows its end, which is where the letters being typed are.
pub(super) fn draw_export(frame: &mut Frame, area: Rect, prompt: &ExportPrompt) {
    let inner = POPUP_TEXT;
    let mut lines = vec![
        Line::from("Write the current view to".dark_gray()),
        Line::from(tail(&prompt.path, inner)).bold(),
    ];
    if let Some(error) = &prompt.error {
        lines.push(Line::from(truncate(error, inner)).red());
    }
    lines.push(Line::from(
        truncate(
            "Enter writes it  Esc cancels  Ctrl+U clears the name",
            inner,
        )
        .dark_gray(),
    ));
    popup(frame, area, " Export ", POPUP_WIDTH, lines);
}

/// The keys, then what the figures in the offers table are and how they are worked out. Taller
/// than the screen can be, so it scrolls: `scroll` is how many lines are above the window, and the
/// result is the help's total lines and how many showed, which the keys scroll against.
pub(super) fn draw_help(frame: &mut Frame, area: Rect, scroll: usize) -> (usize, usize) {
    let keys = [
        ("Tab", "switch between groups, offers, and details"),
        (
            "Up, Down, j, k",
            "move the selection, or scroll the details and this help",
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
    // The key column fits the longest key, `Up, Down, j, k`, and the popup the longest line. A
    // figure's second line has an empty key, so it lines up under the first.
    let row = |(key, text): &(&str, &str)| Line::from(format!("{:<16}{}", key, text));
    let mut lines: Vec<Line> = keys.iter().map(row).collect();
    lines.push(Line::from(""));
    lines.push(Line::from("Figures").bold());
    lines.extend(HELP_FIGURES.iter().map(row));

    let visible = usize::from(area.height.saturating_sub(2)).min(lines.len());
    let (start, end) = pane_window(lines.len(), scroll, visible);
    let title = if visible < lines.len() {
        " Help, Up/Down to scroll "
    } else {
        " Help "
    };
    let total = lines.len();
    popup(
        frame,
        area,
        title,
        HELP_WIDTH,
        lines.drain(start..end).collect(),
    );
    (total, visible)
}

/// The help's width: the screen's minimum, so a figure's formula fits on one 78-column line.
const HELP_WIDTH: u16 = MIN_WIDTH;

/// What each figure in the offers table means, as the code works it out: the reward and the
/// stations in `lpstore`, and the colour bands beside `SPREAD_WIDE` and `IMBALANCE_STRONG`.
pub(super) const HELP_FIGURES: [(&str, &str); 11] = [
    (
        "ISK/LP",
        "(reward × units − ISK cost − items) ÷ LP cost, per redemption;",
    ),
    (
        "",
        "reward at the best buy order, items at the best sell orders",
    ),
    (
        "Spread",
        "(best ask − best bid) ÷ best bid. Green to 10%, yellow to 50%,",
    ),
    (
        "",
        "red past it: the bid may be one stale order. Magenta: inverted",
    ),
    (
        "Imbal",
        "(bid − ask volume) ÷ their sum, within 10% of each best price.",
    ),
    (
        "",
        "Green from +25%: buyers outweigh sellers; red from −25%",
    ),
    (
        "Vol ±10%",
        "units buy orders take at 90% or more of the best bid",
    ),
    (
        "Profit ±10%",
        "selling them: (price − unit cost) × units, unit cost being",
    ),
    ("", "(ISK cost + items) ÷ units"),
    (
        "n/a",
        "a side of the market is missing, or nothing to compute from",
    ),
    (
        "Prices",
        "from the Market region, or only the Market system's stations",
    ),
];

/// The donation popup: the address as a QR code, with the address as text under it. When the
/// terminal is too small for the code, the popup shows only the address and a note.
pub(super) fn draw_donate(frame: &mut Frame, area: Rect) {
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

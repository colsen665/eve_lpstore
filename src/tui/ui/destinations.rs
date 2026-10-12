//! The mission destinations pane and the destination summaries the details pane shares.

use super::*;

/// One line per probability group: whole percents, since the counts are small and a decimal
/// suggests a precision the model does not have.
pub(super) fn percent(part: f64) -> String {
    format!("{:.0}%", part * 100.0)
}

/// The destination table as plain text, one line per system and per destination. Split out from the
/// drawing so the wording is testable without a frame.
fn destination_rows(app: &App, sdata: &StaticData, width: usize) -> Vec<String> {
    let Some(group) = app.selected_group() else {
        return vec!["No group selected.".to_string()];
    };
    let mut rows = Vec::new();
    let scope = destination_scope(sdata.security_division_id(), app.agent_filter().division_id);
    if scope != Scope::Included {
        return vec![scope_note(sdata, scope, width)];
    }
    for corp_id in &group.corp_ids {
        rows.push(output::corporation_label(sdata, *corp_id));
        let systems = sdata.security_agent_systems(*corp_id, &app.agent_filter());
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
pub(super) fn system_lines(
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
pub(super) fn band_colour(tenths: i32) -> Color {
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
pub(super) fn coloured(text: &str, base: Style) -> Vec<Span<'static>> {
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
pub(super) fn cap_systems<T>(systems: Vec<T>) -> (Vec<T>, usize) {
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
pub(super) fn more_systems_row(hidden: usize) -> String {
    let word = if hidden == 1 { "system" } else { "systems" };
    format!("  +{hidden} more {word}, press m for all of them")
}

/// The details-pane line for one agent system: its name and true security, then the band groups and
/// the hop groups. The destinations pane spells the same numbers with the system id and the word
/// `route`; here the row sits under its corporation, so the security stands in for the id.
pub(super) fn summary_text(
    name: &str,
    true_sec: f64,
    spread: &eve_lpstore::static_data::DestinationSpread,
) -> String {
    let (bands, hops) = distribution_parts(spread);
    format!("  {name} ({true_sec:.2}) {bands}  {hops}")
}

/// A separate burner marginal, shown only when a matching level-4 Basic Security agent is at the
/// system. Its indented label keeps it visually apart from the regular-mission distribution above.
pub(super) fn burner_summary_text(spread: &eve_lpstore::static_data::DestinationSpread) -> String {
    let (bands, hops) = distribution_parts(spread);
    format!("    Burners: {bands}  {hops}")
}

fn distribution_parts(spread: &eve_lpstore::static_data::DestinationSpread) -> (String, String) {
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
    (bands, hops)
}

/// Tenths to the one-decimal label the panes and `--security` both use.
fn band_tenths(tenths: i32) -> String {
    format!("{:.1}", tenths as f64 / 10.0)
}

/// The pane's standing lines, kept above the scrolling body so the disclaimer cannot be scrolled
/// out of sight. Both are 74 characters or fewer, which is inside the 78 columns a border leaves at
/// the minimum supported width, so neither wraps and neither eats a row of the body.
pub(super) const DESTINATION_NOTES: [&str; 2] = [
    "model, not measurement: security missions stay in the constellation",
    "bands are the lowest security on the safest gate route, not a system's own",
];

/// The one-line reason the systems are missing, for whichever view asks.
pub(super) fn scope_note(sdata: &StaticData, scope: Scope, width: usize) -> String {
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
pub(super) enum Scope {
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
pub(super) fn destination_scope(security: Option<i32>, asked: Option<i32>) -> Scope {
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

pub(super) fn excluded_row(division: &str, width: usize) -> String {
    let budget = width.saturating_sub(EXCLUDED_TAIL.chars().count());
    format!("{}{EXCLUDED_TAIL}", truncate(division, budget))
}

/// Which of the pane's rows are on screen. The window starts at the top of the table, so a pane that
/// opens with nothing scrolled shows the first corporation and its systems rather than the tail of
/// the list, and a scroll past the end is pulled back until the last row is on screen.
pub(super) fn pane_window(rows: usize, scroll: usize, height: usize) -> (usize, usize) {
    if height == 0 {
        return (0, 0);
    }
    let start = scroll.min(rows.saturating_sub(height));
    (start, (start + height).min(rows))
}

/// The pane's rows, each given the style its indent asks for and cut to the pane's width. A
/// corporation names itself at column 0, a system indents two, a destination four, and the
/// indentation survives the cut so a wrapped row cannot be mistaken for the start of a new entry.
pub(super) fn pane_body(rows: &[String], width: usize) -> Vec<(String, Style)> {
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

/// The mission destinations pane for the selected group. Probabilities are even shares of the
/// systems in the agent system's constellation, which is an assumption this tool makes rather than a
/// measurement of how missions are routed, and the band a destination lands in is the lowest
/// security on the safest way to reach it, which is why it can sit below the destination's own.
pub(super) fn draw_destinations(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    sdata: &StaticData,
) -> PaneLayout {
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
        // The count the keys scroll against is the wrapped rows, not the logical lines.
        destinations_rows: body.len(),
        destinations_visible: height,
        ..PaneLayout::default()
    }
}

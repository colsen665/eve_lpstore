//! The groups list and the offers table, with the spread and imbalance formatting.

use super::*;

/// The offers pane is at least this many columns wide, including its borders, for the wide layout.
/// Narrower panes leave out the imbalance and volume columns. The spread column shows in both.
///
/// This is measured, not derived: at 72 the last column's header is cut, and at 73 it is not. The
/// render tests below are what keep it honest.
pub(super) const WIDE_OFFERS: u16 = 73;

/// A spread up to this percentage means the market is tight enough to trust the best bid.
pub(super) const SPREAD_TIGHT: f64 = 10.0;

/// A spread past this percentage means the best bid is probably a single stale order, and the
/// ISK per LP built on it is optimistic.
pub(super) const SPREAD_WIDE: f64 = 50.0;

/// An imbalance at least this far from balanced is strong enough to colour. Closer to even than
/// this stays yellow, so a book that favours nobody is not reported as a win.
pub(super) const IMBALANCE_STRONG: f64 = 25.0;

pub(super) fn draw_groups(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let block = pane_block(
        &groups_title(app, usize::from(area.width.saturating_sub(2))),
        app.focus == Focus::Groups,
        app,
    );
    let block = match search_title(app, usize::from(area.width.saturating_sub(2))) {
        Some(title) => block.title_bottom(title),
        None => block,
    };
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
        .map(|group| group_item(group, &app.arrangement, sdata, names_width))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(app.group));
    frame.render_stateful_widget(list, area, &mut state);
}

/// The groups pane's title, before [`pane_block`] marks it stale: what the groups are sorted by,
/// and an arrow pointing down when the best go first and up when the worst do. It names the sort
/// by its `--sort` word, which keeps it inside the pane at 80 columns, where the pane is 26 cells
/// wide with 24 between its corners. When the stale mark would not fit beside it in `room`
/// cells, the pane's name is left out so the sort and the mark stay whole.
pub(super) fn groups_title(app: &App, room: usize) -> String {
    let arrangement = &app.arrangement;
    let arrow = if arrangement.reverse { '▲' } else { '▼' };
    let sort = format!("{} {}", arrangement.sort.name(), arrow);
    let title = format!("Groups: {sort}");
    // The stale mark adds " (stale)", and the padding a space on either side.
    if app.stale && title.chars().count() + 10 > room {
        sort
    } else {
        title
    }
}

/// The groups pane's bottom title while a search is set: ` /query `, or ` /query: no match `
/// in red after `n` or `N` found nothing. The query is cut to fit `room` cells.
pub(super) fn search_title(app: &App, room: usize) -> Option<Line<'static>> {
    let search = app.search.as_ref()?;
    let note = if search.missed { ": no match" } else { "" };
    // The slash, the note, and a space on either side.
    let query = truncate(&search.query, room.saturating_sub(note.chars().count() + 3));
    let text = format!(" /{query}{note} ");
    Some(if search.missed {
        Line::from(text).red()
    } else {
        Line::from(text).dark_gray()
    })
}

/// The value a group shows for `sort`, with the words after it: the ISK per LP figures with two
/// decimals, the ISK amounts shortened the way the offers table shortens them.
pub(super) fn sort_value_text(group: &Group, arrangement: &Arrangement) -> String {
    let value = group.sort_value(arrangement.sort, arrangement.depth);
    match arrangement.sort {
        GroupSort::AverageIskPerLp => format!("{} avg ISK/LP", optional(value)),
        GroupSort::BestOffer => format!("{} best ISK/LP", optional(value)),
        GroupSort::BudgetIsk => format!("{} ISK for budget", short_optional(value)),
        GroupSort::ProfitWithin10 => format!("{} profit ±10%", short_optional(value)),
    }
}

/// [`short_isk`] for a value that may be unknown, which reads `n/a`.
fn short_optional(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), short_isk)
}

/// A group takes two lines: the value it is sorted by, its offer count, then its corporations.
fn group_item(
    group: &Group,
    arrangement: &Arrangement,
    sdata: &StaticData,
    names_width: usize,
) -> ListItem<'static> {
    let names: Vec<String> = group
        .corp_ids
        .iter()
        .map(|&id| corp_name(sdata, id))
        .collect();
    ListItem::new(vec![
        Line::from(format!(
            "{} | {} offers",
            sort_value_text(group, arrangement),
            group.offer_count_text()
        )),
        Line::from(Span::styled(
            truncate(&names.join(", "), names_width),
            Style::new().dark_gray(),
        )),
    ])
}

/// The offers pane's title, before [`pane_block`] marks it stale. It names the table's sort
/// unless that is the ranking, with an arrow as the groups title has it. With hiding on it counts
/// the offers the filters hide in the selected group's stores, when there are any. With hiding off
/// it says so, and counts the offers the filters would hide, which are the rows drawn dim. When
/// the title and the stale mark would not fit in `room` cells, the pane's name is left out.
pub(super) fn offers_title(app: &App, room: usize) -> String {
    let mut parts = Vec::new();
    let sort = app.offer_sort;
    if sort != OfferSort::default() {
        parts.push(format!("{} {}", sort.key.name(), sort.arrow()));
    }
    if let (Some(report), Some(group)) = (app.report.as_ref(), app.selected_group()) {
        let (_, most) = report.hideable_range(group);
        let count = report.hideable_count_text(group);
        match (app.arrangement.hiding, most) {
            (true, 0) => {}
            (true, _) => parts.push(format!("{count} hidden")),
            (false, 0) => parts.push("hiding off".to_string()),
            (false, _) => parts.push(format!("hiding off, {count} would hide")),
        }
    }
    if parts.is_empty() {
        return "Offers".to_string();
    }
    let state = parts.join(", ");
    let title = format!("Offers: {state}");
    // The padding is a space on either side, and the stale mark adds " (stale)".
    let marks = if app.stale { 10 } else { 2 };
    if title.chars().count() + marks > room {
        state
    } else {
        title
    }
}

pub(super) fn draw_offers(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
    let title = offers_title(app, usize::from(area.width.saturating_sub(2)));
    let block = pane_block(&title, app.focus == Focus::Offers, app);
    if app.selected_group().is_none() {
        frame.render_widget(Paragraph::new("").block(block), area);
        return;
    }
    let wide = area.width >= WIDE_OFFERS;
    let arrangement = &app.arrangement;
    // In the table's order; the `#` column keeps each offer's place in the ranking.
    let rows: Vec<Row> = app
        .offer_rows()
        .into_iter()
        .map(|(rank, result)| {
            let item = item_name(sdata, result.offer.type_id);
            let cells = offer_cells(rank, result, item, wide)
                .into_iter()
                .map(|(text, style)| Cell::from(text).style(row_style(arrangement, result, style)));
            Row::new(cells)
        })
        .collect();
    let columns = offer_columns(wide);
    let header =
        Row::new(columns.iter().map(|(column, _)| column.title)).style(Style::new().bold());
    let table = Table::new(rows, columns.iter().map(|(_, width)| *width))
        .header(header)
        .block(block)
        .row_highlight_style(Style::new().reversed())
        .highlight_symbol("> ");
    let mut state = TableState::default().with_selected(Some(app.offer));
    frame.render_stateful_widget(table, area, &mut state);
}

/// The style of a cell of `result`'s row, given the cell's own `style`. With hiding off, a row the
/// filters would hide is dim throughout, its coloured cells too, so it reads as set aside rather
/// than as a market warning, and an excluded row is struck through as well, so it reads as the one
/// `x` set aside. With hiding on neither is listed, and every cell keeps its own style.
pub(super) fn row_style(arrangement: &Arrangement, result: &OfferResult, style: Style) -> Style {
    if arrangement.hiding {
        return style;
    }
    let excluded = arrangement.excluded.contains(&result.offer.offer_id);
    if excluded {
        Style::new().dark_gray().crossed_out()
    } else if !arrangement.filter.passes(result) {
        Style::new().dark_gray()
    } else {
        style
    }
}

/// One column of the offers table: its title, its width in each layout, and what a row shows in
/// it. `narrow` is `None` for a column the narrow layout leaves out.
pub(super) struct OfferColumn {
    pub(super) title: &'static str,
    wide: Constraint,
    narrow: Option<Constraint>,
    cell: fn(&OfferLine) -> (String, Style),
}

/// What the cells of one offers row are read from.
struct OfferLine<'a> {
    rank: usize,
    result: &'a OfferResult,
    item: String,
}

/// The offers table. The imbalance and the volume within 10% of the best bid only show in the wide
/// layout, the spread in both, and the profit is the total on that volume.
const OFFER_COLUMNS: [OfferColumn; 7] = [
    OfferColumn {
        title: "#",
        wide: Constraint::Length(4),
        narrow: Some(Constraint::Length(4)),
        cell: |line| ((line.rank + 1).to_string(), Style::new()),
    },
    OfferColumn {
        title: "Item",
        wide: Constraint::Min(12),
        // A lower floor than the wide layout, so the fixed columns keep their width at an
        // 80-column terminal. Item is the only column that can shrink.
        narrow: Some(Constraint::Min(10)),
        cell: |line| (line.item.clone(), Style::new()),
    },
    OfferColumn {
        title: "ISK/LP",
        wide: Constraint::Length(12),
        narrow: Some(Constraint::Length(12)),
        cell: |line| (optional(line.result.isk_per_lp), Style::new()),
    },
    OfferColumn {
        title: "Spread",
        wide: Constraint::Length(8),
        narrow: Some(Constraint::Length(8)),
        cell: |line| {
            let spread = line.result.spread;
            (format_spread(spread), spread_style(spread))
        },
    },
    OfferColumn {
        title: "Imbal",
        wide: Constraint::Length(6),
        narrow: None,
        cell: |line| {
            let imbalance = line.result.imbalance;
            (format_imbalance(imbalance), imbalance_style(imbalance))
        },
    },
    OfferColumn {
        title: "Vol ±10%",
        wide: Constraint::Length(10),
        narrow: None,
        cell: |line| {
            let volume = line.result.within_10.map(|(volume, _)| volume as f64);
            (format_optional(volume, 0), Style::new())
        },
    },
    OfferColumn {
        title: "Profit ±10%",
        wide: Constraint::Length(11),
        narrow: Some(Constraint::Length(11)),
        cell: |line| {
            let profit = line.result.within_10.map(|(_, profit)| short_isk(profit));
            (profit.unwrap_or_else(|| "n/a".to_string()), Style::new())
        },
    },
];

/// The columns a layout shows, with their widths in it.
pub(super) fn offer_columns(wide: bool) -> Vec<(&'static OfferColumn, Constraint)> {
    OFFER_COLUMNS
        .iter()
        .filter_map(|column| Some((column, if wide { column.wide } else { column.narrow? })))
        .collect()
}

/// The text and style of each cell of an offer row, in column order.
pub(super) fn offer_cells(
    rank: usize,
    result: &OfferResult,
    item: String,
    wide: bool,
) -> Vec<(String, Style)> {
    let line = OfferLine { rank, result, item };
    offer_columns(wide)
        .into_iter()
        .map(|(column, _)| (column.cell)(&line))
        .collect()
}

/// The spread as a percentage. An illiquid reward can spread by orders of magnitude, so anything
/// past 999% is capped to hold the column's width.
pub(super) fn format_spread(spread: Option<f64>) -> String {
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
pub(super) fn spread_style(spread: Option<f64>) -> Style {
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
pub(super) fn format_imbalance(imbalance: Option<f64>) -> String {
    match imbalance.map(|value| value.round()) {
        None => "n/a".to_string(),
        // The literal pattern compares with `==`, so it matches -0.0 as well.
        Some(0.0) => "0%".to_string(),
        Some(rounded) if rounded > 0.0 => format!("+{:.0}%", rounded),
        Some(rounded) => format!("{:.0}%", rounded),
    }
}

/// The colour of an imbalance. Green when buyers outweigh sellers, which is what an LP holder
/// selling into the market wants, and red when a wall of sellers sits over thin bids. A book too
/// close to even to favour either side stays yellow, and a missing one is dim rather than
/// alarming, since a reward nobody sells is not the same thing as a bad market.
pub(super) fn imbalance_style(imbalance: Option<f64>) -> Style {
    match imbalance {
        None => Style::new().dark_gray(),
        Some(value) if value >= IMBALANCE_STRONG => Style::new().green(),
        Some(value) if value <= -IMBALANCE_STRONG => Style::new().red(),
        Some(_) => Style::new().yellow(),
    }
}

/// ISK in about eight characters: billions, millions, or thousands, with two decimals.
pub(super) fn short_isk(value: f64) -> String {
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

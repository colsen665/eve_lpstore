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
    let isk = optional(group.average_isk_per_lp(depth));
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

pub(super) fn draw_offers(frame: &mut Frame, area: Rect, app: &App, sdata: &StaticData) {
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
        .map(|(rank, result)| {
            let item = item_name(sdata, result.offer.type_id);
            let cells = offer_cells(rank, result, item, wide)
                .into_iter()
                .map(|(text, style)| Cell::from(text).style(style));
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

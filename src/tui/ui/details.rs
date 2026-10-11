//! The details pane of the selected group and offer.

use super::*;

/// Draws the details pane, wrapped to its width, and returns (rows, rows on screen).
pub(super) fn draw_details(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    sdata: &StaticData,
) -> (usize, usize) {
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

/// The text of the details pane, one entry per logical line. `wrap_text` turns each entry into one
/// or more screen rows.
fn detail_entries(app: &App, sdata: &StaticData) -> Vec<(String, Style)> {
    let plain = Style::new();
    let bold = Style::new().bold();
    let Some(group) = app.selected_group() else {
        return Vec::new();
    };
    let mut lines = vec![("Corporations".to_string(), bold)];
    let scope = destination_scope(sdata.security_division_id(), app.agent_filter().division_id);
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
        let systems = sdata.security_agent_systems(id, &app.agent_filter());
        if systems.is_empty() {
            lines.push(("  no security agents match the filters".to_string(), plain));
            continue;
        }
        // A corporation can hold far more agent systems than the pane has rows for: measured on the
        // archive, 1 to 53 with a median of 11, and at 80 columns each summary wraps to two rows, so
        // the largest case pushed the offer breakdown 107 rows down. The systems past the cap are
        // named by count and pointed at the pane that lists them all.
        let (shown, hidden) = cap_systems(systems);
        let burner_systems: HashSet<i32> = sdata
            .burner_agent_systems(id, &app.agent_filter())
            .into_iter()
            .map(|(system, _, _)| system)
            .collect();
        for (system_id, name, true_sec) in shown {
            let text = match sdata.mission_spread(system_id) {
                Some(spread) => summary_text(&name, true_sec, &spread),
                None => format!("  {name} ({true_sec:.2})  no constellation data"),
            };
            lines.push((text, plain));
            if burner_systems.contains(&system_id) {
                let text = match sdata.burner_mission_spread(system_id) {
                    Some(spread) if spread.systems > 0 => burner_summary_text(&spread),
                    Some(_) => "    Burners: no eligible destination systems".to_string(),
                    None => "    Burners: no constellation data".to_string(),
                };
                lines.push((text, plain));
            }
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
                    optional(result.profit_per_unit())
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

use eve_lpstore::StaticData;
use eve_lpstore::esi::ClientStats;
use eve_lpstore::format::{format_isk, format_number, format_optional, group_digits};
use eve_lpstore::report::budget::Stop;
use eve_lpstore::report::{Event, Group, Report, Resolved};
use eve_lpstore::static_data::AgentFilter;
use std::fmt::Write;

/// The text for one event, and whether it is an error.
pub struct Message {
    pub text: String,
    pub is_error: bool,
}

/// How many of a corporation's agent systems are listed before the rest are summarised. The CLI
/// report cuts its inline list at this many, and the details pane caps its indented rows at the
/// same number, so the two views agree about which systems earn a row.
pub const MAX_SYSTEMS_SHOWN: usize = 10;

/// The corporation's name and id on its own, for panes that list systems separately.
pub fn corporation_label(sdata: &StaticData, corp_id: i32) -> String {
    match sdata.corporation_name(corp_id) {
        Some(name) => format!("{} ({})", name, corp_id),
        None => corp_id.to_string(),
    }
}

/// A corporation with its agent systems, as the CLI prints it.
pub fn corp_line(sdata: &StaticData, corp_id: i32, filter: &AgentFilter) -> String {
    let systems = sdata.corporation_systems(corp_id, filter);
    let mut line = corporation_label(sdata, corp_id);
    if !systems.is_empty() {
        let shown: Vec<String> = systems
            .iter()
            .take(MAX_SYSTEMS_SHOWN)
            .map(|(name, true_sec)| format!("{} ({:.2})", name, true_sec))
            .collect();
        line.push_str(": ");
        line.push_str(&shown.join(", "));
        if systems.len() > MAX_SYSTEMS_SHOWN {
            line.push_str(", ...");
        }
    }
    line
}

/// Prints what was resolved from the options, before the run starts.
pub fn print_resolution(resolved: &Resolved) {
    if let Some(summary) = &resolved.filter_summary {
        println!(
            "Filtered to {} corporations by agent, security, and faction filters ({} removed)",
            summary.kept, summary.removed
        );
    }
    // The agent-side places are named because they decided which corporations are below. The market
    // region on the next line is a different thing: where the prices are read from.
    match (&resolved.agent_region, &resolved.agent_system) {
        (Some(region), Some(system)) => {
            println!("Agents in {}, system {}", region.label, system.label);
        }
        (Some(region), None) => println!("Agents in {}", region.label),
        (None, Some(system)) => println!("Agents in {}", system.label),
        (None, None) => {}
    }
    println!("Region: {}", resolved.region_label);
    if resolved.corp_ids.len() > 1 {
        println!("Processing {} corporations...", resolved.corp_ids.len());
    }
    if let Some(system) = &resolved.system {
        println!(
            "Pricing from {} NPC stations in {}",
            system.stations, system.value
        );
    }
}

/// The text that describes `event`, or `None` for progress updates that have no text.
pub fn message(event: &Event, sdata: &StaticData) -> Option<Message> {
    let (text, is_error) = match event {
        Event::Log(line) => (line.clone(), false),
        Event::FetchStarted { corporations } => (
            format!(
                "Fetching loyalty offers for {} corporations...",
                corporations
            ),
            false,
        ),
        Event::FetchQueued { tasks } => (
            format!("Waiting for {} loyalty offer fetches to finish...", tasks),
            false,
        ),
        Event::FetchTaskFailed { error } => {
            (format!("Failed to fetch loyalty offers: {}", error), true)
        }
        Event::CorpFetchFailed { corp_id, error } => (
            format!(
                "Failed to fetch loyalty offers for corporation {}: {}",
                corporation_label(sdata, *corp_id),
                error
            ),
            true,
        ),
        Event::FetchFinished(summary) => (
            format!(
                "Fetched loyalty offers for {} corporations in {:.2}s, {} errors, {} cache hits, {} cache misses, {} cache writes",
                summary.corporations,
                summary.elapsed.as_secs_f64(),
                summary.errors,
                summary.cache.hits,
                summary.cache.misses,
                summary.cache.writes
            ),
            false,
        ),
        Event::PricingStarted { offers } => (format!("Pricing {} offers...", offers), false),
        Event::PricingQueued { tasks } => (
            format!("Waiting for {} offers to finish pricing...", tasks),
            false,
        ),
        Event::PricingFinished(summary) => (
            format!(
                "Priced {} offers in {:.2}s, {} offer errors, {} cache hits, {} cache misses, {} cache writes",
                summary.offers,
                summary.elapsed.as_secs_f64(),
                summary.offer_errors,
                summary.cache.hits,
                summary.cache.misses,
                summary.cache.writes
            ),
            false,
        ),
        Event::FetchProgress { .. } | Event::PricingProgress { .. } => return None,
    };
    Some(Message { text, is_error })
}

/// Prints one progress event as it arrives.
pub fn print_event(event: &Event, sdata: &StaticData) {
    let Some(line) = message(event, sdata) else {
        return;
    };
    if line.is_error {
        eprintln!("{}", line.text);
    } else {
        println!("{}", line.text);
    }
}

/// ESI's limits as the CLI and the TUI show them: the token budget of the route group with the
/// least left, and the older error limit, each once a response has reported it.
pub fn rate_limit_text(s: &ClientStats) -> String {
    let mut parts = Vec::new();
    if let Some(budget) = &s.budget {
        parts.push(format!(
            "Budget: {}/{} per {}",
            group_digits(&budget.remaining.to_string()),
            group_digits(&budget.limit.to_string()),
            budget.window
        ));
    }
    if let (Some(remain), Some(reset)) = (s.rate_limit_remain, s.rate_limit_reset) {
        parts.push(format!("Error limit: {remain} left, resets in {reset}s"));
    }
    if parts.is_empty() {
        "Rate limit: unknown".to_string()
    } else {
        parts.join(" | ")
    }
}

/// The client's counters on one line, as `--status` prints them. The TUI footer builds its own
/// shorter line, sharing only [`rate_limit_text`].
pub fn status_text(s: &ClientStats) -> String {
    format!(
        "In-flight: {} | Waiting: {} | {} | Total: {} | Errors: {}",
        s.in_flight,
        s.waiting,
        rate_limit_text(s),
        s.total_requests,
        s.total_errors
    )
}

/// Prints the offers that failed, each once, to standard error, then [`report_text`].
pub fn print_report(report: &Report, resolved: &Resolved, sdata: &StaticData) {
    for e in &report.errors {
        eprintln!("Failed to price offer {}: {}", e.offer_id, e.error);
    }
    print!("{}", report_text(report, resolved, sdata));
}

/// The groups in the order the report's arrangement sorts them, with the best group last, each
/// with up to the arrangement's depth of offers. When a threshold is set, the heading says the
/// offers listed are the ones that pass, and a group whose stores hide offers says how many.
pub fn report_text(report: &Report, resolved: &Resolved, sdata: &StaticData) -> String {
    let mut out = String::new();
    let multiple = resolved.corp_ids.len() > 1;
    let arrangement = report.arrangement();
    let depth = arrangement.depth;
    let filtered = !arrangement.filter.is_empty();
    // Writing to a String cannot fail, so the results of `writeln!` are ignored.
    for group in report.groups.iter().rev() {
        if multiple {
            let _ = writeln!(out, "\n=== Processing corporations ===");
            for &id in &group.corp_ids {
                let _ = writeln!(out, "{}", corp_line(sdata, id, &resolved.agent_filter));
            }
        }
        let _ = writeln!(out, "Loyalty offers: {} found", group.offer_count_text());
        if group.hidden_range().1 > 0 {
            let _ = writeln!(out, "Hidden by filters: {}", group.hidden_count_text());
        }
        if let Some(line) = budget_line(group, arrangement.lp_budget) {
            let _ = writeln!(out, "{}", line);
        }

        let passing = if filtered {
            " that pass the filters,"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "Top {} offers{} by ISK per LP (descending):",
            group.results.len().min(depth),
            passing
        );
        for (rank, r) in group.results.iter().take(depth).enumerate() {
            let item_name = sdata
                .type_name(r.offer.type_id)
                .cloned()
                .unwrap_or_else(|| format!("Type {}", r.offer.type_id));
            let reward_str = format_optional(r.reward, 2);
            let unit_cost_str = format_optional(r.unit_cost, 2);
            let profit_per_unit_str = format_optional(r.profit_per_unit(), 2);
            let isk_per_lp_str = format_optional(r.isk_per_lp, 2);
            let spread_str = r
                .spread
                .map(|v| format!("{:.2}%", v))
                .unwrap_or_else(|| "n/a".to_string());
            let imbalance_str = r
                .imbalance
                .map(|v| format!("{:+.2}%", v))
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(
                out,
                "{}. {}: LP={}, units per redemption={}, reward={}, unit cost={}, profit per unit={}, ISK/LP={}, spread={}, imbalance={}",
                rank + 1,
                item_name,
                format_number(r.offer.lp_cost as f64, 0),
                format_number(r.offer.quantity as f64, 0),
                reward_str,
                unit_cost_str,
                profit_per_unit_str,
                isk_per_lp_str,
                spread_str,
                imbalance_str
            );
            let _ = match r.within_10 {
                Some((volume, profit)) => writeln!(
                    out,
                    "   Volume within 10% of best bid: {}, total profit: {}",
                    format_number(volume as f64, 0),
                    format_isk(profit)
                ),
                None => writeln!(
                    out,
                    "   Volume within 10% of best bid: n/a, total profit: n/a"
                ),
            };
            for item in &r.breakdown.item_expenses {
                let name = sdata
                    .type_name(item.type_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Type {}", item.type_id));
                let _ = writeln!(
                    out,
                    "   {}: {} x {} ISK = {} ISK",
                    name,
                    format_number(item.quantity as f64, 0),
                    format_number(item.best_price, 2),
                    format_number(item.total_cost, 2)
                );
            }
        }
    }
    out
}

/// What spending `lp_budget` on the group's listed offers realizes, as one line, or `None` when
/// no budget is set. The view's details pane shows the same line.
pub(crate) fn budget_line(group: &Group, lp_budget: Option<u64>) -> Option<String> {
    let lp_budget = lp_budget?;
    let budget = format_number(lp_budget as f64, 0);
    let Some(fill) = group.budget else {
        return Some(format!(
            "LP budget {budget}: n/a, no buy orders take a whole redemption of these offers"
        ));
    };
    Some(format!(
        "LP budget {budget}: {} for {} LP, {} ISK/LP realized, {} redemptions ({})",
        format_isk(fill.isk),
        format_number(fill.lp_spent as f64, 0),
        format_optional(fill.isk_per_lp(), 2),
        format_number(fill.redemptions as f64, 0),
        stop_text(fill.stopped)
    ))
}

/// Why the ladder stopped, in words.
fn stop_text(stopped: Stop) -> &'static str {
    match stopped {
        Stop::Budget => "budget spent",
        Stop::Book => "book exhausted",
        Stop::Unprofitable => "rest unprofitable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_lpstore::OfferResult;
    use eve_lpstore::esi::Budget;
    use eve_lpstore::expenses::BookOrder;
    use eve_lpstore::report::{LpStoreOptions, Store};
    use std::sync::Arc;

    /// An offer of one unit for 100 LP and no ISK, with this ISK per LP and spread, whose reward
    /// sells into `book`, given as (price, volume) best first.
    fn offer(offer_id: i32, isk_per_lp: f64, spread: f64, book: &[(f64, i32)]) -> Arc<OfferResult> {
        let mut result = OfferResult {
            isk_per_lp: Some(isk_per_lp),
            spread: Some(spread),
            ..Default::default()
        };
        result.offer.offer_id = offer_id;
        result.offer.type_id = offer_id;
        result.offer.quantity = 1;
        result.offer.lp_cost = 100;
        result.buy_orders = book
            .iter()
            .map(|&(price, volume_remain)| BookOrder {
                price,
                volume_remain,
            })
            .collect();
        Arc::new(result)
    }

    /// The report text for `stores`, run with `options` on an empty `StaticData`, so
    /// corporations and items print as their IDs.
    fn text(stores: Vec<Store>, options: LpStoreOptions) -> String {
        let sdata = StaticData::default();
        let resolved = options.resolve(&sdata).expect("valid options");
        let mut report = Report::new(&resolved.corp_ids, stores, Vec::new(), resolved.depth);
        report.arrange(&resolved.arrangement());
        report_text(&report, &resolved, &sdata)
    }

    /// One corporation's store holding offer 1, which pays 10 ISK per LP with a spread of 80%
    /// and five units of buy orders, and offer 2, which pays 6 with a spread of 5% and a deep book.
    fn one_store() -> Vec<Store> {
        vec![Store::new(
            vec![1000002],
            vec![1, 2],
            vec![
                offer(1, 10.0, 80.0, &[(1000.0, 5)]),
                offer(2, 6.0, 5.0, &[(600.0, 100)]),
            ],
        )]
    }

    fn one_corporation() -> LpStoreOptions {
        LpStoreOptions {
            lpstore: Some("1000002".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn without_filters_the_report_lists_every_offer_and_the_budget_fill() {
        let text = text(one_store(), one_corporation());
        let lines: Vec<&str> = text.lines().take(3).collect();
        // Five redemptions of offer 1 empty its book, then five of offer 2 spend the rest.
        assert_eq!(
            lines,
            [
                "Loyalty offers: 2 found",
                "LP budget 1,000: 8,000.00 ISK for 1,000 LP, 8.00 ISK/LP realized, 10 \
                 redemptions (budget spent)",
                "Top 2 offers by ISK per LP (descending):",
            ]
        );
        assert!(!text.contains("Hidden"), "{text}");
    }

    #[test]
    fn with_a_filter_the_heading_says_so_and_the_hidden_offers_are_counted() {
        let options = LpStoreOptions {
            max_spread: Some("50".to_string()),
            lp_budget: Some("2k".to_string()),
            ..one_corporation()
        };
        let text = text(one_store(), options);
        let lines: Vec<&str> = text.lines().take(5).collect();
        // Offer 1 fails the spread filter, so only offer 2 is listed or redeemed.
        assert_eq!(
            lines,
            [
                "Loyalty offers: 2 found",
                "Hidden by filters: 1",
                "LP budget 2,000: 12,000.00 ISK for 2,000 LP, 6.00 ISK/LP realized, 20 \
                 redemptions (budget spent)",
                "Top 1 offers that pass the filters, by ISK per LP (descending):",
                "1. Type 2: LP=100, units per redemption=1, reward=n/a, unit cost=n/a, profit per \
                 unit=n/a, ISK/LP=6.00, spread=5.00%, imbalance=n/a",
            ]
        );
    }

    #[test]
    fn a_group_nobody_buys_from_says_its_budget_has_no_fill() {
        let stores = vec![Store::new(
            vec![1000002],
            vec![1],
            vec![offer(1, 10.0, 5.0, &[])],
        )];
        let text = text(stores, one_corporation());
        assert_eq!(
            text.lines().nth(1),
            Some("LP budget 1,000: n/a, no buy orders take a whole redemption of these offers")
        );
    }

    #[test]
    fn groups_print_in_the_sorts_order_with_the_best_last() {
        // 1000002 has the better ISK per LP, 1000003 the more ISK for the budget: 5,000 against
        // 6,000.
        let stores = || {
            vec![
                Store::new(
                    vec![1000002],
                    vec![1],
                    vec![offer(1, 10.0, 5.0, &[(1000.0, 5)])],
                ),
                Store::new(
                    vec![1000003],
                    vec![2],
                    vec![offer(2, 6.0, 5.0, &[(600.0, 100)])],
                ),
            ]
        };
        let corporations = |sort: Option<&str>| {
            let options = LpStoreOptions {
                lpstore: Some("1000002,1000003".to_string()),
                sort: sort.map(str::to_string),
                ..Default::default()
            };
            text(stores(), options)
                .lines()
                .filter(|line| line.starts_with("100000"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(corporations(None), ["1000003", "1000002"]);
        assert_eq!(corporations(Some("budget")), ["1000002", "1000003"]);
    }

    #[test]
    fn the_limits_read_as_the_budget_and_the_error_limit() {
        let mut stats = ClientStats::default();
        assert_eq!(rate_limit_text(&stats), "Rate limit: unknown");
        stats.rate_limit_remain = Some(100);
        stats.rate_limit_reset = Some(50);
        assert_eq!(
            rate_limit_text(&stats),
            "Error limit: 100 left, resets in 50s"
        );
        stats.budget = Some(Budget {
            group: "market-order".to_string(),
            remaining: 4940,
            limit: 12000,
            window: "15m".to_string(),
        });
        assert_eq!(
            rate_limit_text(&stats),
            "Budget: 4,940/12,000 per 15m | Error limit: 100 left, resets in 50s"
        );
        assert!(
            status_text(&stats).contains("| Budget: 4,940/12,000 per 15m |"),
            "{}",
            status_text(&stats)
        );
    }
}

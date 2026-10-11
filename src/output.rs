use eve_lpstore::StaticData;
use eve_lpstore::esi::ClientStats;
use eve_lpstore::format::{format_isk, format_number, format_optional};
use eve_lpstore::report::{Event, Report, Resolved};
use eve_lpstore::static_data::AgentFilter;

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
        Event::PricingStarted { groups } => (format!("Pricing {} groups...", groups), false),
        Event::PricingQueued { tasks } => (
            format!("Waiting for {} groups to finish pricing...", tasks),
            false,
        ),
        Event::GroupFailed { error } => (format!("Failed to price a group: {}", error), true),
        Event::PricingFinished(summary) => (
            format!(
                "Priced {} groups in {:.2}s, {} offer errors, {} group failures, {} cache hits, {} cache misses, {} cache writes",
                summary.groups,
                summary.elapsed.as_secs_f64(),
                summary.offer_errors,
                summary.group_failures,
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

/// The rate limit as the CLI and the TUI show it.
pub fn rate_limit_text(s: &ClientStats) -> String {
    match (s.rate_limit_remain, s.rate_limit_reset) {
        (Some(remain), Some(reset)) => format!("{}/200 (resets in {}s)", remain, reset),
        _ => "unknown".to_string(),
    }
}

/// The client's counters on one line, as `--status` prints them. The TUI footer builds its own
/// shorter line, sharing only [`rate_limit_text`].
pub fn status_text(s: &ClientStats) -> String {
    format!(
        "In-flight: {} | Waiting: {} | Rate limit: {} | Total: {} | Errors: {}",
        s.in_flight,
        s.waiting,
        rate_limit_text(s),
        s.total_requests,
        s.total_errors
    )
}

/// Prints the groups with up to `depth` offers each. The best group prints last.
pub fn print_report(report: &Report, resolved: &Resolved, sdata: &StaticData) {
    let multiple = resolved.corp_ids.len() > 1;
    let depth = resolved.depth;
    for group in report.groups.iter().rev() {
        if multiple {
            println!("\n=== Processing corporations ===");
            for &id in &group.corp_ids {
                println!("{}", corp_line(sdata, id, &resolved.agent_filter));
            }
        }
        println!("Loyalty offers: {} found", group.offer_count);
        for e in &group.errors {
            eprintln!("Failed to price offer {}: {}", e.offer_id, e.error);
        }

        println!(
            "Top {} offers by ISK per LP (descending):",
            group.results.len().min(depth)
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
            println!(
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
            match r.within_10 {
                Some((volume, profit)) => println!(
                    "   Volume within 10% of best bid: {}, total profit: {}",
                    format_number(volume as f64, 0),
                    format_isk(profit)
                ),
                None => println!("   Volume within 10% of best bid: n/a, total profit: n/a"),
            }
            for item in &r.breakdown.item_expenses {
                let name = sdata
                    .type_name(item.type_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Type {}", item.type_id));
                println!(
                    "   {}: {} x {} ISK = {} ISK",
                    name,
                    format_number(item.quantity as f64, 0),
                    format_number(item.best_price, 2),
                    format_number(item.total_cost, 2)
                );
            }
        }
    }
}

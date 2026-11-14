//! Writing the priced report out as a file, for a spreadsheet or another tool.
//!
//! Both shapes are rendered from one intermediate tree, so a field renamed in one is renamed in
//! the other, and the two column sets cannot drift apart. The export follows the report's
//! arrangement, as the `--tui` view does: groups in its sort order, best first, and only the top
//! `depth` offers of each group that its filters leave visible, in ranking order.
//!
//! Numbers are written plainly, without the thousands separators the terminal shows, so a
//! spreadsheet reads them as numbers. A value the run could not work out is empty in CSV and
//! `null` in JSON. There is no timestamp in the file, so the same report always exports to the
//! same bytes; put a date in the name instead.

use crate::lpstore::OfferResult;
use crate::report::{Arrangement, Group, GroupSort, Report};
use crate::static_data::StaticData;
use serde::Serialize;
use std::path::Path;

/// The file shapes an export can use, chosen by the extension of the name the user types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    /// The format `path` asks for. An unknown extension is `None`, so the caller can say so rather
    /// than guess a shape nobody asked for.
    pub fn from_path(path: &str) -> Option<Self> {
        let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Where an export goes, and in what shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub path: String,
    pub format: ExportFormat,
}

impl ExportRequest {
    /// Reads a typed name back into a request. A blank name or an unknown extension is `None`.
    pub fn parse(path: &str) -> Option<Self> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }
        let format = ExportFormat::from_path(path)?;
        Some(Self {
            path: path.to_string(),
            format,
        })
    }
}

/// What the file says about the run, which the report itself does not carry, and how the report
/// was arranged.
pub struct ExportContext<'a> {
    pub region: &'a str,
    pub filters: &'a str,
    pub depth: usize,
    pub sort: GroupSort,
    pub lp_budget: Option<u64>,
    /// Whether offers failing the filters, or excluded, were left out.
    pub hiding: bool,
}

impl<'a> ExportContext<'a> {
    /// The context of a report grouped and sorted for `arrangement`, which is the report's own
    /// [`Report::arrangement`] when the file is to match what was shown.
    pub fn new(region: &'a str, filters: &'a str, arrangement: &Arrangement) -> Self {
        Self {
            region,
            filters,
            depth: arrangement.depth,
            sort: arrangement.sort,
            lp_budget: arrangement.lp_budget,
            hiding: arrangement.hiding,
        }
    }
}

/// The columns of the CSV export, in order. The required items of an offer are a list, so they
/// only appear in the JSON export.
const CSV_COLUMNS: [&str; 28] = [
    "group",
    "average_isk_per_lp",
    "min_offer_count",
    "max_offer_count",
    "best_isk_per_lp",
    "profit_within_10_percent_total",
    "budget_isk",
    "budget_lp_spent",
    "budget_isk_per_lp",
    "min_hidden",
    "max_hidden",
    "corporations",
    "rank",
    "item",
    "type_id",
    "lp_cost",
    "units",
    "reward",
    "unit_cost",
    "profit_per_unit",
    "isk_per_lp",
    "best_ask",
    "spread_percent",
    "bid_depth",
    "ask_depth",
    "imbalance_percent",
    "volume_within_10_percent",
    "profit_within_10_percent",
];

/// The whole file, in the shape both exports render from.
#[derive(Serialize)]
struct ExportFile<'a> {
    region: &'a str,
    filters: &'a str,
    depth: usize,
    /// The [`GroupSort::name`] the groups are ordered by.
    sort: &'static str,
    lp_budget: Option<u64>,
    hiding: bool,
    groups: Vec<ExportGroup<'a>>,
    errors: Vec<ExportError<'a>>,
}

/// One corporation group, as the groups pane lists it. Its corporations' stores can hold
/// different numbers of offers, since a group only needs the offers it lists to be the same.
#[derive(Serialize)]
struct ExportGroup<'a> {
    rank: usize,
    average_isk_per_lp: Option<f64>,
    min_offer_count: usize,
    max_offer_count: usize,
    best_isk_per_lp: Option<f64>,
    profit_within_10_percent_total: Option<f64>,
    /// The fill of the LP budget over the listed offers. All three are `None` when no budget is
    /// set or no listed offer can be redeemed into a buy order; the ISK per LP also when no LP
    /// was spent.
    budget_isk: Option<f64>,
    budget_lp_spent: Option<u64>,
    budget_isk_per_lp: Option<f64>,
    /// The fewest and the most offers the arrangement hides in the group's corporations' stores.
    min_hidden: usize,
    max_hidden: usize,
    corporations: Vec<ExportCorp<'a>>,
    offers: Vec<ExportOffer<'a>>,
}

/// A corporation, with how many offers its store holds. The name is `None` when static data does
/// not know the id.
#[derive(Serialize)]
struct ExportCorp<'a> {
    id: i32,
    name: Option<&'a str>,
    offer_count: usize,
}

/// One loyalty offer, with the same figures the offers pane and the detail pane show.
#[derive(Serialize)]
struct ExportOffer<'a> {
    rank: usize,
    item: Option<&'a str>,
    type_id: i32,
    lp_cost: i32,
    units: i32,
    reward: Option<f64>,
    unit_cost: Option<f64>,
    profit_per_unit: Option<f64>,
    isk_per_lp: Option<f64>,
    best_ask: Option<f64>,
    spread_percent: Option<f64>,
    bid_depth: Option<i64>,
    ask_depth: Option<i64>,
    imbalance_percent: Option<f64>,
    volume_within_10_percent: Option<i64>,
    profit_within_10_percent: Option<f64>,
    required_items: Vec<ExportItem<'a>>,
}

/// An item the redemption charges for.
#[derive(Serialize)]
struct ExportItem<'a> {
    item: Option<&'a str>,
    type_id: i32,
    quantity: i32,
    price: f64,
    cost: f64,
}

/// An offer the run could not price, once however many stores hold it, with the corporations
/// whose stores do.
#[derive(Serialize)]
struct ExportError<'a> {
    offer_id: i32,
    error: String,
    corporations: Vec<ExportCorp<'a>>,
}

/// The report as a CSV document, one row per offer, with its group repeated on every row so the
/// rows stay usable after a sort or a filter. Rows are separated by newlines.
pub fn csv(report: &Report, context: &ExportContext, sdata: &StaticData) -> String {
    let file = build(report, context, sdata);
    let mut out = CSV_COLUMNS.join(",");
    out.push('\n');
    for group in &file.groups {
        for offer in &group.offers {
            out.push_str(&csv_row(group, offer).join(","));
            out.push('\n');
        }
    }
    out
}

/// The report as a JSON document, in the same order, with the required items, and the offers that
/// failed to price listed once each after the groups.
pub fn json(report: &Report, context: &ExportContext, sdata: &StaticData) -> String {
    let file = build(report, context, sdata);
    // Every field is a string, a number, a sequence, or an option of those, so this cannot fail.
    serde_json::to_string_pretty(&file).expect("the export shape always serializes")
}

/// The number of offers the export writes, which is what a completion message should report.
pub fn offer_count(report: &Report, depth: usize) -> usize {
    report
        .groups
        .iter()
        .map(|group| group.results.len().min(depth))
        .sum()
}

fn build<'a>(
    report: &'a Report,
    context: &ExportContext<'a>,
    sdata: &'a StaticData,
) -> ExportFile<'a> {
    ExportFile {
        region: context.region,
        filters: context.filters,
        depth: context.depth,
        sort: context.sort.name(),
        lp_budget: context.lp_budget,
        hiding: context.hiding,
        groups: report
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| export_group(group, index + 1, context.depth, sdata))
            .collect(),
        errors: report
            .errors
            .iter()
            .map(|error| ExportError {
                offer_id: error.offer_id,
                error: error.error.to_string(),
                corporations: corporations(report.corporations_with(error.offer_id), sdata),
            })
            .collect(),
    }
}

/// Corporations as (id, offer count) pairs, named.
fn corporations<'a>(
    corporations: impl IntoIterator<Item = (i32, usize)>,
    sdata: &'a StaticData,
) -> Vec<ExportCorp<'a>> {
    corporations
        .into_iter()
        .map(|(id, offer_count)| ExportCorp {
            id,
            name: sdata.corporation_name(id).map(String::as_str),
            offer_count,
        })
        .collect()
}

/// ISK and percentages carry two decimals in practice, and a mean of them is not exact in binary
/// floating point. Rounded here so the file shows `1263.89` rather than `1263.8899999999999`.
fn round_money(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn money(value: Option<f64>) -> Option<f64> {
    value.map(round_money)
}

fn export_group<'a>(
    group: &'a Group,
    rank: usize,
    depth: usize,
    sdata: &'a StaticData,
) -> ExportGroup<'a> {
    let (min_hidden, max_hidden) = group.hidden_range();
    ExportGroup {
        rank,
        average_isk_per_lp: money(group.average_isk_per_lp(depth)),
        min_offer_count: group.offer_range().0,
        max_offer_count: group.offer_range().1,
        best_isk_per_lp: money(group.best_isk_per_lp()),
        profit_within_10_percent_total: money(group.profit_within_10()),
        budget_isk: money(group.budget.map(|fill| fill.isk)),
        budget_lp_spent: group.budget.map(|fill| fill.lp_spent),
        budget_isk_per_lp: money(group.budget.and_then(|fill| fill.isk_per_lp())),
        min_hidden,
        max_hidden,
        corporations: corporations(
            group
                .corp_ids
                .iter()
                .copied()
                .zip(group.offer_counts.iter().copied()),
            sdata,
        ),
        offers: group
            .results
            .iter()
            .take(depth)
            .enumerate()
            .map(|(index, result)| export_offer(result, index + 1, sdata))
            .collect(),
    }
}

fn export_offer<'a>(
    result: &'a OfferResult,
    rank: usize,
    sdata: &'a StaticData,
) -> ExportOffer<'a> {
    let offer = &result.offer;
    ExportOffer {
        rank,
        item: sdata.type_name(offer.type_id).map(String::as_str),
        type_id: offer.type_id,
        lp_cost: offer.lp_cost,
        units: offer.quantity,
        reward: money(result.reward),
        unit_cost: money(result.unit_cost),
        profit_per_unit: money(result.profit_per_unit()),
        isk_per_lp: money(result.isk_per_lp),
        best_ask: money(result.best_ask),
        spread_percent: money(result.spread),
        bid_depth: result.book_depth.map(|(bid, _)| bid),
        ask_depth: result.book_depth.map(|(_, ask)| ask),
        imbalance_percent: money(result.imbalance),
        volume_within_10_percent: result.within_10.map(|(volume, _)| volume),
        profit_within_10_percent: money(result.within_10.map(|(_, profit)| profit)),
        required_items: result
            .breakdown
            .item_expenses
            .iter()
            .map(|item| ExportItem {
                item: sdata.type_name(item.type_id).map(String::as_str),
                type_id: item.type_id,
                quantity: item.quantity,
                price: round_money(item.best_price),
                cost: round_money(item.total_cost),
            })
            .collect(),
    }
}

/// One CSV row: the group's columns, then the offer's.
fn csv_row(group: &ExportGroup, offer: &ExportOffer) -> Vec<String> {
    let corporations = group
        .corporations
        .iter()
        .map(|corp| match corp.name {
            Some(name) => format!("{} ({})", name, corp.id),
            None => corp.id.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        group.rank.to_string(),
        number(group.average_isk_per_lp),
        group.min_offer_count.to_string(),
        group.max_offer_count.to_string(),
        number(group.best_isk_per_lp),
        number(group.profit_within_10_percent_total),
        number(group.budget_isk),
        group
            .budget_lp_spent
            .map_or_else(String::new, |lp| lp.to_string()),
        number(group.budget_isk_per_lp),
        group.min_hidden.to_string(),
        group.max_hidden.to_string(),
        csv_field(&corporations),
        offer.rank.to_string(),
        csv_text(offer.item),
        offer.type_id.to_string(),
        offer.lp_cost.to_string(),
        offer.units.to_string(),
        number(offer.reward),
        number(offer.unit_cost),
        number(offer.profit_per_unit),
        number(offer.isk_per_lp),
        number(offer.best_ask),
        number(offer.spread_percent),
        whole(offer.bid_depth),
        whole(offer.ask_depth),
        number(offer.imbalance_percent),
        whole(offer.volume_within_10_percent),
        number(offer.profit_within_10_percent),
    ]
}

/// A number as a spreadsheet wants it: no separators, and empty when the run could not work it out.
fn number(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn whole(value: Option<i64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn csv_text(value: Option<&str>) -> String {
    value.map(csv_field).unwrap_or_default()
}

/// One CSV field, quoted as RFC 4180 requires when it holds a comma, a quote, or a line break.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
    use crate::report::Store;
    use crate::{ExpenseBreakdown, ItemExpense, OfferError, PricingError};
    use std::sync::Arc;

    /// A run over two stores. The first, of two corporations, has two priced offers and offer 42,
    /// which failed; the second has one.
    fn report() -> Report {
        let failed = OfferError {
            offer_id: 42,
            error: PricingError::NoSellOrders {
                type_id: 34,
                region_id: 10000002,
            },
        };
        Report::new(
            &[],
            vec![
                Store::new(
                    vec![1000002, 999999],
                    vec![1, 2, 42],
                    vec![
                        Arc::new(offer(1, 30486, 1627.78, true)),
                        Arc::new(offer(2, 1, 900.0, false)),
                    ],
                ),
                Store::new(
                    vec![1000001],
                    vec![3],
                    vec![Arc::new(offer(3, 30486, 500.0, false))],
                ),
            ],
            vec![failed],
            10,
        )
    }

    /// One priced offer. With `full`, every figure has a value; without it, the market side is
    /// missing, the way it is for a reward nobody sells.
    fn offer(offer_id: i32, type_id: i32, isk_per_lp: f64, full: bool) -> OfferResult {
        OfferResult {
            offer: LoyaltyOffer {
                offer_id,
                type_id,
                quantity: 10,
                lp_cost: 1800,
                isk_cost: 1_200_000.0,
                required_items: vec![RequiredItem {
                    type_id: 34,
                    quantity: 5,
                }],
            },
            breakdown: ExpenseBreakdown {
                offer_id,
                lp_cost: 1800,
                isk_cost: 1_200_000.0,
                item_expenses: vec![ItemExpense {
                    type_id: 34,
                    quantity: 5,
                    best_price: 20.5,
                    total_cost: 102.5,
                }],
                total_isk: 1_649_000.0,
            },
            reward: full.then_some(457_900.0),
            unit_cost: Some(164_900.0),
            isk_per_lp: Some(isk_per_lp),
            best_ask: full.then_some(515_000.0),
            spread: full.then_some(12.5),
            book_depth: full.then_some((2_474_600, 800_000)),
            imbalance: full.then_some(51.2),
            within_10: full.then_some((2_474_600, 3_980_000_000.0)),
            buy_orders: Default::default(),
        }
    }

    fn context(depth: usize) -> ExportContext<'static> {
        ExportContext::new("The Forge (10000002)", "none", &Arrangement::depth(depth))
    }

    /// The position of the CSV column `name`.
    fn column(name: &str) -> usize {
        CSV_COLUMNS
            .iter()
            .position(|&column| column == name)
            .unwrap_or_else(|| panic!("no column {name}"))
    }

    /// An empty `StaticData`, so names are unknown and the export has to fall back to ids.
    fn no_names() -> StaticData {
        StaticData::default()
    }

    #[test]
    fn the_format_comes_from_the_extension() {
        assert_eq!(
            ExportRequest::parse("offers.csv"),
            Some(ExportRequest {
                path: "offers.csv".to_string(),
                format: ExportFormat::Csv
            })
        );
        assert_eq!(
            ExportRequest::parse("offers.JSON").unwrap().format,
            ExportFormat::Json
        );
        assert_eq!(
            ExportRequest::parse(" a/b/c.json ").unwrap().path,
            "a/b/c.json"
        );
    }

    #[test]
    fn an_unknown_or_missing_extension_is_no_request() {
        assert!(ExportRequest::parse("offers.txt").is_none());
        assert!(ExportRequest::parse("offers").is_none());
        assert!(ExportRequest::parse("").is_none());
        assert!(ExportRequest::parse("   ").is_none());
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        assert_eq!(csv_field("Probe"), "Probe");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn every_csv_row_has_one_cell_per_column() {
        let report = report();
        let context = context(10);
        let sdata = no_names();
        let file = build(&report, &context, &sdata);
        for group in &file.groups {
            for offer in &group.offers {
                assert_eq!(
                    csv_row(group, offer).len(),
                    CSV_COLUMNS.len(),
                    "group {} offer {}",
                    group.rank,
                    offer.rank
                );
            }
        }
    }

    #[test]
    fn csv_starts_with_the_column_names() {
        let report = report();
        let context = context(10);
        let text = csv(&report, &context, &no_names());
        assert_eq!(text.lines().next().unwrap(), CSV_COLUMNS.join(","));
    }

    #[test]
    fn csv_writes_one_row_per_offer_in_view_order() {
        let report = report();
        let context = context(10);
        let text = csv(&report, &context, &no_names());
        let rows: Vec<Vec<String>> = text.lines().skip(1).map(csv_fields).collect();
        // Group 1 first with its two offers, then group 2. The unknown corporation and item fall
        // back to their ids, and the group lists both of its corps in one cell.
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], "1");
        assert_eq!(rows[0][2..4], ["3", "3"], "fewest and most offers");
        assert_eq!(rows[0][column("corporations")], "1000002, 999999");
        assert_eq!(rows[0][column("item")], "");
        assert_eq!(rows[0][column("type_id")], "30486");
        assert_eq!(rows[1][0], "1");
        assert_eq!(rows[1][column("rank")], "2");
        assert_eq!(rows[2][0], "2");
        assert_eq!(rows[2][column("corporations")], "1000001");
    }

    #[test]
    fn csv_leaves_a_missing_number_empty() {
        let report = report();
        let context = context(10);
        let text = csv(&report, &context, &no_names());
        // The second offer of the first group has no reward, no ask, and no book.
        let cells = csv_fields(text.lines().skip(1).nth(1).unwrap());
        assert_eq!(cells[column("reward")], "", "reward");
        assert_eq!(cells[column("best_ask")], "", "best ask");
        assert_eq!(cells[column("spread_percent")], "", "spread");
        assert_eq!(cells[column("imbalance_percent")], "", "imbalance");
        assert_eq!(
            cells[column("profit_within_10_percent")],
            "",
            "profit within 10%"
        );
        assert_eq!(
            cells[column("isk_per_lp")],
            "900",
            "isk per lp is still there"
        );
    }

    /// Splits a CSV row on commas, keeping a quoted field together however many commas it holds.
    /// The export quotes those fields, so a test that reads them back has to do the same.
    fn csv_fields(row: &str) -> Vec<String> {
        let mut fields = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        for character in row.chars() {
            match character {
                '"' => quoted = !quoted,
                ',' if !quoted => {
                    fields.push(std::mem::take(&mut field));
                }
                other => field.push(other),
            }
        }
        fields.push(field);
        fields
    }

    #[test]
    fn export_lists_only_the_top_depth_offers() {
        let report = report();
        let context = context(1);
        let sdata = no_names();
        let file = build(&report, &context, &sdata);
        assert_eq!(file.groups[0].offers.len(), 1);
        assert_eq!(file.groups[0].corporations.len(), 2);
        // The header plus one row per group.
        assert_eq!(csv(&report, &context, &sdata).lines().count(), 3);
        assert_eq!(offer_count(&report, 1), 2);
        assert_eq!(offer_count(&report, 10), 3);
    }

    #[test]
    fn the_average_is_over_the_same_top_depth_offers_the_view_shows() {
        let report = report();
        let sdata = no_names();
        let narrow_context = context(1);
        let wide_context = context(10);
        let narrow = build(&report, &narrow_context, &sdata);
        let wide = build(&report, &wide_context, &sdata);
        assert_eq!(narrow.groups[0].average_isk_per_lp, Some(1627.78));
        assert_eq!(wide.groups[0].average_isk_per_lp, Some(1263.89));
    }

    #[test]
    fn money_is_rounded_to_the_two_decimals_it_will_be_read_as() {
        // The mean of 1,627.78 and 900 is not exact in binary floating point, and a file full of
        // 1263.8899999999999 is worse than useless in a spreadsheet.
        let report = report();
        let ctx = context(10);
        let sdata = no_names();
        assert_eq!(round_money(1263.8899999999999), 1263.89);
        assert_eq!(round_money(1627.775), 1627.78);
        assert_eq!(round_money(-0.004), 0.0);
        let text = csv(&report, &ctx, &sdata);
        assert!(!text.contains("8899999"), "float noise reached the file");
        assert!(
            text.contains("1263.89"),
            "row: {}",
            text.lines().nth(1).unwrap()
        );
        let parsed: serde_json::Value = serde_json::from_str(&json(&report, &ctx, &sdata)).unwrap();
        assert_eq!(parsed["groups"][0]["average_isk_per_lp"], 1263.89);
    }

    #[test]
    fn json_names_every_field_and_nulls_the_missing_ones() {
        let report = report();
        let context = context(10);
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert_eq!(parsed["region"], "The Forge (10000002)");
        assert_eq!(parsed["depth"], 10);
        let group = &parsed["groups"][0];
        assert_eq!(group["rank"], 1);
        assert_eq!(group["min_offer_count"], 3);
        assert_eq!(group["max_offer_count"], 3);
        assert_eq!(group["corporations"][0]["id"], 1000002);
        assert_eq!(group["corporations"][0]["offer_count"], 3);
        assert_eq!(group["corporations"][0]["name"], serde_json::Value::Null);
        let offer = &group["offers"][0];
        assert_eq!(offer["rank"], 1);
        assert_eq!(offer["type_id"], 30486);
        assert_eq!(offer["spread_percent"], 12.5);
        assert_eq!(offer["bid_depth"], 2_474_600);
        assert_eq!(offer["ask_depth"], 800_000);
        assert_eq!(offer["profit_per_unit"], 293_000.0);
        assert_eq!(offer["required_items"][0]["price"], 20.5);
        assert_eq!(offer["required_items"][0]["quantity"], 5);
        // The second offer has no reward, so the figures built on it are null rather than absent.
        assert!(parsed["groups"][0]["offers"][1]["reward"].is_null());
        assert!(parsed["groups"][0]["offers"][1]["profit_per_unit"].is_null());
    }

    #[test]
    fn json_carries_the_offers_that_failed_to_price() {
        let report = report();
        let context = context(10);
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        // Once for the run, not under each group, with the corporations whose store holds it.
        let errors = parsed["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["offer_id"], 42);
        assert!(
            errors[0]["error"]
                .as_str()
                .unwrap()
                .contains("no sell orders")
        );
        assert_eq!(errors[0]["corporations"][0]["id"], 1000002);
        assert_eq!(errors[0]["corporations"][1]["id"], 999999);
        assert!(parsed["groups"][0].get("errors").is_none());
    }

    #[test]
    fn a_joined_group_writes_its_fewest_and_most_offers() {
        // At depth 1 both stores list only offer 1, so they are one group, though one store holds
        // two offers and the other one.
        let shared = || Arc::new(offer(1, 30486, 1627.78, true));
        let report = Report::new(
            &[1000001, 1000002],
            vec![
                Store::new(
                    vec![1000001],
                    vec![1, 2],
                    vec![shared(), Arc::new(offer(2, 1, 900.0, false))],
                ),
                Store::new(vec![1000002], vec![1], vec![shared()]),
            ],
            Vec::new(),
            1,
        );
        let context = context(1);
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        let group = &parsed["groups"][0];
        assert_eq!(parsed["groups"].as_array().unwrap().len(), 1);
        assert_eq!(group["min_offer_count"], 1);
        assert_eq!(group["max_offer_count"], 2);
        assert_eq!(group["corporations"][0]["offer_count"], 2);
        assert_eq!(group["corporations"][1]["offer_count"], 1);
        let rows: Vec<Vec<String>> = csv(&report, &context, &no_names())
            .lines()
            .skip(1)
            .map(csv_fields)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][2..4], ["1", "2"]);
        assert_eq!(rows[0][column("corporations")], "1000001, 1000002");
    }

    #[test]
    fn an_empty_report_still_writes_a_valid_file() {
        let report = Report::new(&[], Vec::new(), Vec::new(), 10);
        let context = context(10);
        assert_eq!(
            csv(&report, &context, &no_names()),
            format!("{}\n", CSV_COLUMNS.join(","))
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert_eq!(parsed["groups"].as_array().unwrap().len(), 0);
        assert_eq!(offer_count(&report, 10), 0);
    }

    /// One store holding offer 1, priced in full with 25 units bid at 500,000 ISK, and offer 2,
    /// which has no spread, arranged with spreads above 50% hidden, a budget of 4,000 LP, and
    /// the budget sort.
    fn filtered_report(hiding: bool) -> Report {
        let mut sold = offer(1, 30486, 1627.78, true);
        sold.buy_orders = vec![crate::expenses::BookOrder {
            price: 500_000.0,
            volume_remain: 25,
        }]
        .into();
        let mut report = Report::new(
            &[],
            vec![Store::new(
                vec![1000002],
                vec![1, 2],
                vec![Arc::new(sold), Arc::new(offer(2, 1, 900.0, false))],
            )],
            Vec::new(),
            10,
        );
        let mut arrangement = Arrangement::depth(10);
        arrangement.filter.max_spread = Some(50.0);
        arrangement.lp_budget = Some(4000);
        arrangement.sort = GroupSort::BudgetIsk;
        arrangement.hiding = hiding;
        report.arrange(&arrangement);
        report
    }

    #[test]
    fn a_filtered_export_lists_only_the_passing_offers_with_the_group_figures() {
        let report = filtered_report(true);
        let context = ExportContext::new("The Forge (10000002)", "none", report.arrangement());
        let rows: Vec<Vec<String>> = csv(&report, &context, &no_names())
            .lines()
            .skip(1)
            .map(csv_fields)
            .collect();
        // Offer 2 has no spread, so the filter hides it. Offer 1 is redeemed twice, each time
        // selling 10 units at 500,000 for a cost of 1,649,000, and the 5 units left take no
        // further redemption.
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        let cell = |name: &str| row[column(name)].as_str();
        assert_eq!(cell("type_id"), "30486");
        assert_eq!(cell("best_isk_per_lp"), "1627.78");
        assert_eq!(cell("profit_within_10_percent_total"), "3980000000");
        assert_eq!(cell("budget_isk"), "6702000");
        assert_eq!(cell("budget_lp_spent"), "3600");
        assert_eq!(cell("budget_isk_per_lp"), "1861.67");
        assert_eq!((cell("min_hidden"), cell("max_hidden")), ("1", "1"));
        assert_eq!(offer_count(&report, context.depth), 1);

        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert_eq!(parsed["sort"], "budget");
        assert_eq!(parsed["lp_budget"], 4000);
        assert_eq!(parsed["hiding"], true);
        let group = &parsed["groups"][0];
        assert_eq!(group["offers"].as_array().unwrap().len(), 1);
        assert_eq!(group["budget_isk"], 6_702_000.0);
        assert_eq!(group["budget_lp_spent"], 3600);
        assert_eq!(group["min_hidden"], 1);
    }

    #[test]
    fn with_hiding_off_the_export_lists_every_offer_in_ranking_order() {
        let report = filtered_report(false);
        let context = ExportContext::new("The Forge (10000002)", "none", report.arrangement());
        let rows: Vec<Vec<String>> = csv(&report, &context, &no_names())
            .lines()
            .skip(1)
            .map(csv_fields)
            .collect();
        let offers: Vec<(&str, &str)> = rows
            .iter()
            .map(|row| {
                (
                    row[column("rank")].as_str(),
                    row[column("type_id")].as_str(),
                )
            })
            .collect();
        assert_eq!(offers, [("1", "30486"), ("2", "1")]);
        assert_eq!(rows[0][column("max_hidden")], "0");
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert_eq!(parsed["hiding"], false);
    }

    #[test]
    fn without_a_budget_or_buy_orders_the_budget_figures_are_empty() {
        let report = report();
        let context = context(10);
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert_eq!(parsed["sort"], "avg");
        assert_eq!(parsed["lp_budget"], 1000);
        // The offers carry no buy orders, so nothing can be redeemed.
        assert!(parsed["groups"][0]["budget_isk"].is_null());
        assert!(parsed["groups"][0]["budget_isk_per_lp"].is_null());
        let row = csv_fields(csv(&report, &context, &no_names()).lines().nth(1).unwrap());
        assert_eq!(row[column("budget_lp_spent")], "");

        let mut report = report;
        let mut arrangement = Arrangement::depth(10);
        arrangement.lp_budget = None;
        report.arrange(&arrangement);
        let context = ExportContext::new("The Forge (10000002)", "none", report.arrangement());
        let parsed: serde_json::Value =
            serde_json::from_str(&json(&report, &context, &no_names())).unwrap();
        assert!(parsed["lp_budget"].is_null());
    }

    #[test]
    fn the_export_is_the_same_bytes_twice() {
        let report = report();
        let context = context(10);
        assert_eq!(
            csv(&report, &context, &no_names()),
            csv(&report, &context, &no_names())
        );
        assert_eq!(
            json(&report, &context, &no_names()),
            json(&report, &context, &no_names())
        );
    }
}

//! Values the tests of the view's modules build on, kept in one place. The library's fixtures are
//! not reachable from here, because the view is part of the binary and the library is compiled
//! without `cfg(test)` for it.

use super::app::{App, ExportPrompt, Overlay, RunInfo, StaticDataDialog};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use eve_lpstore::OfferResult;
use eve_lpstore::esi::loyalty::LoyaltyOffer;
use eve_lpstore::expenses::ExpenseBreakdown;
use eve_lpstore::report::{LpStoreOptions, Report, Store};
use eve_lpstore::static_data::AgentFilter;
use std::sync::Arc;

/// A key press with no modifiers.
pub fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A run in The Forge over `corporations` corporations, with no agent filter.
pub fn forge_run(corporations: usize) -> RunInfo {
    RunInfo {
        region: "The Forge".to_string(),
        region_id: 10000002,
        agent_region_id: None,
        corporations,
        agent_filter: AgentFilter::default(),
    }
}

/// The Sisters Combat Scanner Probe offer the worked example uses, priced: 10 units for 1,800 LP
/// and 1,200,000 ISK, a best bid of 457,900, and 1,627.78 ISK per LP. The market side is given by
/// the caller, and the ask and the book are derived from it so the fixture stays consistent.
pub fn priced_offer(
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
        best_ask: spread.map(|s| 457_900.0 * (1.0 + s / 100.0)),
        spread,
        // A book of 100 units split to match the imbalance.
        book_depth: imbalance.map(|percent| {
            let bid = (100.0 + percent) / 2.0;
            (bid as i64, 100 - bid as i64)
        }),
        imbalance,
        within_10,
    }
}

/// A report whose only store is corporation 1000002's, holding `offers`.
pub fn report_of(offers: Vec<OfferResult>) -> Report {
    let offer_ids = offers.iter().map(|result| result.offer.offer_id).collect();
    let results = offers.into_iter().map(Arc::new).collect();
    Report::new(
        &[1000002],
        vec![Store::new(vec![1000002], offer_ids, results)],
        Vec::new(),
        10,
    )
}

/// An app on the results view of one finished run, with `offers` as the only group, which is
/// corporation 1000002.
pub fn results_app(offers: Vec<OfferResult>, depth: usize) -> App {
    let mut app = App::new(LpStoreOptions::default());
    app.begin_run(forge_run(1), "none".to_string(), depth);
    app.finish(report_of(offers));
    app
}

/// The open export prompt, to type into or read back.
pub fn open_prompt_mut(app: &mut App) -> &mut ExportPrompt {
    match &mut app.overlay {
        Overlay::Export(prompt) => prompt,
        other => panic!("the export prompt is not open: {other:?}"),
    }
}

/// The open export prompt.
pub fn open_prompt(app: &App) -> &ExportPrompt {
    app.export_prompt().expect("the export prompt is open")
}

/// The open static data dialog.
pub fn dialog(app: &App) -> &StaticDataDialog {
    match &app.overlay {
        Overlay::StaticData(dialog) => dialog,
        other => panic!("the static data dialog is not open: {other:?}"),
    }
}

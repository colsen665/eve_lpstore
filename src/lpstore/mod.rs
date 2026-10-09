use crate::esi::error::EsiError;
use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
use crate::esi::EsiClient;
use crate::expenses::{breakdown_from, ExpenseBreakdown, ItemExpense, MarketQuote};
use crate::tasks::run_children;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// The calculated expenses, reward, and profit for a loyalty offer.
#[derive(Debug, Clone)]
pub struct OfferResult {
    pub offer: LoyaltyOffer,
    pub breakdown: ExpenseBreakdown,
    pub reward: Option<f64>,
    pub unit_cost: Option<f64>,
    pub isk_per_lp: Option<f64>,
    /// The lowest sell price for the reward, or `None` when nothing sells it.
    pub best_ask: Option<f64>,
    /// The bid/ask spread on the reward as a percentage of the best bid, or `None` when one side
    /// of the market is missing.
    pub spread: Option<f64>,
    /// Volume within 10% of the touch on each side, as (bid, ask), or `None` when one side of the
    /// market is missing.
    pub book_depth: Option<(i64, i64)>,
    /// How lopsided that volume is, from -100 to +100, or `None` when either side is empty.
    pub imbalance: Option<f64>,
    pub within_10: Option<(i64, f64)>,
}

/// An offer that could not be priced.
#[derive(Debug)]
pub struct OfferError {
    pub offer_id: i32,
    pub error: EsiError,
}

/// Endpoint for evaluating loyalty store offers.
pub struct LpStore {
    client: Arc<EsiClient>,
    pool: Arc<Semaphore>,
}

impl LpStore {
    pub fn new(client: Arc<EsiClient>, pool: Arc<Semaphore>) -> Self {
        Self { client, pool }
    }

    /// Price each offer in the given region.
    ///
    /// Returns the successful results sorted by ISK per LP descending, with offers
    /// lacking a value last, and the offers that failed to price.
    pub async fn offer_results(
        &self,
        offers: Vec<LoyaltyOffer>,
        region_id: i32,
        locations: Option<Arc<HashSet<i64>>>,
    ) -> (Vec<OfferResult>, Vec<OfferError>) {
        let client = self.client.clone();
        let pool = self.pool.clone();
        let outcomes = run_children(&self.pool, offers, move |offer| {
            price_offer(client.clone(), pool.clone(), offer, region_id, locations.clone())
        })
        .await;

        let mut results = Vec::new();
        let mut errors = Vec::new();
        for outcome in outcomes {
            match outcome {
                Ok(result) => results.push(result),
                Err(error) => errors.push(error),
            }
        }

        results.sort_by(|a, b| {
            b.isk_per_lp
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&a.isk_per_lp.unwrap_or(f64::NEG_INFINITY))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        (results, errors)
    }
}

/// A subtask of pricing one offer.
enum Child {
    Item(RequiredItem),
    Reward,
}

/// The output of a subtask of pricing one offer.
enum Priced {
    Item(Result<ItemExpense, EsiError>),
    Reward(Result<MarketQuote, EsiError>),
}

async fn price_offer(
    client: Arc<EsiClient>,
    pool: Arc<Semaphore>,
    offer: LoyaltyOffer,
    region_id: i32,
    locations: Option<Arc<HashSet<i64>>>,
) -> Result<OfferResult, OfferError> {
    let offer_id = offer.offer_id;
    let type_id = offer.type_id;

    let mut children = vec![Child::Reward];
    children.extend(offer.required_items.iter().cloned().map(Child::Item));

    let run_client = client.clone();
    let priced = run_children(&pool, children, move |child| {
        let client = run_client.clone();
        let locations = locations.clone();
        async move {
            match child {
                Child::Item(item) => Priced::Item(
                    client
                        .expenses()
                        .item_expense(&item, region_id, locations.as_deref())
                        .await,
                ),
                Child::Reward => Priced::Reward(
                    client
                        .expenses()
                        .reward_quote(type_id, region_id, locations.as_deref())
                        .await,
                ),
            }
        }
    })
    .await;

    let mut item_expenses = Vec::with_capacity(offer.required_items.len());
    let mut quote = None;
    let mut failure = None;
    for outcome in priced {
        match outcome {
            Priced::Item(Ok(item)) => item_expenses.push(item),
            Priced::Reward(Ok(market)) => quote = Some(market),
            Priced::Item(Err(error)) | Priced::Reward(Err(error)) => {
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(error) = failure {
        return Err(OfferError { offer_id, error });
    }
    let quote = quote.expect("reward lookup always produces an output");

    let breakdown = breakdown_from(&offer, item_expenses);
    let reward = quote.best_bid();
    let best_ask = quote.best_ask();
    let unit_cost = (offer.quantity > 0).then(|| breakdown.total_isk / offer.quantity as f64);
    let isk_per_lp = redemption_isk_per_lp(reward, &offer, breakdown.total_isk);
    let spread = spread_percent(reward, best_ask);
    let within_10 = match (reward, unit_cost) {
        (Some(best), Some(unit_cost)) => {
            let threshold = best * 0.9;
            Some(
                quote
                    .buy_orders
                    .iter()
                    .filter(|o| o.price >= threshold)
                    .fold((0i64, 0f64), |(volume, profit), o| {
                        (
                            volume + o.volume_remain as i64,
                            profit + (o.price - unit_cost) * o.volume_remain as f64,
                        )
                    }),
            )
        }
        _ => None,
    };
    // The bid side is the volume `within_10` already counted, so the two cannot disagree. The ask
    // side is the sellers within the same band, mirrored above the best ask. Both sides have to be
    // present, so a reward nobody sells leaves the book unknown rather than looking all-buyers.
    let book_depth = match (within_10, best_ask) {
        (Some((bid_depth, _)), Some(best_ask)) => {
            let ceiling = best_ask * 1.1;
            let ask_depth: i64 = quote
                .ask_orders
                .iter()
                .filter(|o| o.price <= ceiling)
                .map(|o| o.volume_remain as i64)
                .sum();
            Some((bid_depth, ask_depth))
        }
        _ => None,
    };
    let imbalance = book_depth.and_then(|(bid, ask)| imbalance_percent(bid, ask));

    Ok(OfferResult {
        offer,
        breakdown,
        reward,
        unit_cost,
        isk_per_lp,
        best_ask,
        spread,
        book_depth,
        imbalance,
        within_10,
    })
}

/// ISK per LP for one redemption. The reward is per unit, so it is multiplied by the units the
/// redemption gives. `total_isk` already covers the whole redemption.
fn redemption_isk_per_lp(reward: Option<f64>, offer: &LoyaltyOffer, total_isk: f64) -> Option<f64> {
    let reward = reward?;
    (offer.quantity > 0 && offer.lp_cost > 0)
        .then(|| (reward * offer.quantity as f64 - total_isk) / offer.lp_cost as f64)
}

/// The bid/ask spread on the reward as a percentage of the best bid. The best bid is the reward,
/// the price the offer can actually sell at, so the spread says how much more the item costs to
/// buy back: a wide one means the best bid may be a single stale order, and the ISK per LP built
/// on it is optimistic.
///
/// `None` when either side of the market is missing, which includes the common case of a reward
/// nobody sells. A negative result means the market is inverted, with the bid above the ask.
fn spread_percent(best_bid: Option<f64>, best_ask: Option<f64>) -> Option<f64> {
    let bid = best_bid.filter(|bid| *bid > 0.0)?;
    Some((best_ask? - bid) / bid * 100.0)
}

/// How lopsided the book is near the touch, from -100 when only sellers remain to +100 when only
/// buyers do. Positive favours an LP holder, who is always selling: the buyers behind the best bid
/// outweigh the sellers competing to undercut it. Negative means thin bids under a wall of
/// sellers, so the reward driving the ISK per LP is hard to actually realize.
///
/// `None` unless both sides have volume, so a reward nobody sells reads as unknown rather than as
/// an overwhelming bid.
fn imbalance_percent(bid_depth: i64, ask_depth: i64) -> Option<f64> {
    (bid_depth > 0 && ask_depth > 0)
        .then(|| (bid_depth - ask_depth) as f64 / (bid_depth + ask_depth) as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(quantity: i32, lp_cost: i32) -> LoyaltyOffer {
        LoyaltyOffer {
            offer_id: 1,
            type_id: 1,
            quantity,
            lp_cost,
            isk_cost: 0.0,
            required_items: Vec::new(),
        }
    }

    #[test]
    fn isk_per_lp_is_for_the_whole_redemption() {
        let ten_units = offer(10, 100);
        assert_eq!(
            redemption_isk_per_lp(Some(1000.0), &ten_units, 5000.0),
            Some(50.0)
        );
        let one_unit = offer(1, 100);
        assert_eq!(
            redemption_isk_per_lp(Some(1000.0), &one_unit, 500.0),
            Some(5.0)
        );
    }

    #[test]
    fn isk_per_lp_matches_sisters_combat_scanner_probe() {
        let probe = offer(10, 1800);
        let value = redemption_isk_per_lp(Some(457_900.0), &probe, 1_649_000.0).unwrap();
        assert!((value - 1627.78).abs() < 0.01, "got {}", value);
    }

    #[test]
    fn isk_per_lp_needs_a_reward_and_lp_cost() {
        assert_eq!(redemption_isk_per_lp(None, &offer(10, 100), 0.0), None);
        assert_eq!(redemption_isk_per_lp(Some(1.0), &offer(10, 0), 0.0), None);
    }

    #[test]
    fn spread_is_the_ask_as_a_percentage_over_the_bid() {
        assert_eq!(spread_percent(Some(200.0), Some(250.0)), Some(25.0));
        assert_eq!(spread_percent(Some(200.0), Some(200.0)), Some(0.0));
    }

    #[test]
    fn spread_is_negative_when_the_market_is_inverted() {
        assert_eq!(spread_percent(Some(200.0), Some(150.0)), Some(-25.0));
    }

    #[test]
    fn spread_needs_both_sides_of_the_market() {
        assert_eq!(spread_percent(Some(200.0), None), None);
        assert_eq!(spread_percent(None, Some(250.0)), None);
        assert_eq!(spread_percent(None, None), None);
    }

    #[test]
    fn spread_needs_a_positive_bid() {
        assert_eq!(spread_percent(Some(0.0), Some(250.0)), None);
    }

    #[test]
    fn imbalance_is_positive_when_buyers_outweigh_sellers() {
        assert_eq!(imbalance_percent(75, 25), Some(50.0));
        assert_eq!(imbalance_percent(25, 75), Some(-50.0));
    }

    #[test]
    fn a_balanced_book_is_zero() {
        assert_eq!(imbalance_percent(50, 50), Some(0.0));
    }

    #[test]
    fn imbalance_needs_volume_on_both_sides() {
        // A reward nobody sells is unknown, not an overwhelming bid.
        assert_eq!(imbalance_percent(100, 0), None);
        assert_eq!(imbalance_percent(0, 100), None);
        assert_eq!(imbalance_percent(0, 0), None);
    }
}

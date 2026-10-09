use crate::esi::error::EsiError;
use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
use crate::esi::EsiClient;
use crate::expenses::{breakdown_from, BuyOrder, ExpenseBreakdown, ItemExpense};
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
    Reward(Result<Vec<BuyOrder>, EsiError>),
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
                        .buy_orders(type_id, region_id, locations.as_deref())
                        .await,
                ),
            }
        }
    })
    .await;

    let mut item_expenses = Vec::with_capacity(offer.required_items.len());
    let mut buy_orders = None;
    let mut failure = None;
    for outcome in priced {
        match outcome {
            Priced::Item(Ok(item)) => item_expenses.push(item),
            Priced::Reward(Ok(orders)) => buy_orders = Some(orders),
            Priced::Item(Err(error)) | Priced::Reward(Err(error)) => {
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(error) = failure {
        return Err(OfferError { offer_id, error });
    }
    let buy_orders = buy_orders.expect("reward lookup always produces an output");

    let breakdown = breakdown_from(&offer, item_expenses);
    let reward = buy_orders
        .iter()
        .map(|o| o.price)
        .fold(None, |best: Option<f64>, price| {
            Some(best.map_or(price, |b| b.max(price)))
        });
    let unit_cost = (offer.quantity > 0).then(|| breakdown.total_isk / offer.quantity as f64);
    let isk_per_lp = redemption_isk_per_lp(reward, &offer, breakdown.total_isk);
    let within_10 = match (reward, unit_cost) {
        (Some(best), Some(unit_cost)) => {
            let threshold = best * 0.9;
            Some(
                buy_orders
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

    Ok(OfferResult {
        offer,
        breakdown,
        reward,
        unit_cost,
        isk_per_lp,
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
}

use crate::esi::error::EsiError;
use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
use crate::esi::EsiClient;
use std::collections::HashSet;

/// Whether an order's location is allowed. With no location set, every order is allowed.
fn in_locations(locations: Option<&HashSet<i64>>, location_id: i64) -> bool {
    locations.map_or(true, |set| set.contains(&location_id))
}

/// A buy order for an item in a region.
#[derive(Debug, Clone)]
pub struct BuyOrder {
    pub price: f64,
    pub volume_remain: i32,
}

/// Both sides of the market for a reward item in a region.
#[derive(Debug, Clone)]
pub struct MarketQuote {
    /// The buy orders the reward can sell into.
    pub buy_orders: Vec<BuyOrder>,
    /// The lowest sell price, or `None` when nothing sells it.
    pub best_ask: Option<f64>,
}

/// The calculated market cost of a single required item.
#[derive(Debug, Clone)]
pub struct ItemExpense {
    pub type_id: i32,
    pub quantity: i32,
    pub best_price: f64,
    pub total_cost: f64,
}

/// Full expense breakdown for a loyalty offer.
#[derive(Debug, Clone)]
pub struct ExpenseBreakdown {
    pub offer_id: i32,
    pub lp_cost: i32,
    pub isk_cost: f64,
    pub item_expenses: Vec<ItemExpense>,
    pub total_isk: f64,
}

/// Build the breakdown for an offer from the priced required items.
pub fn breakdown_from(offer: &LoyaltyOffer, item_expenses: Vec<ItemExpense>) -> ExpenseBreakdown {
    let items_total: f64 = item_expenses.iter().map(|e| e.total_cost).sum();
    let total_isk = offer.isk_cost + items_total;

    ExpenseBreakdown {
        offer_id: offer.offer_id,
        lp_cost: offer.lp_cost,
        isk_cost: offer.isk_cost,
        item_expenses,
        total_isk,
    }
}

/// Endpoint for calculating ISK expenses.
pub struct ExpensesEndpoint<'a> {
    pub(crate) client: &'a EsiClient,
}

impl<'a> ExpensesEndpoint<'a> {
    /// Both sides of the market for a reward type in the given region.
    ///
    /// The buy orders are what the reward can sell into. The best ask is what buying one would
    /// cost, and is `None` when nothing sells it. That is not an error: the offer still prices,
    /// and only the spread is unknown.
    pub async fn reward_quote(
        &self,
        type_id: i32,
        region_id: i32,
        locations: Option<&HashSet<i64>>,
    ) -> Result<MarketQuote, EsiError> {
        let orders = self
            .client
            .markets()
            .orders(region_id, Some(type_id))
            .await?;

        let buy_orders = orders
            .iter()
            .filter(|o| o.is_buy_order && in_locations(locations, o.location_id))
            .map(|o| BuyOrder {
                price: o.price,
                volume_remain: o.volume_remain,
            })
            .collect();
        let best_ask = orders
            .iter()
            .filter(|o| !o.is_buy_order && in_locations(locations, o.location_id))
            .map(|o| o.price)
            .fold(None, |best: Option<f64>, price| {
                Some(best.map_or(price, |b| b.min(price)))
            });

        Ok(MarketQuote {
            buy_orders,
            best_ask,
        })
    }

    /// The cost of one required item at the best (lowest) sell price in the region.
    pub async fn item_expense(
        &self,
        item: &RequiredItem,
        region_id: i32,
        locations: Option<&HashSet<i64>>,
    ) -> Result<ItemExpense, EsiError> {
        let orders = self
            .client
            .markets()
            .orders(region_id, Some(item.type_id))
            .await?;

        let best_price = orders
            .iter()
            .filter(|o| !o.is_buy_order && in_locations(locations, o.location_id))
            .map(|o| o.price)
            .fold(f64::INFINITY, f64::min);

        if best_price.is_infinite() {
            return Err(EsiError::Esi {
                status: 404,
                message: format!(
                    "no sell orders found for type_id {} in region {}",
                    item.type_id, region_id
                ),
            });
        }

        Ok(ItemExpense {
            type_id: item.type_id,
            quantity: item.quantity,
            best_price,
            total_cost: best_price * item.quantity as f64,
        })
    }

    /// Calculate the total ISK expenses for a loyalty offer.
    ///
    /// Uses the best (lowest) sell price in the given region for each required
    /// item, regardless of whether any single order has sufficient volume.
    pub async fn offer_expenses(
        &self,
        offer: &LoyaltyOffer,
        region_id: i32,
    ) -> Result<ExpenseBreakdown, EsiError> {
        let mut item_expenses = Vec::with_capacity(offer.required_items.len());
        for item in &offer.required_items {
            item_expenses.push(self.item_expense(item, region_id, None).await?);
        }
        Ok(breakdown_from(offer, item_expenses))
    }
}

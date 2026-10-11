use crate::esi::error::EsiError;
use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
use crate::esi::EsiClient;
use std::collections::HashSet;

/// Whether an order's location is allowed. With no location set, every order is allowed.
fn in_locations(locations: Option<&HashSet<i64>>, location_id: i64) -> bool {
    locations.is_none_or(|set| set.contains(&location_id))
}

/// One order on either side of the market for an item in a region.
#[derive(Debug, Clone)]
pub struct BookOrder {
    pub price: f64,
    pub volume_remain: i32,
}

/// Both sides of the market for a reward item in a region.
#[derive(Debug, Clone)]
pub struct MarketQuote {
    /// The buy orders the reward can sell into.
    pub buy_orders: Vec<BookOrder>,
    /// The sell orders competing to sell it. Empty when nothing sells it, which is not an error:
    /// the offer still prices, and only the ask side of the comparison is unknown.
    pub ask_orders: Vec<BookOrder>,
}

impl MarketQuote {
    /// The highest buy price, which is what the reward actually sells for. `None` when nobody is
    /// buying it.
    pub fn best_bid(&self) -> Option<f64> {
        self.buy_orders
            .iter()
            .map(|o| o.price)
            .fold(None, |best: Option<f64>, price| {
                Some(best.map_or(price, |b| b.max(price)))
            })
    }

    /// The lowest sell price, or `None` when nothing sells the reward.
    pub fn best_ask(&self) -> Option<f64> {
        self.ask_orders
            .iter()
            .map(|o| o.price)
            .fold(None, |best: Option<f64>, price| {
                Some(best.map_or(price, |b| b.min(price)))
            })
    }
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
#[derive(Debug, Clone, Default)]
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
    /// The buy orders are what the reward can sell into, and the sell orders are the competition
    /// to sell it. An empty ask side is not an error: the offer still prices, and only the
    /// comparisons that need a sell price come back unknown.
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

        let side = |is_buy: bool| {
            orders
                .iter()
                .filter(move |o| o.is_buy_order == is_buy && in_locations(locations, o.location_id))
                .map(|o| BookOrder {
                    price: o.price,
                    volume_remain: o.volume_remain,
                })
                .collect()
        };

        Ok(MarketQuote {
            buy_orders: side(true),
            ask_orders: side(false),
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
}

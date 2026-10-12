use crate::esi::error::EsiError;
use crate::esi::loyalty::{LoyaltyOffer, RequiredItem};
use crate::esi::EsiClient;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

/// Whether an order's location is allowed. With no location set, every order is allowed.
fn in_locations(locations: Option<&HashSet<i64>>, location_id: i64) -> bool {
    locations.is_none_or(|set| set.contains(&location_id))
}

/// Why an offer could not be priced: an ESI request failed, or a required item has nothing for sale
/// where the run prices.
#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error(transparent)]
    Esi(#[from] EsiError),

    /// Every required item is bought at the lowest sell price, so an item nobody sells leaves the
    /// offer without a cost. This is an answer from the market, not a failed request.
    #[error("no sell orders found for type_id {type_id} in region {region_id}")]
    NoSellOrders { type_id: i32, region_id: i32 },

    /// The task pricing the offer ended without a result, such as by a panic.
    #[error("pricing task failed: {0}")]
    TaskFailed(String),
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

/// The best sell price of each required item type one run has looked up, in the run's region and
/// stations, so a type many offers require is looked up once rather than once per offer. An
/// answer is kept, including that nothing sells the type; a failed request is not, so the next
/// offer that needs the type asks again.
#[derive(Debug)]
pub struct ItemPrices {
    region_id: i32,
    locations: Option<Arc<HashSet<i64>>>,
    prices: Mutex<HashMap<i32, Arc<OnceCell<Option<f64>>>>>,
}

impl ItemPrices {
    pub fn new(region_id: i32, locations: Option<Arc<HashSet<i64>>>) -> Self {
        Self {
            region_id,
            locations,
            prices: Mutex::new(HashMap::new()),
        }
    }

    /// The best sell price of `type_id`, or `None` when nothing sells it, from `look_up` the first
    /// time and from memory after that. Offers asking at the same moment wait for one lookup.
    async fn best_price<F, Fut>(&self, type_id: i32, look_up: F) -> Result<Option<f64>, EsiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Option<f64>, EsiError>>,
    {
        let cell = self
            .prices
            .lock()
            .unwrap()
            .entry(type_id)
            .or_default()
            .clone();
        cell.get_or_try_init(look_up).await.copied()
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

    /// The cost of one required item at the best (lowest) sell price in the region and stations
    /// of `prices`, which keeps each type's price for the rest of the run.
    pub async fn item_expense(
        &self,
        item: &RequiredItem,
        prices: &ItemPrices,
    ) -> Result<ItemExpense, PricingError> {
        let region_id = prices.region_id;
        let locations = prices.locations.as_deref();
        let best_price = prices
            .best_price(item.type_id, || async {
                let orders = self
                    .client
                    .markets()
                    .orders(region_id, Some(item.type_id))
                    .await?;
                let best = orders
                    .iter()
                    .filter(|o| !o.is_buy_order && in_locations(locations, o.location_id))
                    .map(|o| o.price)
                    .fold(f64::INFINITY, f64::min);
                Ok(best.is_finite().then_some(best))
            })
            .await?;

        let Some(best_price) = best_price else {
            return Err(PricingError::NoSellOrders {
                type_id: item.type_id,
                region_id,
            });
        };

        Ok(ItemExpense {
            type_id: item.type_id,
            quantity: item.quantity,
            best_price,
            total_cost: best_price * item.quantity as f64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A lookup that counts its calls and answers `answer` after a short pause.
    fn counted(
        calls: &AtomicUsize,
        answer: Result<Option<f64>, EsiError>,
    ) -> impl Future<Output = Result<Option<f64>, EsiError>> + '_ {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            answer
        }
    }

    fn failed() -> EsiError {
        EsiError::Esi {
            status: 503,
            message: String::new(),
        }
    }

    #[tokio::test]
    async fn a_type_is_looked_up_once_however_many_offers_need_it() {
        let prices = ItemPrices::new(10000002, None);
        let calls = AtomicUsize::new(0);
        for _ in 0..3 {
            let price = prices
                .best_price(34, || counted(&calls, Ok(Some(4.5))))
                .await
                .unwrap();
            assert_eq!(price, Some(4.5));
        }
        let other = prices.best_price(35, || counted(&calls, Ok(None))).await;
        assert_eq!(other.unwrap(), None, "nothing sells it, which is an answer");
        let again = prices
            .best_price(35, || counted(&calls, Ok(Some(1.0))))
            .await;
        assert_eq!(again.unwrap(), None, "and is kept like one");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "one lookup per type");
    }

    #[tokio::test]
    async fn offers_asking_at_once_share_one_lookup() {
        let prices = ItemPrices::new(10000002, None);
        let calls = AtomicUsize::new(0);
        let (a, b) = tokio::join!(
            prices.best_price(34, || counted(&calls, Ok(Some(4.5)))),
            prices.best_price(34, || counted(&calls, Ok(Some(9.9)))),
        );
        assert_eq!((a.unwrap(), b.unwrap()), (Some(4.5), Some(4.5)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_lookup_is_not_kept() {
        let prices = ItemPrices::new(10000002, None);
        let calls = AtomicUsize::new(0);
        let first = prices
            .best_price(34, || counted(&calls, Err(failed())))
            .await;
        assert!(first.is_err());
        let second = prices
            .best_price(34, || counted(&calls, Ok(Some(4.5))))
            .await;
        assert_eq!(second.unwrap(), Some(4.5), "the next offer asked again");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

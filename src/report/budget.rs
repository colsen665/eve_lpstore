//! The LP-budget ladder: the ISK a group's offers realize when a budget of LP is spent on them,
//! with every reward sold into the buy orders that are on the book.
//!
//! [`fill`] spends the budget one redemption at a time, always on the offer whose next redemption
//! earns the most ISK per LP. Selling a redemption uses up buy orders, best price first, so the
//! next redemption of the same reward sells lower, and the ladder moves on to another offer once
//! that one pays better. Consecutive redemptions of one offer that sell at a single price are taken
//! as one batch, which gives the same result as taking them one by one.
//!
//! Limitations:
//! - The choice is greedy. Each step takes the best ISK per LP at that moment, which is not
//!   guaranteed to give the most ISK for the whole budget: a costly offer taken first can leave
//!   LP that no other offer fits.
//! - The ISK cost and required items of every redemption are what the offer was priced at, with
//!   each item bought at the best sell price however many redemptions need it.
//! - Each call evaluates one group's offers on their own. Two groups whose offers sell the same
//!   reward each see the whole book.
//! - Only whole redemptions count. A redemption that spans several buy orders is priced across
//!   them exactly.
//! - A buy order's range and minimum volume are not checked, since the reward quote does not keep
//!   them.

use crate::expenses::BookOrder;
use crate::lpstore::OfferResult;
use std::collections::HashMap;
use std::sync::Arc;

/// What spending an LP budget on a group's offers realizes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BudgetFill {
    /// The profit of the redemptions taken: what their rewards sell for on the buy orders, less
    /// each redemption's ISK cost and required items.
    pub isk: f64,
    /// The LP the redemptions took, at most the budget.
    pub lp_spent: u64,
    /// The number of whole redemptions taken.
    pub redemptions: u64,
    /// Why the ladder stopped.
    pub stopped: Stop,
}

impl BudgetFill {
    /// The realized ISK per LP, or `None` when no LP was spent.
    pub fn isk_per_lp(&self) -> Option<f64> {
        (self.lp_spent > 0).then(|| self.isk / self.lp_spent as f64)
    }
}

/// Why the ladder stopped. Every check is on the next whole redemption of each offer, and an offer
/// with no positive quantity or LP cost, or no finite ISK cost, never counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stop {
    /// At least one offer's book still holds its next redemption, but the LP left pays for none of
    /// those offers. A spent budget stops here unless the books ran out at the same time.
    Budget,
    /// No offer's book holds its next redemption.
    Book,
    /// The best next redemption the LP left pays for earns zero ISK per LP or less.
    Unprofitable,
}

/// Spends `budget` LP on `offers`, a group's offers, and returns what it realizes.
///
/// Offers with the same reward type share one book, the `buy_orders` of the first of them listed,
/// so each buy order is sold into once. Ties in ISK per LP go to the lower offer ID.
///
/// `None` when no offer can be redeemed at all: no offer's book holds even one whole redemption,
/// whatever the budget, which includes a group whose rewards nobody buys. Otherwise the fill is
/// returned even when it takes nothing, such as with a budget of 0, which stops at
/// [`Stop::Budget`] with no ISK and no LP spent, or when the first redemption already loses ISK,
/// which stops at [`Stop::Unprofitable`].
pub fn fill(offers: &[Arc<OfferResult>], budget: u64) -> Option<BudgetFill> {
    let mut books: Vec<Book> = Vec::new();
    let mut book_of_type: HashMap<i32, usize> = HashMap::new();
    let mut candidates = Vec::new();
    for result in offers {
        let offer = &result.offer;
        let cost = result.breakdown.total_isk;
        if offer.quantity <= 0 || offer.lp_cost <= 0 || !cost.is_finite() {
            continue;
        }
        let book = *book_of_type.entry(offer.type_id).or_insert_with(|| {
            books.push(Book::new(&result.buy_orders));
            books.len() - 1
        });
        candidates.push(Candidate {
            offer_id: offer.offer_id,
            book,
            quantity: offer.quantity as u64,
            lp_cost: offer.lp_cost as u64,
            cost,
        });
    }
    if !candidates
        .iter()
        .any(|c| c.quantity <= books[c.book].total())
    {
        return None;
    }

    let mut fill = BudgetFill {
        isk: 0.0,
        lp_spent: 0,
        redemptions: 0,
        stopped: Stop::Book,
    };
    let mut left = budget;
    loop {
        // The best next redemption, as (candidate, its revenue, its ISK per LP).
        let mut best: Option<(usize, f64, f64)> = None;
        let mut any_in_book = false;
        for (index, candidate) in candidates.iter().enumerate() {
            let book = &books[candidate.book];
            if book.sold + candidate.quantity > book.total() {
                continue;
            }
            any_in_book = true;
            if candidate.lp_cost > left {
                continue;
            }
            let revenue = book.revenue(book.sold, candidate.quantity);
            let ratio = (revenue - candidate.cost) / candidate.lp_cost as f64;
            let better = match best {
                None => !ratio.is_nan(),
                Some((other, _, best_ratio)) => {
                    ratio > best_ratio
                        || (ratio == best_ratio && candidate.offer_id < candidates[other].offer_id)
                }
            };
            if better {
                best = Some((index, revenue, ratio));
            }
        }

        let Some((index, revenue, ratio)) = best else {
            fill.stopped = if any_in_book {
                Stop::Budget
            } else {
                Stop::Book
            };
            return Some(fill);
        };
        if ratio <= 0.0 {
            fill.stopped = Stop::Unprofitable;
            return Some(fill);
        }

        let candidate = &candidates[index];
        let book = &mut books[candidate.book];
        let level = book.level(book.sold);
        let level_end = book.units[level + 1];
        // Within one price level, every further redemption of this offer earns the same, and
        // every other offer's next redemption earns the same or less, so this offer stays the
        // best until the level, or the LP for it, runs out.
        let (count, isk) = if book.sold + candidate.quantity <= level_end {
            let count =
                ((level_end - book.sold) / candidate.quantity).min(left / candidate.lp_cost);
            (count, count as f64 * (revenue - candidate.cost))
        } else {
            (1, revenue - candidate.cost)
        };
        book.sold += count * candidate.quantity;
        left -= count * candidate.lp_cost;
        fill.lp_spent += count * candidate.lp_cost;
        fill.redemptions += count;
        fill.isk += isk;
    }
}

/// One offer the ladder can redeem.
struct Candidate {
    offer_id: i32,
    /// Its reward type's book, as an index into the ladder's books.
    book: usize,
    quantity: u64,
    lp_cost: u64,
    /// The ISK cost and required items of one redemption.
    cost: f64,
}

/// The buy orders for one reward type, best price first, and how many units are sold into them.
struct Book {
    prices: Vec<f64>,
    /// The units of the first `i` orders at `units[i]`, from 0 up to the whole book.
    units: Vec<u64>,
    /// The revenue of the first `i` orders at `revenue[i]`.
    revenue: Vec<f64>,
    /// The units sold so far, by every offer of the type.
    sold: u64,
}

impl Book {
    /// A book of `orders`, which are best price first. A negative volume counts as none.
    fn new(orders: &[BookOrder]) -> Self {
        let mut book = Book {
            prices: Vec::with_capacity(orders.len()),
            units: Vec::with_capacity(orders.len() + 1),
            revenue: Vec::with_capacity(orders.len() + 1),
            sold: 0,
        };
        book.units.push(0);
        book.revenue.push(0.0);
        let (mut units, mut revenue) = (0u64, 0.0);
        for order in orders {
            let volume = order.volume_remain.max(0) as u64;
            units += volume;
            revenue += volume as f64 * order.price;
            book.prices.push(order.price);
            book.units.push(units);
            book.revenue.push(revenue);
        }
        book
    }

    /// The units on the whole book.
    fn total(&self) -> u64 {
        self.units[self.units.len() - 1]
    }

    /// The number of orders the first `x` units use up entirely. Below the total, that is the
    /// order the unit after them sells into, which always has volume.
    fn level(&self, x: u64) -> usize {
        self.units[1..].partition_point(|&units| units <= x)
    }

    /// The revenue of `count` units after the first `from`, with `from + count` at most the total.
    /// Units within one order are priced as one multiplication, and units across several as the
    /// part of the first, the whole orders between, and the part of the last.
    fn revenue(&self, from: u64, count: u64) -> f64 {
        let first = self.level(from);
        let to = from + count;
        if to <= self.units[first + 1] {
            return count as f64 * self.prices[first];
        }
        let last = self.level(to);
        let head = (self.units[first + 1] - from) as f64 * self.prices[first];
        let middle = self.revenue[last] - self.revenue[first + 1];
        let tail = match self.prices.get(last) {
            Some(price) => (to - self.units[last]) as f64 * price,
            None => 0.0,
        };
        head + middle + tail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    /// An offer of `quantity` units of `type_id` for `lp_cost` LP and `cost` ISK in all, whose
    /// reward sells into `book`, given as (price, volume) best first.
    fn offer(
        offer_id: i32,
        type_id: i32,
        quantity: i32,
        lp_cost: i32,
        cost: f64,
        book: &[(f64, i32)],
    ) -> Arc<OfferResult> {
        let mut result = OfferResult::default();
        result.offer.offer_id = offer_id;
        result.offer.type_id = type_id;
        result.offer.quantity = quantity;
        result.offer.lp_cost = lp_cost;
        result.breakdown.total_isk = cost;
        result.buy_orders = book
            .iter()
            .map(|&(price, volume_remain)| BookOrder {
                price,
                volume_remain,
            })
            .collect();
        Arc::new(result)
    }

    fn filled(isk: f64, lp_spent: u64, redemptions: u64, stopped: Stop) -> Option<BudgetFill> {
        Some(BudgetFill {
            isk,
            lp_spent,
            redemptions,
            stopped,
        })
    }

    #[test]
    fn one_redemption_fills_one_order_exactly() {
        let offers = [offer(1, 1, 10, 100, 200.0, &[(100.0, 10)])];
        let result = fill(&offers, 100);
        assert_eq!(result, filled(800.0, 100, 1, Stop::Book));
        assert_eq!(result.unwrap().isk_per_lp(), Some(8.0));
        // More LP changes nothing once the book is sold out.
        assert_eq!(fill(&offers, 10_000), filled(800.0, 100, 1, Stop::Book));
    }

    #[test]
    fn the_budget_limits_the_redemptions() {
        let offers = [offer(1, 1, 10, 100, 200.0, &[(100.0, 1000)])];
        assert_eq!(fill(&offers, 350), filled(2400.0, 300, 3, Stop::Budget));
    }

    #[test]
    fn the_book_limits_the_redemptions_and_a_straddle_is_priced_exactly() {
        // The second redemption sells 5 units at 100 and 5 at 50; a third has 5 units left.
        let offers = [offer(1, 1, 10, 100, 0.0, &[(100.0, 15), (50.0, 10)])];
        assert_eq!(fill(&offers, 10_000), filled(1750.0, 200, 2, Stop::Book));

        // One redemption across three orders and part of a fourth, with an empty order between.
        let offers = [offer(
            1,
            1,
            10,
            100,
            0.0,
            &[(100.0, 5), (95.0, 0), (90.0, 3), (80.0, 4)],
        )];
        assert_eq!(
            fill(&offers, 10_000),
            filled(500.0 + 270.0 + 160.0, 100, 1, Stop::Book)
        );
    }

    #[test]
    fn it_stops_when_the_next_redemption_loses_isk() {
        // The first redemption sells at 100 a unit, and the next at 10, below the 100 ISK cost.
        let offers = [offer(1, 1, 5, 10, 100.0, &[(100.0, 5), (10.0, 100)])];
        assert_eq!(
            fill(&offers, 10_000),
            filled(400.0, 10, 1, Stop::Unprofitable)
        );
        // A redemption that earns exactly nothing is not taken either.
        let offers = [offer(1, 1, 5, 10, 500.0, &[(100.0, 5)])];
        assert_eq!(fill(&offers, 10_000), filled(0.0, 0, 0, Stop::Unprofitable));
    }

    #[test]
    fn offers_of_one_type_share_the_book_and_the_better_goes_first() {
        // Offer 1 pays 5 ISK per LP and is listed first, so its 20 units are the book. Offer 2
        // pays 10, so it takes both redemptions, and offer 2's own, larger book is not used.
        let offers = [
            offer(1, 7, 10, 200, 0.0, &[(100.0, 20)]),
            offer(2, 7, 10, 100, 0.0, &[(100.0, 1000)]),
        ];
        assert_eq!(fill(&offers, 1000), filled(2000.0, 200, 2, Stop::Book));
    }

    #[test]
    fn two_types_interleave_by_isk_per_lp() {
        // Type 1 pays 10 then 4 ISK per LP, and type 2 pays 6 then 2.
        let offers = [
            offer(1, 1, 10, 100, 0.0, &[(100.0, 10), (40.0, 10)]),
            offer(2, 2, 10, 100, 0.0, &[(60.0, 10), (20.0, 10)]),
        ];
        assert_eq!(fill(&offers, 100), filled(1000.0, 100, 1, Stop::Budget));
        assert_eq!(fill(&offers, 200), filled(1600.0, 200, 2, Stop::Budget));
        assert_eq!(fill(&offers, 300), filled(2000.0, 300, 3, Stop::Budget));
        assert_eq!(fill(&offers, 400), filled(2200.0, 400, 4, Stop::Book));
    }

    #[test]
    fn an_offer_costing_more_lp_than_is_left_is_skipped() {
        // Offer 1 pays 20 ISK per LP but costs 500 LP; offer 2 pays 10 for 100 LP.
        let offers = [
            offer(1, 1, 10, 500, 0.0, &[(1000.0, 100)]),
            offer(2, 2, 10, 100, 0.0, &[(100.0, 100)]),
        ];
        assert_eq!(fill(&offers, 450), filled(4000.0, 400, 4, Stop::Budget));
    }

    #[test]
    fn nothing_to_sell_into_is_none() {
        assert_eq!(fill(&[], 1000), None);
        assert_eq!(fill(&[offer(1, 1, 10, 100, 0.0, &[])], 1000), None);
        // The book holds fewer units than one redemption.
        assert_eq!(
            fill(&[offer(1, 1, 10, 100, 0.0, &[(100.0, 9)])], 1000),
            None
        );
        // Offers without a quantity or LP cost are left out.
        assert_eq!(fill(&[offer(1, 1, 0, 100, 0.0, &[(100.0, 9)])], 1000), None);
        assert_eq!(fill(&[offer(1, 1, 1, 0, 0.0, &[(100.0, 9)])], 1000), None);
        // Even with no budget, an empty book is None.
        assert_eq!(fill(&[offer(1, 1, 10, 100, 0.0, &[])], 0), None);
    }

    #[test]
    fn no_budget_takes_nothing() {
        let offers = [offer(1, 1, 10, 100, 0.0, &[(100.0, 10)])];
        let result = fill(&offers, 0);
        assert_eq!(result, filled(0.0, 0, 0, Stop::Budget));
        assert_eq!(result.unwrap().isk_per_lp(), None);
    }

    /// The ladder one redemption at a time, walking the orders one by one, written apart from
    /// [`fill`] to check its batches and its revenue from prefix sums.
    fn reference(offers: &[Arc<OfferResult>], budget: u64) -> Option<BudgetFill> {
        let mut books: HashMap<i32, Vec<(f64, u64)>> = HashMap::new();
        let mut valid = Vec::new();
        for result in offers {
            let offer = &result.offer;
            if offer.quantity <= 0 || offer.lp_cost <= 0 {
                continue;
            }
            books.entry(offer.type_id).or_insert_with(|| {
                result
                    .buy_orders
                    .iter()
                    .map(|o| (o.price, o.volume_remain.max(0) as u64))
                    .collect()
            });
            valid.push(result.clone());
        }
        let units_left =
            |book: &Vec<(f64, u64)>| -> u64 { book.iter().map(|&(_, volume)| volume).sum() };
        if !valid
            .iter()
            .any(|r| r.offer.quantity as u64 <= units_left(&books[&r.offer.type_id]))
        {
            return None;
        }

        let mut fill = BudgetFill {
            isk: 0.0,
            lp_spent: 0,
            redemptions: 0,
            stopped: Stop::Book,
        };
        let mut left = budget;
        loop {
            let mut best: Option<(&Arc<OfferResult>, f64, f64)> = None;
            let mut any_in_book = false;
            for result in &valid {
                let offer = &result.offer;
                let book = &books[&offer.type_id];
                if (offer.quantity as u64) > units_left(book) {
                    continue;
                }
                any_in_book = true;
                if offer.lp_cost as u64 > left {
                    continue;
                }
                let mut wanted = offer.quantity as u64;
                let mut revenue = 0.0;
                for &(price, volume) in book {
                    let take = volume.min(wanted);
                    revenue += take as f64 * price;
                    wanted -= take;
                }
                let ratio = (revenue - result.breakdown.total_isk) / offer.lp_cost as f64;
                let better = best.is_none_or(|(other, _, best_ratio)| {
                    ratio > best_ratio
                        || (ratio == best_ratio && offer.offer_id < other.offer.offer_id)
                });
                if better {
                    best = Some((result, revenue, ratio));
                }
            }
            let Some((result, revenue, ratio)) = best else {
                fill.stopped = if any_in_book {
                    Stop::Budget
                } else {
                    Stop::Book
                };
                return Some(fill);
            };
            if ratio <= 0.0 {
                fill.stopped = Stop::Unprofitable;
                return Some(fill);
            }
            let offer = &result.offer;
            let mut wanted = offer.quantity as u64;
            for (_, volume) in books.get_mut(&offer.type_id).unwrap() {
                let take = (*volume).min(wanted);
                *volume -= take;
                wanted -= take;
            }
            left -= offer.lp_cost as u64;
            fill.lp_spent += offer.lp_cost as u64;
            fill.redemptions += 1;
            fill.isk += revenue - result.breakdown.total_isk;
        }
    }

    #[test]
    fn batches_match_one_redemption_at_a_time() {
        // Whole-number prices, volumes and costs keep every sum exact, so the two must agree to
        // the last bit, including on which tied offer goes first.
        let mut rng = StdRng::seed_from_u64(0x1b5_70e);
        let mut stops = HashMap::new();
        for _ in 0..3000 {
            let types = rng.gen_range(1..=3);
            let books: Vec<Vec<(f64, i32)>> = (0..types)
                .map(|_| {
                    let mut book: Vec<(f64, i32)> = (0..rng.gen_range(0..=6))
                        .map(|_| (rng.gen_range(1..=1000) as f64, rng.gen_range(-2..=60)))
                        .collect();
                    book.sort_by(|a, b| b.0.total_cmp(&a.0));
                    book
                })
                .collect();
            let offers: Vec<Arc<OfferResult>> = (0..rng.gen_range(1..=6))
                .map(|offer_id| {
                    let type_id = rng.gen_range(0..types);
                    offer(
                        offer_id,
                        type_id as i32,
                        rng.gen_range(0..=20),
                        rng.gen_range(0..=500),
                        rng.gen_range(0..=8000) as f64,
                        &books[type_id],
                    )
                })
                .collect();
            let budget = rng.gen_range(0..=6000);
            let expected = reference(&offers, budget);
            assert_eq!(
                fill(&offers, budget),
                expected,
                "budget {budget}, {offers:?}"
            );
            *stops.entry(expected.map(|f| f.stopped)).or_insert(0u32) += 1;
        }
        // Every way of stopping, and None, came up.
        assert_eq!(stops.len(), 4, "{stops:?}");
    }
}

//! The `--lpstores` report: option checks, fetching, pricing, and grouping.

mod options;
mod pipeline;

use crate::lpstore::{OfferError, OfferResult};
use std::time::Duration;

pub use options::{
    DEFAULT_DEPTH, DEFAULT_REGION_ID, Field, FilterSummary, LpStoreOptions, NamedValue,
    OptionError, Parsed, Resolved, SystemSummary,
};
pub use pipeline::run;

/// The priced offers for each corporation group, best first.
#[derive(Debug)]
pub struct Report {
    pub groups: Vec<Group>,
}

impl Report {
    /// Orders the groups by the average ISK per LP over the top `depth` offers, best first,
    /// with ties broken by the lower first corporation ID.
    pub fn sort_by_depth(&mut self, depth: usize) {
        let key = |group: &Group| group.average_isk_per_lp(depth).unwrap_or(f64::NEG_INFINITY);
        self.groups.sort_by(|a, b| {
            key(b)
                .total_cmp(&key(a))
                .then_with(|| a.corp_ids[0].cmp(&b.corp_ids[0]))
        });
    }
}

/// Corporations whose loyalty offers are identical, with those offers priced.
#[derive(Debug)]
pub struct Group {
    pub corp_ids: Vec<i32>,
    pub offer_count: usize,
    pub results: Vec<OfferResult>,
    pub errors: Vec<OfferError>,
}

impl Group {
    /// The average ISK per LP over the top `depth` priced offers, or `None` when none of them
    /// have a value. Offers without one are left out of the average.
    pub fn average_isk_per_lp(&self, depth: usize) -> Option<f64> {
        let top = self.results.get(..depth.min(self.results.len()))?;
        let (sum, count) = top
            .iter()
            .filter_map(|result| result.isk_per_lp)
            .fold((0.0, 0usize), |(sum, count), value| {
                (sum + value, count + 1)
            });
        (count > 0).then(|| sum / count as f64)
    }
}

/// A progress event, tagged with the run it belongs to.
#[derive(Debug, Clone)]
pub struct RunEvent {
    pub run: u64,
    pub event: Event,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A verbose line from the ESI client.
    Log(String),
    FetchStarted {
        corporations: usize,
    },
    /// Every fetch task is queued; this many have not finished yet.
    FetchQueued {
        tasks: usize,
    },
    FetchProgress {
        done: usize,
        total: usize,
    },
    /// A fetch task ended without a result, such as a panic.
    FetchTaskFailed {
        error: String,
    },
    /// The offers of one corporation could not be fetched.
    CorpFetchFailed {
        corp_id: i32,
        error: String,
    },
    FetchFinished(FetchSummary),
    PricingStarted {
        groups: usize,
    },
    /// Every pricing task is queued; this many have not finished yet.
    PricingQueued {
        tasks: usize,
    },
    PricingProgress {
        done: usize,
        total: usize,
    },
    /// A pricing task ended without a result, such as a panic.
    GroupFailed {
        error: String,
    },
    PricingFinished(PricingSummary),
}

/// Cache activity during one phase of a run.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheCounts {
    pub hits: u64,
    pub misses: u64,
    pub writes: u64,
}

#[derive(Debug, Clone)]
pub struct FetchSummary {
    pub corporations: usize,
    pub elapsed: Duration,
    pub errors: usize,
    pub cache: CacheCounts,
}

#[derive(Debug, Clone)]
pub struct PricingSummary {
    pub groups: usize,
    pub elapsed: Duration,
    pub offer_errors: usize,
    pub group_failures: usize,
    pub cache: CacheCounts,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esi::loyalty::LoyaltyOffer;
    use crate::expenses::ExpenseBreakdown;

    fn group(isk_per_lp: &[Option<f64>]) -> Group {
        let results: Vec<OfferResult> = isk_per_lp
            .iter()
            .map(|&isk_per_lp| OfferResult {
                offer: LoyaltyOffer {
                    offer_id: 1,
                    type_id: 1,
                    quantity: 1,
                    lp_cost: 1,
                    isk_cost: 0.0,
                    required_items: Vec::new(),
                },
                breakdown: ExpenseBreakdown {
                    offer_id: 1,
                    lp_cost: 1,
                    isk_cost: 0.0,
                    item_expenses: Vec::new(),
                    total_isk: 0.0,
                },
                reward: None,
                unit_cost: None,
                isk_per_lp,
                best_ask: None,
                spread: None,
                book_depth: None,
                imbalance: None,
                within_10: None,
            })
            .collect();
        Group {
            corp_ids: vec![1],
            offer_count: results.len(),
            results,
            errors: Vec::new(),
        }
    }

    #[test]
    fn the_average_covers_only_the_top_depth_offers_that_have_a_value() {
        let group = group(&[Some(10.0), Some(2.0), None, Some(2.0)]);
        assert_eq!(group.average_isk_per_lp(10), Some(14.0 / 3.0));
        assert_eq!(group.average_isk_per_lp(3), Some(6.0));
        assert_eq!(group.average_isk_per_lp(1), Some(10.0));
    }

    #[test]
    fn the_average_is_none_without_a_valued_offer() {
        assert_eq!(group(&[None, None]).average_isk_per_lp(10), None);
        assert_eq!(group(&[]).average_isk_per_lp(10), None);
    }
}

//! The `--lpstores` report: option checks, fetching, pricing, and grouping.

mod options;
mod pipeline;

use crate::lpstore::{OfferError, OfferResult};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

pub use options::{
    DEFAULT_DEPTH, DEFAULT_REGION_ID, Field, FieldSpec, FilterSummary, LpStoreOptions, NamedValue,
    OptionError, Parsed, Resolved, SystemSummary, region_id,
};
pub use pipeline::run;

/// The priced offers of a run, and the corporation groups they are shown in.
#[derive(Debug)]
pub struct Report {
    /// Corporations whose loyalty stores hold the same offers, each with its offers ranked. The
    /// groups are built from these, so a new depth regroups without pricing anything again.
    pub stores: Vec<Store>,
    /// The offers that failed to price, each once, in offer ID order.
    pub errors: Vec<OfferError>,
    /// The groups for the depth last given to [`Report::regroup`], best first.
    pub groups: Vec<Group>,
}

impl Report {
    /// A report of `stores` and the offers that failed, grouped for `depth`.
    pub fn new(stores: Vec<Store>, errors: Vec<OfferError>, depth: usize) -> Self {
        let mut report = Self {
            stores,
            errors,
            groups: Vec::new(),
        };
        report.regroup(depth);
        report
    }

    /// Builds the groups for `depth`, one per store, and orders them by the average ISK per LP
    /// over their top `depth` offers, best first, with ties broken by the lower first
    /// corporation ID.
    pub fn regroup(&mut self, depth: usize) {
        let mut groups: Vec<Group> = self.stores.iter().map(Group::of_store).collect();
        let key = |group: &Group| group.average_isk_per_lp(depth).unwrap_or(f64::NEG_INFINITY);
        groups.sort_by(|a, b| {
            key(b)
                .total_cmp(&key(a))
                .then_with(|| a.corp_ids[0].cmp(&b.corp_ids[0]))
        });
        self.groups = groups;
    }

    /// How many different offers the stores hold, priced or not, which is how many were priced.
    pub fn offer_count(&self) -> usize {
        let offers: HashSet<i32> = self
            .stores
            .iter()
            .flat_map(|store| store.offer_ids.iter().copied())
            .collect();
        offers.len()
    }

    /// The corporations whose stores hold `offer_id`, in store order.
    pub fn corporations_with(&self, offer_id: i32) -> Vec<i32> {
        self.stores
            .iter()
            .filter(|store| store.offer_ids.binary_search(&offer_id).is_ok())
            .flat_map(|store| store.corp_ids.iter().copied())
            .collect()
    }
}

/// Corporations whose loyalty stores hold the same offers, with the priced ones ranked.
#[derive(Debug, Clone)]
pub struct Store {
    pub corp_ids: Vec<i32>,
    /// Every offer in the store, priced or not, sorted. Equal lists make corporations one store.
    pub offer_ids: Vec<i32>,
    /// The offers that priced, in [`ranking`] order.
    pub results: Vec<Arc<OfferResult>>,
}

impl Store {
    /// A store of `corp_ids` holding `offer_ids`, with `results` ranked.
    pub fn new(
        corp_ids: Vec<i32>,
        mut offer_ids: Vec<i32>,
        mut results: Vec<Arc<OfferResult>>,
    ) -> Self {
        offer_ids.sort_unstable();
        results.sort_by(|a, b| ranking(a, b));
        Self {
            corp_ids,
            offer_ids,
            results,
        }
    }
}

/// The order offers are listed in: ISK per LP, best first, then offers without a value, with ties
/// going to the lower offer ID. The tie-break makes it a total order, so the same offers rank the
/// same way in every store that holds them.
fn ranking(a: &OfferResult, b: &OfferResult) -> Ordering {
    let value = |result: &OfferResult| result.isk_per_lp.unwrap_or(f64::NEG_INFINITY);
    value(b)
        .total_cmp(&value(a))
        .then_with(|| a.offer.offer_id.cmp(&b.offer.offer_id))
}

/// Corporations listed together, with their offers best first.
#[derive(Debug)]
pub struct Group {
    pub corp_ids: Vec<i32>,
    pub offer_count: usize,
    pub results: Vec<Arc<OfferResult>>,
}

impl Group {
    fn of_store(store: &Store) -> Self {
        Self {
            corp_ids: store.corp_ids.clone(),
            offer_count: store.offer_ids.len(),
            results: store.results.clone(),
        }
    }

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
    /// Pricing has started on this many offers, each priced once however many stores hold it.
    PricingStarted {
        offers: usize,
    },
    /// Every pricing task is queued; this many have not finished yet.
    PricingQueued {
        tasks: usize,
    },
    PricingProgress {
        done: usize,
        total: usize,
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
    pub offers: usize,
    pub elapsed: Duration,
    pub offer_errors: usize,
    pub cache: CacheCounts,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Results with these ISK per LP values, in this order. Offer IDs are `first_id` and up.
    fn results(first_id: i32, isk_per_lp: &[Option<f64>]) -> Vec<Arc<OfferResult>> {
        isk_per_lp
            .iter()
            .enumerate()
            .map(|(index, &isk_per_lp)| {
                let mut result = OfferResult {
                    isk_per_lp,
                    ..Default::default()
                };
                result.offer.offer_id = first_id + index as i32;
                Arc::new(result)
            })
            .collect()
    }

    /// A group of `corp_ids` whose offers have these ISK per LP values, in this order.
    fn group(corp_ids: &[i32], isk_per_lp: &[Option<f64>]) -> Group {
        let results = results(1, isk_per_lp);
        Group {
            corp_ids: corp_ids.to_vec(),
            offer_count: results.len(),
            results,
        }
    }

    /// The store of `corp_ids`, whose offers have these ISK per LP values. Offer IDs start at the
    /// first corporation's times a hundred, so no two stores share an offer.
    fn store(corp_ids: &[i32], isk_per_lp: &[Option<f64>]) -> Store {
        let results = results(corp_ids[0] * 100, isk_per_lp);
        let offer_ids = results.iter().map(|result| result.offer.offer_id).collect();
        Store::new(corp_ids.to_vec(), offer_ids, results)
    }

    #[test]
    fn the_average_covers_only_the_top_depth_offers_that_have_a_value() {
        let group = group(&[1], &[Some(10.0), Some(2.0), None, Some(2.0)]);
        assert_eq!(group.average_isk_per_lp(10), Some(14.0 / 3.0));
        assert_eq!(group.average_isk_per_lp(3), Some(6.0));
        assert_eq!(group.average_isk_per_lp(1), Some(10.0));
    }

    #[test]
    fn the_average_is_none_without_a_valued_offer() {
        assert_eq!(group(&[1], &[None, None]).average_isk_per_lp(10), None);
        assert_eq!(group(&[1], &[]).average_isk_per_lp(10), None);
    }

    /// Groups `stores` for `depth` and returns the groups' first corporation IDs, in order.
    fn sorted_corp_ids(stores: Vec<Store>, depth: usize) -> Vec<i32> {
        let report = Report::new(stores, Vec::new(), depth);
        report.groups.iter().map(|g| g.corp_ids[0]).collect()
    }

    #[test]
    fn groups_sort_best_first_with_ties_by_first_corporation() {
        let order = sorted_corp_ids(
            vec![
                store(&[30], &[Some(1.0)]),
                store(&[20], &[]),
                store(&[10], &[]),
                store(&[40], &[Some(5.0)]),
                store(&[15], &[Some(5.0)]),
            ],
            10,
        );
        assert_eq!(order, [15, 40, 30, 10, 20]);
    }

    #[test]
    fn groups_sort_by_the_average_over_the_top_depth_offers() {
        // [10, 2, 2] averages 4.67 over three offers, while [6, 6, 6] averages 6.
        let order = sorted_corp_ids(
            vec![
                store(&[30], &[Some(10.0), Some(2.0), Some(2.0)]),
                store(&[10], &[Some(6.0), Some(6.0), Some(6.0)]),
            ],
            3,
        );
        assert_eq!(order, [10, 30]);
    }

    #[test]
    fn the_average_only_uses_the_top_depth_offers() {
        // Over the top two offers both groups average 6 and tie, so the lower first
        // corporation ID wins. Their third offers are far apart and must not matter.
        let order = sorted_corp_ids(
            vec![
                store(&[40], &[Some(10.0), Some(2.0), Some(2.0)]),
                store(&[15], &[Some(6.0), Some(6.0), Some(0.0)]),
            ],
            2,
        );
        assert_eq!(order, [15, 40]);
    }

    #[test]
    fn offers_without_a_value_are_left_out_of_the_average() {
        // Only the first offer has a value, so both groups average it alone.
        let order = sorted_corp_ids(
            vec![
                store(&[40], &[Some(5.0), None, None]),
                store(&[15], &[Some(5.0), None, None]),
            ],
            10,
        );
        assert_eq!(order, [15, 40]);

        let order = sorted_corp_ids(vec![store(&[40], &[None, None]), store(&[15], &[None])], 10);
        assert_eq!(order, [15, 40], "groups with no valued offers sort last");
    }

    #[test]
    fn offers_rank_by_isk_per_lp_then_by_offer_id() {
        // Equal values, and offers without one, keep no order of their own: ties go to the lower
        // offer ID, so two stores holding the same offers rank them the same way.
        let value = |offer_id: i32, isk_per_lp: Option<f64>| {
            let mut result = OfferResult {
                isk_per_lp,
                ..Default::default()
            };
            result.offer.offer_id = offer_id;
            Arc::new(result)
        };
        let results = vec![
            value(5, Some(2.0)),
            value(3, None),
            value(1, Some(2.0)),
            value(4, Some(9.0)),
            value(2, None),
        ];
        let store = Store::new(vec![1], vec![5, 3, 1, 4, 2], results);
        let order: Vec<i32> = store
            .results
            .iter()
            .map(|result| result.offer.offer_id)
            .collect();
        assert_eq!(order, [4, 1, 5, 2, 3]);
        assert_eq!(store.offer_ids, [1, 2, 3, 4, 5], "sorted");
    }

    #[test]
    fn an_offer_two_stores_hold_counts_once_and_names_both() {
        let mut first = store(&[10, 11], &[Some(1.0)]);
        let mut second = store(&[20], &[Some(1.0)]);
        // Offer 7 failed to price, so neither store has a result for it.
        first.offer_ids.push(7);
        second.offer_ids.push(7);
        first.offer_ids.sort_unstable();
        second.offer_ids.sort_unstable();
        let report = Report::new(vec![first, second], Vec::new(), 10);
        assert_eq!(report.offer_count(), 3, "1000, 2000, and 7 once");
        assert_eq!(report.corporations_with(7), [10, 11, 20]);
        assert_eq!(report.corporations_with(2000), [20]);
    }
}

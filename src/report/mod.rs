//! The `--lpstores` report: option checks, fetching, pricing, and grouping.

mod options;
mod pipeline;

use crate::lpstore::{OfferError, OfferResult};
use std::time::Duration;

pub use options::{
    DEFAULT_DEPTH, DEFAULT_REGION_ID, Field, FieldSpec, FilterSummary, LpStoreOptions, NamedValue,
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

    /// A group of `corp_ids` whose offers have these ISK per LP values, in order.
    fn group(corp_ids: &[i32], isk_per_lp: &[Option<f64>]) -> Group {
        let results: Vec<OfferResult> = isk_per_lp
            .iter()
            .map(|&isk_per_lp| OfferResult {
                isk_per_lp,
                ..Default::default()
            })
            .collect();
        Group {
            corp_ids: corp_ids.to_vec(),
            offer_count: results.len(),
            results,
            errors: Vec::new(),
        }
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

    /// Sorts `groups` with `Report::sort_by_depth` and returns their first corporation IDs.
    fn sorted_corp_ids(groups: Vec<Group>, depth: usize) -> Vec<i32> {
        let mut report = Report { groups };
        report.sort_by_depth(depth);
        report.groups.iter().map(|g| g.corp_ids[0]).collect()
    }

    #[test]
    fn groups_sort_best_first_with_ties_by_first_corporation() {
        let order = sorted_corp_ids(
            vec![
                group(&[30], &[Some(1.0)]),
                group(&[20], &[]),
                group(&[10], &[]),
                group(&[40], &[Some(5.0)]),
                group(&[15], &[Some(5.0)]),
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
                group(&[30], &[Some(10.0), Some(2.0), Some(2.0)]),
                group(&[10], &[Some(6.0), Some(6.0), Some(6.0)]),
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
                group(&[40], &[Some(10.0), Some(2.0), Some(2.0)]),
                group(&[15], &[Some(6.0), Some(6.0), Some(0.0)]),
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
                group(&[40], &[Some(5.0), None, None]),
                group(&[15], &[Some(5.0), None, None]),
            ],
            10,
        );
        assert_eq!(order, [15, 40]);

        let order = sorted_corp_ids(vec![group(&[40], &[None, None]), group(&[15], &[None])], 10);
        assert_eq!(order, [15, 40], "groups with no valued offers sort last");
    }
}

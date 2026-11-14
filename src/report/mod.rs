//! The `--lpstores` report: option checks, fetching, pricing, and grouping.

mod arrange;
mod options;
mod pipeline;

pub mod budget;

use crate::lpstore::{OfferError, OfferResult};
use budget::BudgetFill;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub use arrange::{Arrangement, DEFAULT_LP_BUDGET, GroupSort, OfferFilter};
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
    /// The groups for the arrangement last given to [`Report::arrange`], in its sort order.
    pub groups: Vec<Group>,
    /// Each corporation's place in the run, which a group lists its corporations in.
    positions: HashMap<i32, usize>,
    /// What `groups` was built and sorted for.
    arrangement: Arrangement,
}

impl Report {
    /// A report of `stores` and the offers that failed, grouped for `depth`. `corporations` is
    /// the run's order, which groups list their corporations in; any it leaves out go after, in
    /// store order.
    pub fn new(
        corporations: &[i32],
        stores: Vec<Store>,
        errors: Vec<OfferError>,
        depth: usize,
    ) -> Self {
        let positions = corporations
            .iter()
            .enumerate()
            .map(|(position, &corp_id)| (corp_id, position))
            .collect();
        let mut report = Self {
            stores,
            errors,
            groups: Vec::new(),
            positions,
            arrangement: Arrangement::depth(depth),
        };
        report.rebuild();
        report
    }

    /// Groups for `depth` over every priced offer, by the average ISK per LP, best first, as
    /// [`Arrangement::depth`] describes. Any filter, exclusion, or sort set before is dropped.
    pub fn regroup(&mut self, depth: usize) {
        self.arrange(&Arrangement::depth(depth));
    }

    /// Lists together the stores whose top `depth` visible offers are the same, in the same
    /// order, and sorts the groups as `arrangement` says. An offer is visible when
    /// [`Arrangement::visible`] says so, so a hidden offer leaves room for the next one that
    /// counts. A store with fewer visible offers than `depth` only joins stores listing exactly
    /// its offers. Nothing is priced again, so this is cheap enough to repeat on every change.
    pub fn arrange(&mut self, arrangement: &Arrangement) {
        self.arrangement = arrangement.clone();
        self.rebuild();
    }

    /// What the groups were last built and sorted for.
    pub fn arrangement(&self) -> &Arrangement {
        &self.arrangement
    }

    /// Sorts the groups again by `sort`, without regrouping, since the groups do not depend on it.
    pub fn sort_groups(&mut self, sort: GroupSort, reverse: bool) {
        self.arrangement.sort = sort;
        self.arrangement.reverse = reverse;
        sort_groups(&mut self.groups, &self.arrangement);
    }

    fn rebuild(&mut self) {
        let arrangement = &self.arrangement;
        let mut found: HashMap<Vec<i32>, usize> = HashMap::new();
        let mut members: Vec<Vec<&Store>> = Vec::new();
        for store in &self.stores {
            let listed: Vec<i32> = store
                .listed(arrangement)
                .map(|result| result.offer.offer_id)
                .collect();
            let index = *found.entry(listed).or_insert_with(|| {
                members.push(Vec::new());
                members.len() - 1
            });
            members[index].push(store);
        }
        let mut groups: Vec<Group> = members
            .iter()
            .map(|stores| Group::of_stores(stores, arrangement, &self.positions))
            .collect();
        sort_groups(&mut groups, arrangement);
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

    /// The corporations whose stores hold `offer_id`, each with its store's offer count, in the
    /// run's order.
    pub fn corporations_with(&self, offer_id: i32) -> Vec<(i32, usize)> {
        let mut corporations: Vec<(i32, usize)> = self
            .stores
            .iter()
            .filter(|store| store.offer_ids.binary_search(&offer_id).is_ok())
            .flat_map(Store::corporations)
            .collect();
        self.in_run_order(&mut corporations);
        corporations
    }

    /// The fewest and the most priced offers the arrangement's filter and exclusions take out of
    /// the stores of `group`'s corporations, counted as [`Group::hidden_counts`] counts them with
    /// hiding on. With hiding on this is [`Group::hidden_range`]; with it off, where a group hides
    /// nothing, it is what turning hiding back on would hide from those stores.
    pub fn hideable_range(&self, group: &Group) -> (usize, usize) {
        let hiding = Arrangement {
            hiding: true,
            ..self.arrangement.clone()
        };
        let counts: Vec<usize> = group
            .corp_ids
            .iter()
            .filter_map(|corp_id| {
                self.stores
                    .iter()
                    .find(|store| store.corp_ids.contains(corp_id))
            })
            .map(|store| store.hidden_count(&hiding))
            .collect();
        range(&counts)
    }

    /// [`Report::hideable_range`] printed the way [`Group::hidden_count_text`] prints its range.
    pub fn hideable_count_text(&self, group: &Group) -> String {
        range_text(self.hideable_range(group))
    }

    fn in_run_order(&self, corporations: &mut [(i32, usize)]) {
        in_run_order(&self.positions, corporations);
    }
}

/// Sorts `corporations` into the run's order, keeping the order of any the run does not place.
fn in_run_order<T>(positions: &HashMap<i32, usize>, corporations: &mut [(i32, T)]) {
    corporations.sort_by_key(|(corp_id, _)| positions.get(corp_id).copied().unwrap_or(usize::MAX));
}

/// Orders `groups` by the value `arrangement.sort` gives each, best first, or worst first when
/// `arrangement.reverse` is set. Groups without a value go last in both directions. Ties, and
/// groups without a value, go to the lower first corporation ID.
fn sort_groups(groups: &mut [Group], arrangement: &Arrangement) {
    let key = |group: &Group| group.sort_value(arrangement.sort, arrangement.depth);
    groups.sort_by(|a, b| {
        let by_value = match (key(a), key(b)) {
            (Some(a), Some(b)) if arrangement.reverse => a.total_cmp(&b),
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_value.then_with(|| a.corp_ids[0].cmp(&b.corp_ids[0]))
    });
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

    /// Each of its corporations with the store's offer count.
    fn corporations(&self) -> impl Iterator<Item = (i32, usize)> + '_ {
        let count = self.offer_ids.len();
        self.corp_ids.iter().map(move |&corp_id| (corp_id, count))
    }

    /// The offers it lists under `arrangement`: the top `depth` of its visible offers, in
    /// [`ranking`] order. Whether an offer is visible depends on the offer alone, so these are a
    /// subsequence of a total order, and two stores holding the same visible offers list them
    /// the same way whatever hidden offers lie between them. Grouping relies on that.
    fn listed<'a>(
        &'a self,
        arrangement: &'a Arrangement,
    ) -> impl Iterator<Item = &'a Arc<OfferResult>> + 'a {
        self.results
            .iter()
            .filter(|result| arrangement.visible(result))
            .take(arrangement.depth)
    }

    /// How many of its priced offers `arrangement` hides.
    fn hidden_count(&self, arrangement: &Arrangement) -> usize {
        self.results
            .iter()
            .filter(|result| !arrangement.visible(result))
            .count()
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

/// Corporations whose top offers, as many as the depth, are the same, listed together with those
/// offers best first.
#[derive(Debug)]
pub struct Group {
    /// In the run's order.
    pub corp_ids: Vec<i32>,
    /// How many offers each corporation's store holds, in `corp_ids` order. They differ when the
    /// group joins stores that differ only below the offers it lists.
    pub offer_counts: Vec<usize>,
    /// How many priced offers in each corporation's store the arrangement hides, in `corp_ids`
    /// order, counted over the whole store rather than only above the offers it lists.
    pub hidden_counts: Vec<usize>,
    /// The offers it lists, at most the depth it was built for.
    pub results: Vec<Arc<OfferResult>>,
    /// What spending the arrangement's LP budget on the offers it lists realizes, or `None` when
    /// the arrangement sets no budget or none of those offers can be redeemed into a buy order.
    /// Hidden offers, and offers below the depth, take no part.
    pub budget: Option<BudgetFill>,
}

impl Group {
    /// The group of `stores`, which list the same offers under `arrangement`.
    fn of_stores(
        stores: &[&Store],
        arrangement: &Arrangement,
        positions: &HashMap<i32, usize>,
    ) -> Self {
        let mut corporations: Vec<(i32, (usize, usize))> = stores
            .iter()
            .flat_map(|store| {
                let hidden = store.hidden_count(arrangement);
                store
                    .corporations()
                    .map(move |(corp_id, count)| (corp_id, (count, hidden)))
            })
            .collect();
        in_run_order(positions, &mut corporations);
        let (corp_ids, counts): (Vec<i32>, Vec<(usize, usize)>) = corporations.into_iter().unzip();
        let (offer_counts, hidden_counts) = counts.into_iter().unzip();
        let results: Vec<Arc<OfferResult>> = stores[0].listed(arrangement).cloned().collect();
        let budget = arrangement
            .lp_budget
            .and_then(|lp| budget::fill(&results, lp));
        Self {
            corp_ids,
            offer_counts,
            hidden_counts,
            results,
            budget,
        }
    }

    /// The fewest and the most offers its corporations' stores hold.
    pub fn offer_range(&self) -> (usize, usize) {
        range(&self.offer_counts)
    }

    /// The offer count as the CLI and the view print it: one number, or the fewest and the most,
    /// such as `118–127`.
    pub fn offer_count_text(&self) -> String {
        range_text(self.offer_range())
    }

    /// The fewest and the most offers the arrangement hides in its corporations' stores.
    pub fn hidden_range(&self) -> (usize, usize) {
        range(&self.hidden_counts)
    }

    /// The hidden count printed the way [`Group::offer_count_text`] prints the offer count.
    pub fn hidden_count_text(&self) -> String {
        range_text(self.hidden_range())
    }

    /// The ISK per LP of the first offer it lists, which is its best, or `None` when it lists
    /// nothing or the first offer has no value.
    pub fn best_isk_per_lp(&self) -> Option<f64> {
        self.results.first()?.isk_per_lp
    }

    /// The profit within 10% of the best bid, summed over the offers it lists that have one, or
    /// `None` when none of them do.
    pub fn profit_within_10(&self) -> Option<f64> {
        self.results
            .iter()
            .filter_map(|result| result.within_10.map(|(_, profit)| profit))
            .reduce(|sum, profit| sum + profit)
    }

    /// The value `sort` orders groups by, for a group built at `depth`, or `None` when the group
    /// has none.
    pub fn sort_value(&self, sort: GroupSort, depth: usize) -> Option<f64> {
        match sort {
            GroupSort::AverageIskPerLp => self.average_isk_per_lp(depth),
            GroupSort::BudgetIsk => self.budget.map(|fill| fill.isk),
            GroupSort::BestOffer => self.best_isk_per_lp(),
            GroupSort::ProfitWithin10 => self.profit_within_10(),
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

/// The fewest and the most of `counts`, or zero for both when there are none.
fn range(counts: &[usize]) -> (usize, usize) {
    let fewest = counts.iter().copied().min().unwrap_or(0);
    let most = counts.iter().copied().max().unwrap_or(0);
    (fewest, most)
}

/// One number when the fewest and the most agree, otherwise both, such as `118–127`.
fn range_text((fewest, most): (usize, usize)) -> String {
    if fewest == most {
        fewest.to_string()
    } else {
        format!("{fewest}–{most}")
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
            offer_counts: vec![results.len(); corp_ids.len()],
            hidden_counts: vec![0; corp_ids.len()],
            results,
            budget: None,
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
        let report = Report::new(&[], stores, Vec::new(), depth);
        report.groups.iter().map(|g| g.corp_ids[0]).collect()
    }

    #[test]
    fn groups_sort_best_first_with_ties_by_first_corporation() {
        let order = sorted_corp_ids(
            vec![
                store(&[30], &[Some(1.0)]),
                // Different offers, both without a value, so neither joins the other.
                store(&[20], &[None]),
                store(&[10], &[None]),
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
        let report = Report::new(&[], vec![first, second], Vec::new(), 10);
        assert_eq!(report.offer_count(), 3, "1000, 2000, and 7 once");
        assert_eq!(report.corporations_with(7), [(10, 2), (11, 2), (20, 2)]);
        assert_eq!(report.corporations_with(2000), [(20, 2)]);
    }

    /// A store of `corp_ids` holding offers with these (offer ID, ISK per LP) pairs.
    fn holding(corp_ids: &[i32], offers: &[(i32, f64)]) -> Store {
        let results: Vec<Arc<OfferResult>> = offers
            .iter()
            .map(|&(offer_id, isk_per_lp)| {
                let mut result = OfferResult {
                    isk_per_lp: Some(isk_per_lp),
                    ..Default::default()
                };
                result.offer.offer_id = offer_id;
                Arc::new(result)
            })
            .collect();
        let offer_ids = offers.iter().map(|&(offer_id, _)| offer_id).collect();
        Store::new(corp_ids.to_vec(), offer_ids, results)
    }

    /// The groups' corporations, best group first.
    fn grouped(report: &Report) -> Vec<Vec<i32>> {
        report.groups.iter().map(|g| g.corp_ids.clone()).collect()
    }

    #[test]
    fn stores_listing_the_same_top_offers_are_one_group() {
        // Both list offers 1 and 2 at depth 2. They differ below that, in what and how many.
        let stores = vec![
            holding(&[10], &[(1, 9.0), (2, 8.0), (3, 1.0)]),
            holding(&[20], &[(1, 9.0), (2, 8.0), (4, 2.0), (5, 0.5)]),
            holding(&[30], &[(1, 9.0), (6, 7.0)]),
        ];
        let report = Report::new(&[10, 20, 30], stores, Vec::new(), 2);
        assert_eq!(grouped(&report), [vec![10, 20], vec![30]]);
        let joined = &report.groups[0];
        assert_eq!(joined.offer_counts, [3, 4]);
        assert_eq!(joined.offer_count_text(), "3–4");
        assert_eq!(report.groups[1].offer_count_text(), "2");
        let listed: Vec<i32> = joined.results.iter().map(|r| r.offer.offer_id).collect();
        assert_eq!(listed, [1, 2], "only the shared offers");
    }

    #[test]
    fn a_new_depth_joins_or_splits_groups() {
        let stores = vec![
            holding(&[10], &[(1, 9.0), (2, 8.0), (3, 1.0)]),
            holding(&[20], &[(1, 9.0), (2, 8.0), (4, 2.0)]),
            holding(&[30], &[(1, 9.0), (6, 7.0)]),
        ];
        let mut report = Report::new(&[10, 20, 30], stores, Vec::new(), 3);
        assert_eq!(grouped(&report), [vec![30], vec![20], vec![10]]);
        report.regroup(1);
        assert_eq!(
            grouped(&report),
            [vec![10, 20, 30]],
            "all list offer 1 first"
        );
        report.regroup(2);
        assert_eq!(grouped(&report), [vec![10, 20], vec![30]]);
    }

    #[test]
    fn a_store_with_fewer_offers_than_the_depth_joins_only_the_same_list() {
        // 30 lists offers 1 and 2 and nothing else. At depth 3, 10 lists a third offer, so they
        // are not the same list; 20 lists exactly what 30 does.
        let stores = vec![
            holding(&[10], &[(1, 9.0), (2, 8.0), (3, 1.0)]),
            holding(&[20], &[(1, 9.0), (2, 8.0)]),
            holding(&[30], &[(1, 9.0), (2, 8.0)]),
        ];
        let report = Report::new(&[10, 20, 30], stores, Vec::new(), 3);
        assert_eq!(grouped(&report), [vec![20, 30], vec![10]]);
    }

    #[test]
    fn stores_listing_nothing_are_one_group() {
        let stores = vec![
            holding(&[10], &[]),
            holding(&[20], &[]),
            holding(&[30], &[(1, 1.0)]),
        ];
        let report = Report::new(&[10, 20, 30], stores, Vec::new(), 10);
        assert_eq!(grouped(&report), [vec![30], vec![10, 20]]);
    }

    #[test]
    fn a_joined_group_lists_its_corporations_in_the_runs_order() {
        // The run is 1, 3, 5, so the first store's 1 and 5 do not stay together ahead of 3.
        let stores = vec![
            holding(&[1, 5], &[(7, 2.0), (8, 1.0)]),
            holding(&[3], &[(7, 2.0)]),
        ];
        let report = Report::new(&[1, 3, 5], stores, Vec::new(), 1);
        assert_eq!(grouped(&report), [vec![1, 3, 5]]);
        assert_eq!(report.groups[0].offer_counts, [2, 1, 2]);
    }

    /// An offer with these figures and nothing else priced.
    fn offer(
        offer_id: i32,
        isk_per_lp: Option<f64>,
        spread: Option<f64>,
        within_10: Option<(i64, f64)>,
    ) -> OfferResult {
        let mut result = OfferResult {
            isk_per_lp,
            spread,
            within_10,
            ..Default::default()
        };
        result.offer.offer_id = offer_id;
        result
    }

    /// The store of `corp_ids` holding `offers`.
    fn store_of(corp_ids: &[i32], offers: Vec<OfferResult>) -> Store {
        let offer_ids = offers.iter().map(|result| result.offer.offer_id).collect();
        let results = offers.into_iter().map(Arc::new).collect();
        Store::new(corp_ids.to_vec(), offer_ids, results)
    }

    /// An offer with this ISK per LP and a spread of `spread`.
    fn spread(offer_id: i32, isk_per_lp: f64, spread: f64) -> OfferResult {
        offer(offer_id, Some(isk_per_lp), Some(spread), None)
    }

    /// The offer IDs the group lists.
    fn listed(group: &Group) -> Vec<i32> {
        group.results.iter().map(|r| r.offer.offer_id).collect()
    }

    /// An arrangement for `depth` with spreads above 50% filtered out.
    fn narrow_spreads(depth: usize) -> Arrangement {
        let mut arrangement = Arrangement::depth(depth);
        arrangement.filter.max_spread = Some(50.0);
        arrangement
    }

    #[test]
    fn a_failing_top_offer_drops_out_and_the_next_passing_one_fills_the_top() {
        let stores = vec![store_of(
            &[10],
            vec![
                spread(1, 9.0, 80.0),
                spread(2, 8.0, 5.0),
                spread(3, 7.0, 5.0),
                spread(4, 6.0, 5.0),
            ],
        )];
        let mut report = Report::new(&[10], stores, Vec::new(), 2);
        assert_eq!(listed(&report.groups[0]), [1, 2]);
        report.arrange(&narrow_spreads(2));
        assert_eq!(listed(&report.groups[0]), [2, 3], "still two offers");
        assert_eq!(report.groups[0].hidden_counts, [1]);
        assert_eq!(report.groups[0].average_isk_per_lp(2), Some(7.5));
        assert_eq!(report.arrangement(), &narrow_spreads(2));
    }

    #[test]
    fn stores_differing_only_in_a_failing_offer_are_one_group() {
        let stores = vec![
            store_of(
                &[10],
                vec![
                    spread(1, 9.0, 5.0),
                    spread(2, 8.0, 80.0),
                    spread(3, 7.0, 5.0),
                ],
            ),
            store_of(
                &[20],
                vec![
                    spread(1, 9.0, 5.0),
                    spread(3, 7.0, 5.0),
                    spread(4, 1.0, 5.0),
                ],
            ),
        ];
        let mut report = Report::new(&[10, 20], stores, Vec::new(), 2);
        assert_eq!(grouped(&report), [vec![10], vec![20]]);
        report.arrange(&narrow_spreads(2));
        assert_eq!(grouped(&report), [vec![10, 20]]);
        assert_eq!(listed(&report.groups[0]), [1, 3]);
        assert_eq!(report.groups[0].offer_counts, [3, 3]);
        assert_eq!(report.groups[0].hidden_counts, [1, 0]);
        assert_eq!(report.groups[0].hidden_count_text(), "0–1");
    }

    #[test]
    fn an_excluded_offer_drops_out_like_a_failing_one() {
        let stores = vec![
            holding(&[10], &[(1, 9.0), (2, 8.0), (3, 7.0)]),
            holding(&[20], &[(1, 9.0), (3, 7.0), (4, 1.0)]),
        ];
        let mut report = Report::new(&[10, 20], stores, Vec::new(), 2);
        let mut arrangement = Arrangement::depth(2);
        arrangement.excluded = Arc::new(HashSet::from([2]));
        report.arrange(&arrangement);
        assert_eq!(grouped(&report), [vec![10, 20]]);
        assert_eq!(listed(&report.groups[0]), [1, 3]);
        assert_eq!(report.groups[0].hidden_counts, [1, 0]);
    }

    #[test]
    fn with_hiding_off_filters_and_exclusions_are_ignored() {
        let stores = vec![store_of(
            &[10],
            vec![
                spread(1, 9.0, 80.0),
                spread(2, 8.0, 5.0),
                spread(3, 7.0, 5.0),
            ],
        )];
        let mut report = Report::new(&[10], stores, Vec::new(), 2);
        let mut arrangement = narrow_spreads(2);
        arrangement.excluded = Arc::new(HashSet::from([2]));
        report.arrange(&arrangement);
        assert_eq!(listed(&report.groups[0]), [3]);
        assert_eq!(report.groups[0].hidden_counts, [2]);
        arrangement.hiding = false;
        report.arrange(&arrangement);
        assert_eq!(listed(&report.groups[0]), [1, 2]);
        assert_eq!(report.groups[0].hidden_counts, [0]);
    }

    #[test]
    fn the_hideable_range_counts_what_hiding_would_take_out_whether_it_is_on_or_off() {
        let stores = vec![
            store_of(
                &[10, 30],
                vec![
                    spread(1, 9.0, 5.0),
                    spread(2, 8.0, 80.0),
                    spread(3, 1.0, 90.0),
                ],
            ),
            store_of(&[20], vec![spread(1, 9.0, 5.0), spread(4, 2.0, 60.0)]),
        ];
        let mut report = Report::new(&[10, 20, 30], stores, Vec::new(), 1);
        let mut arrangement = narrow_spreads(1);
        report.arrange(&arrangement);
        let on = report.hideable_range(&report.groups[0]);
        assert_eq!(
            on,
            report.groups[0].hidden_range(),
            "hiding on: the group's own"
        );
        arrangement.hiding = false;
        report.arrange(&arrangement);
        // Hiding off lists offer 1 in every store too, so the group is the same three.
        assert_eq!(report.groups[0].hidden_range(), (0, 0), "nothing is hidden");
        assert_eq!(report.hideable_range(&report.groups[0]), (1, 2));
        assert_eq!(report.hideable_count_text(&report.groups[0]), "1–2");
        assert_eq!(
            report.groups[0].sort_value(GroupSort::BestOffer, 1),
            Some(9.0)
        );
    }

    #[test]
    fn the_hideable_range_counts_an_exclusion_with_the_filter_failures() {
        // Store 10 holds a failing offer (2) and an excluded one (3); store 20 only the excluded one.
        let stores = vec![
            store_of(
                &[10],
                vec![
                    spread(1, 9.0, 5.0),
                    spread(2, 8.0, 80.0),
                    spread(3, 7.0, 5.0),
                ],
            ),
            store_of(&[20], vec![spread(1, 9.0, 5.0), spread(3, 7.0, 5.0)]),
        ];
        let mut report = Report::new(&[10, 20], stores, Vec::new(), 1);
        let mut arrangement = narrow_spreads(1);
        arrangement.excluded = Arc::new(HashSet::from([3]));
        report.arrange(&arrangement);
        assert_eq!(grouped(&report), [vec![10, 20]]);
        assert_eq!(report.groups[0].hidden_counts, [2, 1]);
        assert_eq!(report.hideable_range(&report.groups[0]), (1, 2));
        arrangement.hiding = false;
        report.arrange(&arrangement);
        assert_eq!(report.groups[0].hidden_range(), (0, 0));
        assert_eq!(
            report.hideable_range(&report.groups[0]),
            (1, 2),
            "hiding off still counts the exclusion as what it would hide"
        );
        // The exclusion alone, with no filter, is counted too.
        arrangement.filter = OfferFilter::default();
        report.arrange(&arrangement);
        assert_eq!(report.hideable_range(&report.groups[0]), (1, 1));
    }

    #[test]
    fn hidden_counts_cover_each_corporations_whole_store_in_the_runs_order() {
        // At depth 1 all three list offer 1. The store of 10 and 30 hides two offers below it,
        // the store of 20 hides one.
        let stores = vec![
            store_of(
                &[10, 30],
                vec![
                    spread(1, 9.0, 5.0),
                    spread(2, 8.0, 80.0),
                    spread(3, 1.0, 90.0),
                ],
            ),
            store_of(&[20], vec![spread(1, 9.0, 5.0), spread(4, 2.0, 60.0)]),
        ];
        let mut report = Report::new(&[10, 20, 30], stores, Vec::new(), 1);
        report.arrange(&narrow_spreads(1));
        let group = &report.groups[0];
        assert_eq!(group.corp_ids, [10, 20, 30]);
        assert_eq!(group.offer_counts, [3, 2, 3]);
        assert_eq!(group.hidden_counts, [2, 1, 2]);
        assert_eq!(group.hidden_range(), (1, 2));
        assert_eq!(group.hidden_count_text(), "1–2");

        report.regroup(1);
        assert_eq!(
            report.groups[0].hidden_count_text(),
            "0",
            "one number when equal"
        );
        assert_eq!(
            report.arrangement(),
            &Arrangement::depth(1),
            "regroup resets"
        );
    }

    #[test]
    fn a_missing_figure_fails_a_filter_on_that_figure() {
        let stores = vec![store_of(
            &[10],
            vec![
                offer(1, Some(9.0), Some(5.0), None),
                offer(2, None, Some(5.0), Some((10, 1e6))),
                offer(3, Some(7.0), None, Some((10, 1e6))),
                offer(4, Some(6.0), Some(5.0), Some((10, 1e6))),
            ],
        )];
        let mut report = Report::new(&[10], stores, Vec::new(), 10);
        // Ranked 1, 3, 4, then 2, which has no ISK per LP.
        type Set = fn(&mut OfferFilter);
        let cases: [(&str, Set, [i32; 3]); 4] = [
            ("volume", |filter| filter.min_volume = Some(0), [3, 4, 2]),
            ("profit", |filter| filter.min_profit = Some(0.0), [3, 4, 2]),
            (
                "ISK/LP",
                |filter| filter.min_isk_per_lp = Some(0.0),
                [1, 3, 4],
            ),
            (
                "spread",
                |filter| filter.max_spread = Some(100.0),
                [1, 4, 2],
            ),
        ];
        for (figure, set, expected) in cases {
            let mut arrangement = Arrangement::depth(10);
            set(&mut arrangement.filter);
            report.arrange(&arrangement);
            assert_eq!(listed(&report.groups[0]), expected, "{figure}");
            assert_eq!(report.groups[0].hidden_counts, [1], "{figure}");
        }
    }

    /// Stores whose groups order differently under each sort, and two that tie or lack a value.
    ///
    /// | corp | ISK/LP   | profit ±10%  | average | best | profit |
    /// |------|----------|--------------|---------|------|--------|
    /// | 10   | 10, 1    | 1M, 1M       | 5.5     | 10   | 2M     |
    /// | 20   | 6, 6     | 5M, none     | 6       | 6    | 5M     |
    /// | 30   | 8, 5     | none         | 6.5     | 8    | none   |
    /// | 5    | 3        | 2M           | 3       | 3    | 2M     |
    /// | 25   | none     | none         | none    | none | none   |
    fn sortable() -> Report {
        let with = |offer_id, isk_per_lp, profit: Option<f64>| {
            offer(offer_id, isk_per_lp, None, profit.map(|p| (1, p)))
        };
        let stores = vec![
            store_of(
                &[10],
                vec![
                    with(101, Some(10.0), Some(1e6)),
                    with(102, Some(1.0), Some(1e6)),
                ],
            ),
            store_of(
                &[20],
                vec![with(201, Some(6.0), Some(5e6)), with(202, Some(6.0), None)],
            ),
            store_of(
                &[30],
                vec![with(301, Some(8.0), None), with(302, Some(5.0), None)],
            ),
            store_of(&[5], vec![with(51, Some(3.0), Some(2e6))]),
            store_of(&[25], vec![with(251, None, None)]),
        ];
        Report::new(&[], stores, Vec::new(), 2)
    }

    /// The groups' first corporations, in order.
    fn firsts(report: &Report) -> Vec<i32> {
        report.groups.iter().map(|g| g.corp_ids[0]).collect()
    }

    #[test]
    fn groups_sort_by_each_key_whether_arranged_or_only_resorted() {
        let expected = [
            (GroupSort::AverageIskPerLp, [30, 20, 10, 5, 25]),
            (GroupSort::BestOffer, [10, 30, 20, 5, 25]),
            (GroupSort::ProfitWithin10, [20, 5, 10, 25, 30]),
        ];
        let mut report = sortable();
        for (sort, order) in expected {
            let mut arrangement = Arrangement::depth(2);
            arrangement.sort = sort;
            report.arrange(&arrangement);
            assert_eq!(firsts(&report), order, "arranged by {sort:?}");

            let mut resorted = sortable();
            resorted.sort_groups(sort, false);
            assert_eq!(firsts(&resorted), order, "resorted by {sort:?}");
            assert_eq!(resorted.arrangement().sort, sort);
        }
    }

    #[test]
    fn a_reversed_sort_keeps_groups_without_a_value_last_and_ties_by_first_corporation() {
        let mut report = sortable();
        report.sort_groups(GroupSort::ProfitWithin10, true);
        // 10 and 5 tie at 2M, so 5 goes first in both directions; 25 and 30 have no value.
        assert_eq!(firsts(&report), [5, 10, 20, 25, 30]);
        assert!(report.arrangement().reverse);
        report.sort_groups(GroupSort::AverageIskPerLp, true);
        assert_eq!(firsts(&report), [5, 10, 20, 30, 25]);
    }

    /// An offer of one unit for 100 LP and no ISK, with this ISK per LP and spread, whose reward
    /// sells into `book`, given as (price, volume) best first. Each offer has its own reward type.
    fn redeemable(offer_id: i32, isk_per_lp: f64, spread: f64, book: &[(f64, i32)]) -> OfferResult {
        let mut result = offer(offer_id, Some(isk_per_lp), Some(spread), None);
        result.offer.type_id = offer_id;
        result.offer.quantity = 1;
        result.offer.lp_cost = 100;
        result.buy_orders = book
            .iter()
            .map(|&(price, volume_remain)| crate::expenses::BookOrder {
                price,
                volume_remain,
            })
            .collect();
        result
    }

    #[test]
    fn the_budget_sort_orders_groups_by_the_isk_the_fill_realizes() {
        // With the default 1,000 LP: 10 sells 5 units at 1,000 and runs out of book, 5,000 ISK;
        // 20 sells 10 units at 600 and runs out of budget, 6,000 ISK. 30 has the best ISK per LP,
        // but nobody buys its reward, so it has no fill and goes last.
        let stores = vec![
            store_of(&[10], vec![redeemable(101, 10.0, 5.0, &[(1000.0, 5)])]),
            store_of(&[20], vec![redeemable(201, 6.0, 5.0, &[(600.0, 100)])]),
            store_of(&[30], vec![redeemable(301, 20.0, 5.0, &[])]),
        ];
        let mut report = Report::new(&[], stores, Vec::new(), 10);
        assert_eq!(firsts(&report), [30, 10, 20], "by the average");
        let fill = |corp_id: i32| {
            let group = report.groups.iter().find(|g| g.corp_ids[0] == corp_id);
            group.unwrap().budget
        };
        let ten = fill(10).unwrap();
        assert_eq!(
            (ten.isk, ten.lp_spent, ten.stopped),
            (5000.0, 500, budget::Stop::Book)
        );
        let twenty = fill(20).unwrap();
        assert_eq!(
            (twenty.isk, twenty.lp_spent, twenty.stopped),
            (6000.0, 1000, budget::Stop::Budget)
        );
        assert_eq!(fill(30), None);

        report.sort_groups(GroupSort::BudgetIsk, false);
        assert_eq!(firsts(&report), [20, 10, 30]);
        report.sort_groups(GroupSort::BudgetIsk, true);
        assert_eq!(firsts(&report), [10, 20, 30], "no fill stays last");

        let mut arrangement = Arrangement::depth(10);
        arrangement.sort = GroupSort::BudgetIsk;
        arrangement.lp_budget = Some(400);
        report.arrange(&arrangement);
        // 400 LP buys 4 redemptions of each: 4,000 ISK against 2,400.
        assert_eq!(firsts(&report), [10, 20, 30]);
        arrangement.lp_budget = None;
        report.arrange(&arrangement);
        assert!(report.groups.iter().all(|g| g.budget.is_none()));
        assert_eq!(firsts(&report), [10, 20, 30], "by first corporation");
    }

    #[test]
    fn hidden_offers_take_no_part_in_the_fill() {
        // Offer 1 pays 50 ISK per LP but fails the spread filter; offer 2 pays 2.
        let stores = vec![store_of(
            &[10],
            vec![
                redeemable(1, 50.0, 80.0, &[(5000.0, 100)]),
                redeemable(2, 2.0, 5.0, &[(200.0, 100)]),
            ],
        )];
        let mut report = Report::new(&[10], stores, Vec::new(), 10);
        let all = report.groups[0].budget.unwrap();
        assert_eq!((all.isk, all.redemptions), (50_000.0, 10));
        report.arrange(&narrow_spreads(10));
        let passing = report.groups[0].budget.unwrap();
        assert_eq!((passing.isk, passing.redemptions), (2000.0, 10));
        assert_eq!(passing.isk_per_lp(), Some(2.0));
    }
}

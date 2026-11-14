//! How a report is shown without pricing anything again: which offers count, how many of them a
//! group lists, and the order of the groups.

use super::DEFAULT_DEPTH;
use crate::lpstore::OfferResult;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

/// The LP budget [`Arrangement::depth`] starts with, and so the one a run uses when none is given.
pub const DEFAULT_LP_BUDGET: u64 = 1000;

/// Thresholds on the offer-table columns. An unset threshold passes every offer; a set one fails
/// an offer that lacks the figure it reads.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OfferFilter {
    /// The least ISK per LP, read from `isk_per_lp`.
    pub min_isk_per_lp: Option<f64>,
    /// The widest bid/ask spread, in percent, read from `spread`.
    pub max_spread: Option<f64>,
    /// The least imbalance, from -100 to +100, read from `imbalance`.
    pub min_imbalance: Option<f64>,
    /// The least volume within 10% of the best bid, read from `within_10.0`.
    pub min_volume: Option<i64>,
    /// The least profit within 10% of the best bid, read from `within_10.1`.
    pub min_profit: Option<f64>,
}

impl OfferFilter {
    /// Whether no threshold is set, so every offer passes.
    pub fn is_empty(&self) -> bool {
        self.min_isk_per_lp.is_none()
            && self.max_spread.is_none()
            && self.min_imbalance.is_none()
            && self.min_volume.is_none()
            && self.min_profit.is_none()
    }

    /// Whether `result` meets every threshold that is set.
    pub fn passes(&self, result: &OfferResult) -> bool {
        let volume = result.within_10.map(|(volume, _)| volume);
        let profit = result.within_10.map(|(_, profit)| profit);
        at_least(self.min_isk_per_lp, result.isk_per_lp)
            && at_most(self.max_spread, result.spread)
            && at_least(self.min_imbalance, result.imbalance)
            && at_least(self.min_volume, volume)
            && at_least(self.min_profit, profit)
    }
}

/// Whether `value` is at least `bound`, when there is a bound. A missing value fails a bound.
fn at_least<T: PartialOrd>(bound: Option<T>, value: Option<T>) -> bool {
    bound.is_none_or(|bound| value.is_some_and(|value| value >= bound))
}

/// Whether `value` is at most `bound`, when there is a bound. A missing value fails a bound.
fn at_most<T: PartialOrd>(bound: Option<T>, value: Option<T>) -> bool {
    bound.is_none_or(|bound| value.is_some_and(|value| value <= bound))
}

/// What the groups are ordered by. Each is worked out over the offers a group lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GroupSort {
    /// The average ISK per LP of the listed offers that have a value.
    #[default]
    AverageIskPerLp,
    /// The ISK the listed offers realize when the arrangement's LP budget is spent on them, as
    /// [`budget::fill`](super::budget::fill) works it out.
    BudgetIsk,
    /// The ISK per LP of the first listed offer.
    BestOffer,
    /// The profit within 10% of the best bid, summed over the listed offers.
    ProfitWithin10,
}

impl GroupSort {
    /// Every sort, in the order [`GroupSort::next`] steps through them.
    pub const ALL: [GroupSort; 4] = [
        GroupSort::AverageIskPerLp,
        GroupSort::BudgetIsk,
        GroupSort::BestOffer,
        GroupSort::ProfitWithin10,
    ];

    /// The sort after this one, back to the first after the last.
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|&sort| sort == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The word `--sort` takes for it.
    pub fn name(self) -> &'static str {
        match self {
            GroupSort::AverageIskPerLp => "avg",
            GroupSort::BudgetIsk => "budget",
            GroupSort::BestOffer => "best",
            GroupSort::ProfitWithin10 => "profit",
        }
    }

    /// What it sorts by, in words.
    pub fn label(self) -> &'static str {
        match self {
            GroupSort::AverageIskPerLp => "average ISK/LP",
            GroupSort::BudgetIsk => "ISK for budget",
            GroupSort::BestOffer => "best ISK/LP",
            GroupSort::ProfitWithin10 => "profit ±10%",
        }
    }
}

impl FromStr for GroupSort {
    type Err = String;

    /// Reads a [`GroupSort::name`], ignoring case and surrounding space.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|sort| sort.name().eq_ignore_ascii_case(text))
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|sort| sort.name()).collect();
                format!("unknown sort '{text}'; use {}", names.join(", "))
            })
    }
}

/// Everything that decides which groups a report shows and in what order, short of pricing.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement {
    /// How many offers a group lists, and so how many its stores must share.
    pub depth: usize,
    pub filter: OfferFilter,
    /// Whether offers failing `filter`, and the ones in `excluded`, are left out. When false both
    /// are ignored and every priced offer counts.
    pub hiding: bool,
    /// Offer IDs left out wherever they are held.
    pub excluded: Arc<HashSet<i32>>,
    pub sort: GroupSort,
    /// Whether the groups go worst first. Groups without a value for the sort stay last either way.
    pub reverse: bool,
    /// The LP spent on each group's listed offers to work out [`Group::budget`](super::Group::budget),
    /// or `None` to work out no fill.
    pub lp_budget: Option<u64>,
}

impl Arrangement {
    /// Groups for `depth` over every priced offer, by the average ISK per LP, best first, with
    /// the fill worked out for [`DEFAULT_LP_BUDGET`].
    pub fn depth(depth: usize) -> Self {
        Self {
            depth,
            filter: OfferFilter::default(),
            hiding: true,
            excluded: Arc::default(),
            sort: GroupSort::default(),
            reverse: false,
            lp_budget: Some(DEFAULT_LP_BUDGET),
        }
    }

    /// Whether `result` counts: it passes the filter and is not excluded, or nothing is hidden.
    /// It reads only the offer, never the store holding it, so every store keeps or drops the
    /// same offers.
    pub fn visible(&self, result: &OfferResult) -> bool {
        !self.hiding
            || (self.filter.passes(result) && !self.excluded.contains(&result.offer.offer_id))
    }
}

impl Default for Arrangement {
    fn default() -> Self {
        Self::depth(DEFAULT_DEPTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_sorts_step_through_every_one_and_read_back_from_their_names() {
        let mut sort = GroupSort::default();
        let mut seen = Vec::new();
        for _ in 0..GroupSort::ALL.len() {
            seen.push(sort);
            sort = sort.next();
        }
        assert_eq!(seen, GroupSort::ALL);
        assert_eq!(sort, GroupSort::default(), "back to the first");
        for sort in GroupSort::ALL {
            assert_eq!(sort.name().parse::<GroupSort>(), Ok(sort));
        }
        assert_eq!(
            " Profit ".parse::<GroupSort>(),
            Ok(GroupSort::ProfitWithin10)
        );
        assert_eq!(" BUDGET".parse::<GroupSort>(), Ok(GroupSort::BudgetIsk));
        assert_eq!(
            "spread".parse::<GroupSort>(),
            Err("unknown sort 'spread'; use avg, budget, best, profit".to_string())
        );
    }

    #[test]
    fn an_empty_filter_passes_an_offer_with_no_figures() {
        let filter = OfferFilter::default();
        assert!(filter.is_empty());
        assert!(filter.passes(&OfferResult::default()));
    }

    #[test]
    fn each_threshold_reads_its_own_column_and_includes_its_bound() {
        let result = OfferResult {
            isk_per_lp: Some(1000.0),
            spread: Some(20.0),
            imbalance: Some(-10.0),
            within_10: Some((50, 2e6)),
            ..Default::default()
        };
        let cases = [
            (
                OfferFilter {
                    min_isk_per_lp: Some(1000.0),
                    ..Default::default()
                },
                OfferFilter {
                    min_isk_per_lp: Some(1000.1),
                    ..Default::default()
                },
            ),
            (
                OfferFilter {
                    max_spread: Some(20.0),
                    ..Default::default()
                },
                OfferFilter {
                    max_spread: Some(19.9),
                    ..Default::default()
                },
            ),
            (
                OfferFilter {
                    min_imbalance: Some(-10.0),
                    ..Default::default()
                },
                OfferFilter {
                    min_imbalance: Some(-9.9),
                    ..Default::default()
                },
            ),
            (
                OfferFilter {
                    min_volume: Some(50),
                    ..Default::default()
                },
                OfferFilter {
                    min_volume: Some(51),
                    ..Default::default()
                },
            ),
            (
                OfferFilter {
                    min_profit: Some(2e6),
                    ..Default::default()
                },
                OfferFilter {
                    min_profit: Some(2.1e6),
                    ..Default::default()
                },
            ),
        ];
        for (at_bound, past_bound) in cases {
            assert!(!at_bound.is_empty());
            assert!(at_bound.passes(&result), "{at_bound:?}");
            assert!(!past_bound.passes(&result), "{past_bound:?}");
        }
    }

    #[test]
    fn hiding_off_shows_failing_and_excluded_offers() {
        let mut result = OfferResult::default();
        result.offer.offer_id = 7;
        let mut arrangement = Arrangement::depth(10);
        assert!(arrangement.visible(&result));
        arrangement.excluded = Arc::new(HashSet::from([7]));
        assert!(!arrangement.visible(&result));
        arrangement.excluded = Arc::default();
        arrangement.filter.min_volume = Some(1);
        assert!(!arrangement.visible(&result), "no volume fails");
        arrangement.excluded = Arc::new(HashSet::from([7]));
        arrangement.hiding = false;
        assert!(arrangement.visible(&result));
    }
}

//! The `--lpstores` report: option checks, fetching, pricing, and grouping.

mod options;
mod pipeline;

use crate::lpstore::{OfferError, OfferResult};
use std::time::Duration;

pub use options::{
    Field, FilterSummary, LpStoreOptions, OptionError, Parsed, Resolved, SystemSummary,
};
pub use pipeline::run;

/// The priced offers for each corporation group, best first.
#[derive(Debug)]
pub struct Report {
    pub groups: Vec<Group>,
}

/// Corporations whose loyalty offers are identical, with those offers priced.
#[derive(Debug)]
pub struct Group {
    pub corp_ids: Vec<i32>,
    pub offer_count: usize,
    pub results: Vec<OfferResult>,
    pub errors: Vec<OfferError>,
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

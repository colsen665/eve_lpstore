use super::options::Resolved;
use super::{CacheCounts, Event, FetchSummary, Group, PricingSummary, Report, RunEvent};
use crate::esi::loyalty::LoyaltyOffer;
use crate::esi::{ClientStats, EsiClient};
use crate::lpstore::LpStore;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;

/// Fetches the loyalty offers of every corporation in `resolved`, groups the corporations
/// whose offers are identical, and prices each group.
///
/// Progress goes to `events` as it happens, tagged with `run_id`.
pub async fn run(
    resolved: &Resolved,
    client: Arc<EsiClient>,
    run_id: u64,
    events: UnboundedSender<RunEvent>,
) -> Report {
    let send = |event: Event| {
        let _ = events.send(RunEvent { run: run_id, event });
    };

    // One shared pool bounds the offer fetches, the groups, and their offer subtasks together
    let pool = Arc::new(Semaphore::new(resolved.tasks));

    let corp_count = resolved.corp_ids.len();
    let mut fetches = JoinSet::new();
    let fetch_start = Instant::now();
    let fetch_before = client.status();
    send(Event::FetchStarted {
        corporations: corp_count,
    });
    for (index, &corp_id) in resolved.corp_ids.iter().enumerate() {
        let permit = pool
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");
        let esi = client.clone();
        fetches.spawn(async move {
            let _permit = permit;
            (index, corp_id, esi.loyalty().offers(corp_id).await)
        });
    }
    send(Event::FetchQueued {
        tasks: fetches.len(),
    });

    let mut fetched = Vec::new();
    let mut fetch_errors = 0usize;
    let mut fetch_done = 0usize;
    while let Some(joined) = fetches.join_next().await {
        fetch_done += 1;
        match joined {
            Ok(item) => fetched.push(item),
            Err(e) => {
                fetch_errors += 1;
                send(Event::FetchTaskFailed {
                    error: e.to_string(),
                });
            }
        }
        send(Event::FetchProgress {
            done: fetch_done,
            total: corp_count,
        });
    }
    fetched.sort_by_key(|(index, _, _)| *index);

    // Group corporations whose offers are identical
    let mut groups: Vec<PendingGroup> = Vec::new();
    for (_, corp_id, result) in fetched {
        let offers = match result {
            Ok(offers) => offers,
            Err(e) => {
                fetch_errors += 1;
                send(Event::CorpFetchFailed {
                    corp_id,
                    error: e.to_string(),
                });
                continue;
            }
        };
        let mut key: Vec<i32> = offers.iter().map(|o| o.offer_id).collect();
        key.sort_unstable();
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, corps, _)) => corps.push(corp_id),
            None => groups.push((key, vec![corp_id], offers)),
        }
    }
    let fetch_after = client.status();
    send(Event::FetchFinished(FetchSummary {
        corporations: corp_count,
        elapsed: fetch_start.elapsed(),
        errors: fetch_errors,
        cache: cache_delta(&fetch_before, &fetch_after),
    }));

    pricing_order(&mut groups);
    let group_count = groups.len();
    let mut tasks = JoinSet::new();
    let price_start = Instant::now();
    let price_before = client.status();
    send(Event::PricingStarted {
        groups: group_count,
    });
    for (_, corp_ids, offers) in groups {
        let permit = pool
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");
        let lp = LpStore::new(client.clone(), pool.clone());
        let locations = resolved.locations.clone();
        let region_id = resolved.region_id;
        let offer_count = offers.len();
        tasks.spawn(async move {
            let _permit = permit;
            let (results, errors) = lp.offer_results(offers, region_id, locations).await;
            Group {
                corp_ids,
                offer_count,
                results,
                errors,
            }
        });
    }
    send(Event::PricingQueued { tasks: tasks.len() });

    let mut priced = Vec::new();
    let mut group_failures = 0usize;
    let mut price_done = 0usize;
    while let Some(joined) = tasks.join_next().await {
        price_done += 1;
        match joined {
            Ok(group) => priced.push(group),
            Err(e) => {
                group_failures += 1;
                send(Event::GroupFailed {
                    error: e.to_string(),
                });
            }
        }
        send(Event::PricingProgress {
            done: price_done,
            total: group_count,
        });
    }
    let offer_errors: usize = priced.iter().map(|group| group.errors.len()).sum();
    let price_after = client.status();
    send(Event::PricingFinished(PricingSummary {
        groups: group_count,
        elapsed: price_start.elapsed(),
        offer_errors,
        group_failures,
        cache: cache_delta(&price_before, &price_after),
    }));

    let mut report = Report { groups: priced };
    report.sort_by_depth(resolved.depth);
    report
}

/// A group waiting to be priced: its offer ids, sorted, which is what makes two corporations one
/// group; its corporations; and its offers.
type PendingGroup = (Vec<i32>, Vec<i32>, Vec<LoyaltyOffer>);

/// Puts the groups with the most offers first, so the longest lists are started early instead of
/// being the last work left running. Ties go to the lower first corporation ID, so the order is
/// the same on every run. The report is sorted by ISK per LP afterwards, so this order is not seen.
fn pricing_order(groups: &mut [PendingGroup]) {
    groups.sort_by(|(_, a_corps, a_offers), (_, b_corps, b_offers)| {
        b_offers
            .len()
            .cmp(&a_offers.len())
            .then_with(|| a_corps[0].cmp(&b_corps[0]))
    });
}

fn cache_delta(before: &ClientStats, after: &ClientStats) -> CacheCounts {
    CacheCounts {
        hits: after.cache_hits.saturating_sub(before.cache_hits),
        misses: after.cache_misses.saturating_sub(before.cache_misses),
        writes: after.cache_writes.saturating_sub(before.cache_writes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(first_corp: i32, offers: usize) -> PendingGroup {
        let offers = (0..offers)
            .map(|offer_id| LoyaltyOffer {
                offer_id: offer_id as i32,
                ..Default::default()
            })
            .collect();
        (Vec::new(), vec![first_corp], offers)
    }

    #[test]
    fn the_groups_with_the_most_offers_are_priced_first() {
        let mut groups = vec![
            pending(30, 3),
            pending(20, 120),
            pending(40, 40),
            pending(10, 120),
        ];
        pricing_order(&mut groups);
        let order: Vec<(i32, usize)> = groups
            .iter()
            .map(|(_, corps, offers)| (corps[0], offers.len()))
            .collect();
        assert_eq!(order, [(10, 120), (20, 120), (40, 40), (30, 3)]);
    }
}

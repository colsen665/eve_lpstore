use super::options::Resolved;
use super::{CacheCounts, Event, FetchSummary, PricingSummary, Report, RunEvent, Store};
use crate::esi::loyalty::LoyaltyOffer;
use crate::esi::{ClientStats, EsiClient};
use crate::expenses::{ItemPrices, PricingError};
use crate::lpstore::{LpStore, OfferError, OfferResult};
use crate::tasks::TaskPool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinSet;

/// Fetches the loyalty offers of every corporation in `resolved`, puts the corporations whose
/// offers are identical in one store, prices each distinct offer once, and groups the stores that
/// list the same top `resolved.depth` offers.
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

    // One shared pool bounds the store fetches, the offers, and their lookups together
    let pool = TaskPool::new(resolved.tasks);

    let corp_count = resolved.corp_ids.len();
    let mut fetches = JoinSet::new();
    let fetch_start = Instant::now();
    let fetch_before = client.status();
    send(Event::FetchStarted {
        corporations: corp_count,
    });
    for (index, &corp_id) in resolved.corp_ids.iter().enumerate() {
        let esi = client.clone();
        pool.spawn(&mut fetches, move |_slot| async move {
            (index, corp_id, esi.loyalty().offers(corp_id).await)
        })
        .await;
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

    // Corporations whose offers are identical share one store
    let mut stores: Vec<PendingStore> = Vec::new();
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
        match stores.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, corps, _)) => corps.push(corp_id),
            None => stores.push((key, vec![corp_id], offers)),
        }
    }
    let fetch_after = client.status();
    send(Event::FetchFinished(FetchSummary {
        corporations: corp_count,
        elapsed: fetch_start.elapsed(),
        errors: fetch_errors,
        cache: cache_delta(&fetch_before, &fetch_after),
    }));

    let offers = distinct_offers(&stores);
    let offer_count = offers.len();
    let prices = Arc::new(ItemPrices::new(
        resolved.region_id,
        resolved.locations.clone(),
    ));
    let mut tasks = JoinSet::new();
    let mut task_offers = HashMap::new();
    let price_start = Instant::now();
    let price_before = client.status();
    send(Event::PricingStarted {
        offers: offer_count,
    });
    for offer in offers {
        let offer_id = offer.offer_id;
        let lp = LpStore::new(client.clone(), prices.clone());
        let locations = resolved.locations.clone();
        let region_id = resolved.region_id;
        let task = pool
            .spawn(&mut tasks, move |slot| async move {
                lp.price(&slot, offer, region_id, locations).await
            })
            .await;
        task_offers.insert(task, offer_id);
    }
    send(Event::PricingQueued { tasks: tasks.len() });

    let mut priced: HashMap<i32, Arc<OfferResult>> = HashMap::new();
    let mut errors = Vec::new();
    let mut price_done = 0usize;
    while let Some(joined) = tasks.join_next_with_id().await {
        price_done += 1;
        match joined {
            Ok((_, Ok(result))) => {
                priced.insert(result.offer.offer_id, Arc::new(result));
            }
            Ok((_, Err(error))) => errors.push(error),
            Err(failure) => errors.push(OfferError {
                offer_id: task_offers[&failure.id()],
                error: PricingError::TaskFailed(failure.to_string()),
            }),
        }
        send(Event::PricingProgress {
            done: price_done,
            total: offer_count,
        });
    }
    errors.sort_by_key(|error| error.offer_id);
    let price_after = client.status();
    send(Event::PricingFinished(PricingSummary {
        offers: offer_count,
        elapsed: price_start.elapsed(),
        offer_errors: errors.len(),
        cache: cache_delta(&price_before, &price_after),
    }));

    let stores: Vec<Store> = stores
        .into_iter()
        .map(|(offer_ids, corp_ids, _)| {
            let results = offer_ids
                .iter()
                .filter_map(|offer_id| priced.get(offer_id).cloned())
                .collect();
            Store::new(corp_ids, offer_ids, results)
        })
        .collect();
    Report::new(&resolved.corp_ids, stores, errors, resolved.depth)
}

/// A store waiting for its offers to be priced: its offer ids, sorted, which is what makes two
/// corporations one store; its corporations; and its offers.
type PendingStore = (Vec<i32>, Vec<i32>, Vec<LoyaltyOffer>);

/// Every offer the stores hold, each once, in the order the stores first list them. Pricing
/// depends only on the offer, the region, and the stations, so an offer many stores hold is still
/// priced once.
fn distinct_offers(stores: &[PendingStore]) -> Vec<LoyaltyOffer> {
    let mut seen = HashSet::new();
    stores
        .iter()
        .flat_map(|(_, _, offers)| offers)
        .filter(|offer| seen.insert(offer.offer_id))
        .cloned()
        .collect()
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

    fn store(first_corp: i32, offer_ids: &[i32]) -> PendingStore {
        let offers = offer_ids
            .iter()
            .map(|&offer_id| LoyaltyOffer {
                offer_id,
                ..Default::default()
            })
            .collect();
        (Vec::new(), vec![first_corp], offers)
    }

    #[test]
    fn an_offer_several_stores_hold_is_priced_once() {
        let stores = vec![
            store(10, &[3, 1, 2]),
            store(20, &[2, 4]),
            store(30, &[1, 4, 5]),
        ];
        let ids: Vec<i32> = distinct_offers(&stores)
            .iter()
            .map(|offer| offer.offer_id)
            .collect();
        assert_eq!(ids, [3, 1, 2, 4, 5], "each once, in the order first listed");
    }
}

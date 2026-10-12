use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// The `--tasks` budget: how many fetch and pricing tasks run at once. Not the ESI client's cap
/// on requests in flight, which `--max-inflight` sets.
///
/// Only the code that owns the pool can wait for one of its slots, through [`TaskPool::spawn`].
/// The work started that way is given a [`Slot`], which can split it into children but cannot
/// wait for a slot. Waiting below the top level could leave every slot held by a parent waiting
/// for a slot for its children, with nothing left to free one. The pool is not `Clone`, and the
/// tasks it starts are `'static`, so they cannot borrow it.
pub(crate) struct TaskPool {
    slots: Arc<Semaphore>,
}

impl TaskPool {
    pub(crate) fn new(tasks: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(tasks)),
        }
    }

    /// Waits for a free slot, then spawns `work` on `set`. The task holds the slot until `work`
    /// finishes, whether or not `work` keeps the [`Slot`] it is given.
    pub(crate) async fn spawn<T, F, Fut>(&self, set: &mut JoinSet<T>, work: F)
    where
        T: Send + 'static,
        F: FnOnce(Slot) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("the pool is never closed");
        let work = work(self.slot());
        set.spawn(async move {
            let _permit = permit;
            work.await
        });
    }

    fn slot(&self) -> Slot {
        Slot {
            slots: self.slots.clone(),
        }
    }
}

/// Given to work that runs on one of a [`TaskPool`]'s slots, its own or, for a child run inline,
/// its parent's. Only the pool makes one. The slot itself is held by the task that runs the work,
/// so dropping this does not free it.
pub(crate) struct Slot {
    slots: Arc<Semaphore>,
}

impl Slot {
    /// Runs `run` on each input and returns the outputs in input order.
    ///
    /// The caller runs inputs itself, on its own slot, one at a time. Before each one, every slot
    /// that is free at that moment goes to an input still waiting, which then runs as its own
    /// task. So a slot that other work frees partway through is put to use at the caller's next
    /// input, instead of the caller working through the rest of the list alone. Nothing here waits
    /// for a slot, so a parent can always make progress.
    pub(crate) async fn run_children<I, O, F, Fut>(&self, inputs: Vec<I>, run: F) -> Vec<O>
    where
        I: Send + 'static,
        O: Send + 'static,
        F: Fn(Slot, I) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = O> + Send + 'static,
    {
        let count = inputs.len();
        let mut outputs: Vec<Option<O>> = std::iter::repeat_with(|| None).take(count).collect();
        let mut spawned = JoinSet::new();
        let mut waiting: VecDeque<(usize, I)> = inputs.into_iter().enumerate().collect();

        while let Some((index, input)) = waiting.pop_front() {
            while !waiting.is_empty() {
                let Ok(permit) = self.slots.clone().try_acquire_owned() else {
                    break;
                };
                let Some((index, input)) = waiting.pop_front() else {
                    break;
                };
                let run = run.clone();
                let slot = self.child();
                spawned.spawn(async move {
                    let _permit = permit;
                    (index, run(slot, input).await)
                });
            }
            outputs[index] = Some(run(self.child(), input).await);
        }

        while let Some(joined) = spawned.join_next().await {
            let (index, output) = joined.expect("subtask panicked");
            outputs[index] = Some(output);
        }

        outputs
            .into_iter()
            .map(|output| output.expect("every subtask produces an output"))
            .collect()
    }

    fn child(&self) -> Slot {
        Slot {
            slots: self.slots.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Counts how many inputs are running at once, and the most there ever were.
    #[derive(Default)]
    struct Running {
        now: AtomicUsize,
        most: AtomicUsize,
    }

    impl Running {
        async fn during(&self, pause: Duration) {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(pause).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
        }

        fn most(&self) -> usize {
            self.most.load(Ordering::SeqCst)
        }
    }

    type Work = std::pin::Pin<Box<dyn Future<Output = usize> + Send>>;

    fn work(running: &Arc<Running>) -> impl Fn(Slot, usize) -> Work + Clone + Send + 'static {
        let running = running.clone();
        move |_slot, input| {
            let running = running.clone();
            Box::pin(async move {
                running.during(Duration::from_millis(20)).await;
                input * 10
            })
        }
    }

    /// Runs `inputs` as the children of one task the pool started, as a pricing group does.
    async fn children_of_a_task(
        pool: &TaskPool,
        inputs: Vec<usize>,
        running: &Arc<Running>,
    ) -> Vec<usize> {
        let mut set = JoinSet::new();
        let work = work(running);
        pool.spawn(&mut set, move |slot| async move {
            slot.run_children(inputs, work).await
        })
        .await;
        set.join_next().await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn with_no_free_slot_every_input_runs_in_turn_on_the_callers() {
        let pool = TaskPool::new(1);
        let running = Arc::new(Running::default());
        let outputs = children_of_a_task(&pool, (0..5).collect(), &running).await;
        assert_eq!(outputs, [0, 10, 20, 30, 40], "in input order");
        assert_eq!(running.most(), 1);
    }

    #[tokio::test]
    async fn free_slots_at_the_start_go_to_the_waiting_inputs() {
        let pool = TaskPool::new(4);
        let running = Arc::new(Running::default());
        let outputs = children_of_a_task(&pool, (0..8).collect(), &running).await;
        assert_eq!(outputs, (0..8).map(|n| n * 10).collect::<Vec<_>>());
        assert_eq!(running.most(), 4, "the caller and three spawned");
    }

    #[tokio::test]
    async fn slots_freed_partway_through_are_put_to_use() {
        // Every slot but the caller's is held elsewhere when the list starts, which is the state
        // a group starts in while the pricing loop holds the others. They are freed after a few
        // inputs have run. Taking spawn-or-inline decisions only at the start left the caller to
        // run all twelve alone.
        let pool = TaskPool::new(4);
        let elsewhere = pool.slots.clone().acquire_many_owned(3).await.unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(elsewhere);
        });
        let running = Arc::new(Running::default());
        let outputs = children_of_a_task(&pool, (0..12).collect(), &running).await;
        assert_eq!(outputs, (0..12).map(|n| n * 10).collect::<Vec<_>>());
        assert_eq!(running.most(), 4, "the freed slots were taken up");
    }

    #[tokio::test]
    async fn a_task_keeps_its_slot_when_it_drops_the_slot_handle() {
        // `move |_slot| async move { .. }` drops the handle before the work runs. The task must
        // still hold its slot, so with one slot the second task waits for the first.
        let pool = TaskPool::new(1);
        let running = Arc::new(Running::default());
        let mut set = JoinSet::new();
        for _ in 0..2 {
            let running = running.clone();
            pool.spawn(&mut set, move |_slot| async move {
                running.during(Duration::from_millis(30)).await;
            })
            .await;
        }
        while set.join_next().await.is_some() {}
        assert_eq!(running.most(), 1);
    }

    #[tokio::test]
    async fn nested_children_stay_within_the_pool() {
        // Groups, their offers, and each offer's lookups: three levels sharing four slots.
        let pool = TaskPool::new(4);
        let running = Arc::new(Running::default());
        let mut set = JoinSet::new();
        for group in 0..3usize {
            let work = work(&running);
            pool.spawn(&mut set, move |slot| async move {
                let offers: Vec<usize> = (0..4).map(|offer| group * 100 + offer * 10).collect();
                slot.run_children(offers, move |slot, offer| {
                    let work = work.clone();
                    async move {
                        let lookups: Vec<usize> = (0..3).map(|lookup| offer + lookup).collect();
                        slot.run_children(lookups, work)
                            .await
                            .into_iter()
                            .sum::<usize>()
                    }
                })
                .await
            })
            .await;
        }
        let mut totals = Vec::new();
        while let Some(joined) = set.join_next().await {
            totals.push(joined.unwrap());
        }
        totals.sort_unstable();
        // Each lookup returns ten times its number; offer o's three sum to 30o + 30.
        let expected = |group: usize| -> Vec<usize> {
            (0..4)
                .map(|offer| 30 * (group * 100 + offer * 10) + 30)
                .collect()
        };
        assert_eq!(totals, (0..3).map(expected).collect::<Vec<_>>());
        assert!(
            running.most() <= 4,
            "at most one running input per slot: {}",
            running.most()
        );
    }
}

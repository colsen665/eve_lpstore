use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Runs `run` on each input and returns the outputs in input order.
///
/// The caller already holds a permit from `pool`, and runs inputs itself on that permit, one at a
/// time. Before each one, every permit that is free at that moment goes to an input still
/// waiting, which then runs as its own task. So a permit that other work releases partway through
/// is put to use at the caller's next input, instead of the caller working through the rest of the
/// list alone. No task ever waits for a permit, so a parent can always make progress.
pub(crate) async fn run_children<I, O, F, Fut>(
    pool: &Arc<Semaphore>,
    inputs: Vec<I>,
    run: F,
) -> Vec<O>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = O> + Send + 'static,
{
    let count = inputs.len();
    let mut outputs: Vec<Option<O>> = std::iter::repeat_with(|| None).take(count).collect();
    let mut spawned = JoinSet::new();
    let mut waiting: VecDeque<(usize, I)> = inputs.into_iter().enumerate().collect();

    while let Some((index, input)) = waiting.pop_front() {
        while !waiting.is_empty() {
            let Ok(permit) = Arc::clone(pool).try_acquire_owned() else {
                break;
            };
            let Some((index, input)) = waiting.pop_front() else {
                break;
            };
            let run = run.clone();
            spawned.spawn(async move {
                let _permit = permit;
                (index, run(input).await)
            });
        }
        outputs[index] = Some(run(input).await);
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
    }

    fn work(
        running: &Arc<Running>,
    ) -> impl Fn(usize) -> std::pin::Pin<Box<dyn Future<Output = usize> + Send>> + Clone + Send + 'static
    {
        let running = running.clone();
        move |input| {
            let running = running.clone();
            Box::pin(async move {
                running.during(Duration::from_millis(20)).await;
                input * 10
            })
        }
    }

    #[tokio::test]
    async fn with_no_free_permit_every_input_runs_in_turn_on_the_callers() {
        let pool = Arc::new(Semaphore::new(1));
        let _callers = pool.clone().acquire_owned().await.unwrap();
        let running = Arc::new(Running::default());
        let outputs = run_children(&pool, (0..5).collect(), work(&running)).await;
        assert_eq!(outputs, [0, 10, 20, 30, 40], "in input order");
        assert_eq!(running.most.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn free_permits_at_the_start_go_to_the_waiting_inputs() {
        let pool = Arc::new(Semaphore::new(4));
        let _callers = pool.clone().acquire_owned().await.unwrap();
        let running = Arc::new(Running::default());
        let outputs = run_children(&pool, (0..8).collect(), work(&running)).await;
        assert_eq!(outputs, (0..8).map(|n| n * 10).collect::<Vec<_>>());
        assert_eq!(
            running.most.load(Ordering::SeqCst),
            4,
            "the caller and three spawned"
        );
    }

    #[tokio::test]
    async fn permits_released_partway_through_are_put_to_use() {
        // Every permit but the caller's is held elsewhere when the list starts, which is the state
        // a group starts in while the pricing loop holds the others. They are released after a
        // few inputs have run. Taking spawn-or-inline decisions only at the start left the caller
        // to run all twelve alone.
        let pool = Arc::new(Semaphore::new(4));
        let _callers = pool.clone().acquire_owned().await.unwrap();
        let elsewhere = pool.clone().acquire_many_owned(3).await.unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(elsewhere);
        });
        let running = Arc::new(Running::default());
        let outputs = run_children(&pool, (0..12).collect(), work(&running)).await;
        assert_eq!(outputs, (0..12).map(|n| n * 10).collect::<Vec<_>>());
        assert_eq!(
            running.most.load(Ordering::SeqCst),
            4,
            "the released permits were taken up"
        );
    }
}

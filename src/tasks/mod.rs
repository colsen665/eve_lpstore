use std::future::Future;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Runs `run` on each input and returns the outputs in input order.
///
/// The caller already holds a permit from `pool`. The first input runs inline on that
/// permit. Each further input is spawned only if a free permit can be taken without
/// waiting; otherwise it runs inline after the spawns. No task ever waits for a permit,
/// so a parent can always make progress.
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
    let mut inline = Vec::new();

    for (index, input) in inputs.into_iter().enumerate() {
        if index == 0 {
            inline.push((index, input));
            continue;
        }
        match Arc::clone(pool).try_acquire_owned() {
            Ok(permit) => {
                let run = run.clone();
                spawned.spawn(async move {
                    let _permit = permit;
                    (index, run(input).await)
                });
            }
            Err(_) => inline.push((index, input)),
        }
    }

    for (index, input) in inline {
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

//! The `--tui` view of `--lpstores`. The pipeline runs as a task on the async runtime, and the
//! terminal is drawn on a blocking thread that reads its progress from a channel.

mod app;
mod ui;

use crate::output;
use app::{Action, App, LogLine};
use crossterm::event::{self, Event as TermEvent};
use eve_lpstore::StaticData;
use eve_lpstore::esi::EsiClient;
use eve_lpstore::report::{self, LpStoreOptions, Report, Resolved, RunEvent};
use ratatui::DefaultTerminal;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

const TICK: Duration = Duration::from_millis(250);

/// Runs one `--lpstores` run in the terminal until the user quits.
pub async fn run(
    sdata: Arc<StaticData>,
    options: LpStoreOptions,
    resolved: Resolved,
) -> io::Result<()> {
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
    let client = Arc::new(resolved.client(0, events.clone()));
    let mut app = App::new(
        resolved.region_label.clone(),
        resolved.corp_ids.len(),
        filter_summary(&options),
        resolved.depth,
        resolved.agent_filter,
    );

    let (done_tx, done_rx) = oneshot::channel();
    let pipeline = tokio::spawn({
        let client = client.clone();
        async move {
            let report = report::run(&resolved, client, 0, events).await;
            let _ = done_tx.send(report);
        }
    });

    let ui_client = client.clone();
    let mut done = Some(done_rx);
    let ui = tokio::task::spawn_blocking(move || {
        ratatui::run(|terminal| {
            ui_loop(
                terminal,
                &mut app,
                &mut received,
                &mut done,
                &ui_client,
                &sdata,
            )
        })
    });
    let result = ui.await.map_err(io::Error::other)?;
    pipeline.abort();
    result
}

fn ui_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    received: &mut UnboundedReceiver<RunEvent>,
    done: &mut Option<oneshot::Receiver<Report>>,
    client: &EsiClient,
    sdata: &StaticData,
) -> io::Result<()> {
    loop {
        while let Ok(run_event) = received.try_recv() {
            let line = output::message(&run_event.event, sdata).map(|message| LogLine {
                text: message.text,
                is_error: message.is_error,
            });
            app.on_event(&run_event.event, line);
        }
        if let Some(receiver) = done.as_mut() {
            match receiver.try_recv() {
                Ok(report) => {
                    app.finish(report);
                    *done = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    app.fail();
                    *done = None;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }

        let stats = client.status();
        let mut layout = (0, 0);
        terminal.draw(|frame| layout = ui::draw(frame, app, sdata, &stats))?;
        app.set_detail_layout(layout.0, layout.1);

        if event::poll(TICK)? {
            let quit = match event::read()? {
                TermEvent::Key(key) => app.on_key(key) == Action::Quit,
                _ => false,
            };
            if quit {
                return Ok(());
            }
        }
    }
}

/// The filters the run was started with, written the way they are on the command line.
fn filter_summary(options: &LpStoreOptions) -> String {
    let filters = [
        ("--lpstore", &options.lpstore),
        ("--security", &options.security),
        ("--agent-level", &options.agent_level),
        ("--agent-division", &options.agent_division),
        ("--agent-type", &options.agent_type),
        ("--faction", &options.faction),
        ("--system", &options.system),
    ];
    let set: Vec<String> = filters
        .iter()
        .filter_map(|(flag, value)| value.as_ref().map(|value| format!("{} {}", flag, value)))
        .collect();
    if set.is_empty() {
        "none".to_string()
    } else {
        set.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_summary_lists_only_the_filters_that_are_set() {
        assert_eq!(filter_summary(&LpStoreOptions::default()), "none");
        let options = LpStoreOptions {
            region: Some("Domain".to_string()),
            security: Some("+0.5".to_string()),
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        assert_eq!(
            filter_summary(&options),
            "--security +0.5 --faction caldari"
        );
    }
}

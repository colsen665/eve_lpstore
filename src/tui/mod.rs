//! The `--tui` view of `--lpstores`. The pipeline runs as a task on the async runtime,
//! and the terminal is drawn on a blocking thread that reads its progress from a channel.

use crate::output;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eve_lpstore::StaticData;
use eve_lpstore::esi::EsiClient;
use eve_lpstore::report::{self, Event, LpStoreOptions, Report, Resolved, RunEvent};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;

const LOG_LIMIT: usize = 500;
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
            let line = output::message(&run_event.event, sdata).map(|line| line.text);
            app.on_event(&run_event.event, line);
        }
        if let Some(receiver) = done.as_mut() {
            match receiver.try_recv() {
                Ok(report) => {
                    app.finish(report.groups.len());
                    *done = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    app.fail();
                    *done = None;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }

        let status = output::status_text(&client.status());
        terminal.draw(|frame| draw(frame, app, &status))?;

        if event::poll(TICK)? {
            let read = event::read()?;
            if matches!(&read, TermEvent::Key(key) if is_quit(key)) {
                return Ok(());
            }
        }
    }
}

/// Whether a key press asks to quit: `q`, or Ctrl+C, which raw mode delivers as a key event.
fn is_quit(key: &KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }
    match key.code {
        KeyCode::Char('q') => key.modifiers.is_empty(),
        KeyCode::Char('c') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Starting,
    Fetching { done: usize, total: usize },
    Pricing { done: usize, total: usize },
    Finished { groups: usize },
    Failed,
}

struct App {
    region: String,
    corporations: usize,
    filters: String,
    stage: Stage,
    log: VecDeque<String>,
}

impl App {
    fn new(region: String, corporations: usize, filters: String) -> Self {
        Self {
            region,
            corporations,
            filters,
            stage: Stage::Starting,
            log: VecDeque::new(),
        }
    }

    /// Moves the stage forward for `event`, and keeps its text in the log if it has any.
    fn on_event(&mut self, event: &Event, line: Option<String>) {
        match event {
            Event::FetchStarted { corporations } => {
                self.stage = Stage::Fetching {
                    done: 0,
                    total: *corporations,
                };
            }
            Event::FetchProgress { done, total } => {
                self.stage = Stage::Fetching {
                    done: *done,
                    total: *total,
                };
            }
            Event::PricingStarted { groups } => {
                self.stage = Stage::Pricing {
                    done: 0,
                    total: *groups,
                };
            }
            Event::PricingProgress { done, total } => {
                self.stage = Stage::Pricing {
                    done: *done,
                    total: *total,
                };
            }
            _ => {}
        }
        if let Some(line) = line {
            self.push_log(line);
        }
    }

    fn finish(&mut self, groups: usize) {
        self.stage = Stage::Finished { groups };
    }

    fn fail(&mut self) {
        self.stage = Stage::Failed;
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() == LOG_LIMIT {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn stage_text(&self) -> String {
        match self.stage {
            Stage::Starting => "Starting...".to_string(),
            Stage::Fetching { done, total } => {
                format!("Fetching loyalty offers: {} / {} corporations", done, total)
            }
            Stage::Pricing { done, total } => format!("Pricing groups: {} / {}", done, total),
            Stage::Finished { groups } => format!("Finished: {} groups priced", groups),
            Stage::Failed => "The run stopped unexpectedly".to_string(),
        }
    }
}

fn draw(frame: &mut Frame, app: &App, status: &str) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    let summary = vec![
        Line::from(format!("Region: {}", app.region)),
        Line::from(format!("Corporations: {}", app.corporations)),
        Line::from(format!("Filters: {}", app.filters)),
    ];
    frame.render_widget(
        Paragraph::new(summary).block(Block::bordered().title(" LP store ")),
        header,
    );

    // The box has two border rows, and the stage line and a blank line take two more.
    let room = usize::from(body.height).saturating_sub(4);
    let mut lines = vec![Line::from(app.stage_text()), Line::from("")];
    let skip = app.log.len().saturating_sub(room);
    lines.extend(
        app.log
            .iter()
            .skip(skip)
            .map(|line| Line::from(line.as_str())),
    );
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Run ")),
        body,
    );

    let footer_lines = vec![
        Line::from(status.to_string()),
        Line::from("q or Ctrl+C to quit"),
    ];
    frame.render_widget(Paragraph::new(footer_lines), footer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q_and_ctrl_c_quit_on_press_only() {
        assert!(is_quit(&KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        )));
        assert!(is_quit(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_quit(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
        assert!(!is_quit(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!is_quit(&KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
    }

    #[test]
    fn progress_events_move_the_stage_forward() {
        let mut app = App::new("The Forge".to_string(), 3, "none".to_string());
        assert_eq!(app.stage, Stage::Starting);
        app.on_event(&Event::FetchStarted { corporations: 3 }, None);
        assert_eq!(app.stage, Stage::Fetching { done: 0, total: 3 });
        app.on_event(&Event::FetchProgress { done: 2, total: 3 }, None);
        assert_eq!(app.stage, Stage::Fetching { done: 2, total: 3 });
        app.on_event(&Event::PricingStarted { groups: 2 }, None);
        assert_eq!(app.stage, Stage::Pricing { done: 0, total: 2 });
        app.finish(2);
        assert_eq!(app.stage, Stage::Finished { groups: 2 });
    }

    #[test]
    fn a_failed_run_says_so() {
        let mut app = App::new("The Forge".to_string(), 3, "none".to_string());
        app.fail();
        assert_eq!(app.stage, Stage::Failed);
        assert_eq!(app.stage_text(), "The run stopped unexpectedly");
    }

    #[test]
    fn log_keeps_only_the_newest_lines() {
        let mut app = App::new("The Forge".to_string(), 3, "none".to_string());
        for i in 0..LOG_LIMIT + 10 {
            app.push_log(format!("line {}", i));
        }
        assert_eq!(app.log.len(), LOG_LIMIT);
        assert_eq!(app.log.front().map(String::as_str), Some("line 10"));
    }

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

//! The `--tui` view of `--lpstores`. The UI runs on a blocking thread. Each run of the
//! pipeline is a task on the async runtime, and applying new filters replaces the current run.

mod app;
mod form;
mod qr;
mod ui;

use crate::output;
use app::{Action, App, LogLine, RunInfo};
use crossterm::event::{self, Event as TermEvent};
use eve_lpstore::StaticData;
use eve_lpstore::esi::EsiClient;
use eve_lpstore::export;
use eve_lpstore::report::{self, LpStoreOptions, Report, Resolved, RunEvent};
use ratatui::DefaultTerminal;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const TICK: Duration = Duration::from_millis(250);

/// One run of the pipeline: its id, its client, the task running it, and the channel that
/// receives its report.
struct Run {
    id: u64,
    client: Arc<EsiClient>,
    pipeline: JoinHandle<()>,
    done: Option<oneshot::Receiver<Report>>,
}

/// What the UI thread needs to start runs.
struct Session {
    handle: Handle,
    sdata: Arc<StaticData>,
    events: UnboundedSender<RunEvent>,
    next_id: u64,
    current: Run,
}

/// Starts `resolved` as a run with `id` on the runtime. Each run has its own client, so its
/// cache offset and status counters are its own.
fn spawn_run(
    handle: &Handle,
    events: &UnboundedSender<RunEvent>,
    resolved: Resolved,
    id: u64,
) -> Run {
    let client = Arc::new(resolved.client(id, events.clone()));
    let (done_tx, done_rx) = oneshot::channel();
    let task_client = client.clone();
    let task_events = events.clone();
    let pipeline = handle.spawn(async move {
        let report = report::run(&resolved, task_client, id, task_events).await;
        let _ = done_tx.send(report);
    });
    Run {
        id,
        client,
        pipeline,
        done: Some(done_rx),
    }
}

/// Runs `resolved` in the terminal until the user quits.
pub async fn run(
    sdata: Arc<StaticData>,
    options: LpStoreOptions,
    resolved: Resolved,
) -> io::Result<()> {
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
    let handle = Handle::current();
    let info = run_info(&resolved, filter_summary(&options));
    let mut app = App::new(info, options);
    let current = spawn_run(&handle, &events, resolved, 0);
    let mut session = Session {
        handle,
        sdata,
        events,
        next_id: 1,
        current,
    };
    let ui = tokio::task::spawn_blocking(move || {
        let result =
            ratatui::run(|terminal| ui_loop(terminal, &mut app, &mut received, &mut session));
        session.current.pipeline.abort();
        result
    });
    match ui.await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(error)),
    }
}

fn ui_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    received: &mut UnboundedReceiver<RunEvent>,
    session: &mut Session,
) -> io::Result<()> {
    loop {
        while let Ok(run_event) = received.try_recv() {
            if run_event.run != session.current.id {
                continue;
            }
            let line = output::message(&run_event.event, &session.sdata).map(|message| LogLine {
                text: message.text,
                is_error: message.is_error,
            });
            app.on_event(&run_event.event, line);
        }
        if let Some(receiver) = session.current.done.as_mut() {
            match receiver.try_recv() {
                Ok(report) => {
                    app.finish(report);
                    session.current.done = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    app.fail();
                    session.current.done = None;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }

        let stats = session.current.client.status();
        let mut layout = (0, 0);
        terminal.draw(|frame| layout = ui::draw(frame, app, &session.sdata, &stats))?;
        app.set_detail_layout(layout.0, layout.1);

        if event::poll(TICK)? {
            let action = match event::read()? {
                TermEvent::Key(key) => app.on_key(key),
                _ => Action::Continue,
            };
            match action {
                Action::Continue => {}
                Action::Quit => return Ok(()),
                Action::Apply => apply(app, session),
                Action::Export => write_export(app, &session.sdata),
            }
        }
    }
}

/// Checks every field of the form. If they are all valid, a change to the depth only
/// re-renders, and any other change replaces the current run with one using the new filters.
/// Otherwise the errors are shown beside their fields and nothing runs.
fn apply(app: &mut App, session: &mut Session) {
    let options = app.form.to_options(&app.applied);
    let resolved = match options.resolve(&session.sdata) {
        Ok(resolved) => resolved,
        Err(errors) => {
            app.form.set_errors(errors);
            return;
        }
    };
    if same_run(&options, &app.applied) {
        app.apply_depth(resolved.depth);
    } else {
        let info = run_info(&resolved, filter_summary(&options));
        let id = session.next_id;
        session.next_id += 1;
        let next = spawn_run(&session.handle, &session.events, resolved, id);
        let previous = std::mem::replace(&mut session.current, next);
        previous.pipeline.abort();
        app.start_run(info);
    }
    app.set_applied(options);
    app.close_form();
}

/// Writes the report on screen to the file the prompt names. Success closes the prompt and says so
/// in the log; a failure leaves the prompt open with the reason beside the name, so it can be
/// corrected and tried again. Split from the loop so the write can be tested without a terminal.
fn write_export(app: &mut App, sdata: &StaticData) {
    let Some(request) = app.export_request() else {
        return;
    };
    let Some(report) = app.report.as_ref() else {
        return;
    };
    let context = export::ExportContext {
        region: &app.region,
        filters: &app.filters,
        depth: app.depth,
    };
    let offers = export::offer_count(report, app.depth);
    let text = match request.format {
        export::ExportFormat::Csv => export::csv(report, &context, sdata),
        export::ExportFormat::Json => export::json(report, &context, sdata),
    };
    match std::fs::write(&request.path, text) {
        Ok(()) => {
            app.push_log(LogLine {
                text: format!("Wrote {} offers to {}", offers, request.path),
                is_error: false,
            });
            app.close_export();
        }
        Err(error) => app.refuse_export(format!("Could not write {}: {}", request.path, error)),
    }
}

fn run_info(resolved: &Resolved, filters: String) -> RunInfo {
    RunInfo {
        region: resolved.region_label.clone(),
        corporations: resolved.corp_ids.len(),
        filters,
        depth: resolved.depth,
        agent_filter: resolved.agent_filter,
    }
}

/// Whether two sets of options start the same run. Depth only changes what is shown.
fn same_run(a: &LpStoreOptions, b: &LpStoreOptions) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.depth = None;
    b.depth = None;
    a == b
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
    use eve_lpstore::esi::loyalty::LoyaltyOffer;
    use eve_lpstore::report::Group;
    use eve_lpstore::static_data::AgentFilter;
    use eve_lpstore::{ExpenseBreakdown, OfferResult};

    use crate::tui::app::Overlay;

    /// An app on the results view with the export prompt open and `path` typed into it.
    fn prompt(path: &str) -> App {
        let info = RunInfo {
            region: "The Forge".to_string(),
            corporations: 1,
            filters: "none".to_string(),
            depth: 10,
            agent_filter: AgentFilter::default(),
        };
        let mut app = App::new(info, LpStoreOptions::default());
        app.finish(Report {
            groups: vec![Group {
                corp_ids: vec![1000002],
                offer_count: 1,
                results: vec![OfferResult {
                    offer: LoyaltyOffer {
                        offer_id: 1,
                        type_id: 30486,
                        quantity: 10,
                        lp_cost: 1800,
                        isk_cost: 1_200_000.0,
                        required_items: Vec::new(),
                    },
                    breakdown: ExpenseBreakdown {
                        offer_id: 1,
                        lp_cost: 1800,
                        isk_cost: 1_200_000.0,
                        item_expenses: Vec::new(),
                        total_isk: 1_649_000.0,
                    },
                    reward: Some(457_900.0),
                    unit_cost: Some(164_900.0),
                    isk_per_lp: Some(1627.78),
                    best_ask: Some(515_000.0),
                    spread: Some(12.5),
                    book_depth: Some((2_474_600, 800_000)),
                    imbalance: Some(51.2),
                    within_10: Some((2_474_600, 3_980_000_000.0)),
                }],
                errors: Vec::new(),
            }],
        });
        app.open_export();
        app.export_path = path.to_string();
        app
    }

    /// A file base name in the system temp directory, distinct per test so they cannot collide.
    /// The caller adds the extension.
    fn temp_path(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("eve_lpstore_{}_{}", std::process::id(), name))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn an_export_writes_the_view_and_reports_it_in_the_log() {
        let path = format!("{}.csv", temp_path("csv"));
        let mut app = prompt(&path);
        write_export(&mut app, &StaticData::default());
        let written = std::fs::read_to_string(&app.export_path).unwrap();
        assert!(written.starts_with("group,average_isk_per_lp,"));
        assert!(written.contains("30486"), "the offer row is there");
        assert!(written.contains("1627.78"));
        assert_eq!(app.overlay, Overlay::None, "success closes the prompt");
        let line = app.log.back().expect("the write is reported");
        assert!(!line.is_error);
        assert!(line.text.contains("Wrote 1 offers"), "{}", line.text);
        std::fs::remove_file(&app.export_path).unwrap();
    }

    #[test]
    fn a_json_name_writes_json() {
        let path = format!("{}.json", temp_path("json"));
        let mut app = prompt(&path);
        write_export(&mut app, &StaticData::default());
        let written = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["region"], "The Forge");
        assert_eq!(parsed["groups"][0]["offers"][0]["spread_percent"], 12.5);
        assert_eq!(app.overlay, Overlay::None);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_write_that_fails_keeps_the_prompt_open_with_the_reason() {
        // A directory that does not exist, so the write cannot succeed.
        let mut app = prompt("/no/such/directory/eve_lpstore_out.csv");
        write_export(&mut app, &StaticData::default());
        assert_eq!(
            app.overlay,
            Overlay::Export,
            "the name stays editable so it can be retried"
        );
        let error = app.export_error.expect("the reason shows beside the name");
        assert!(error.contains("Could not write"), "{}", error);
        assert!(error.contains("no/such/directory"), "{}", error);
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

    #[test]
    fn a_depth_change_alone_is_the_same_run() {
        let base = LpStoreOptions::default();
        let deeper = LpStoreOptions {
            depth: Some("25".to_string()),
            ..Default::default()
        };
        let other_region = LpStoreOptions {
            region: Some("Domain".to_string()),
            ..Default::default()
        };
        assert!(same_run(&deeper, &base));
        assert!(!same_run(&other_region, &base));
    }
}

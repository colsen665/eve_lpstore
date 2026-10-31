//! The `--tui` view of `--lpstores`. The UI runs on a blocking thread. Each run of the
//! pipeline is a task on the async runtime, and applying new filters replaces the current run.

mod app;
mod form;
mod qr;
#[cfg(test)]
mod test_support;
mod ui;

use crate::output;
use app::{Action, App, LogLine, RunInfo};
use crossterm::event::{self, Event as TermEvent};
use eve_lpstore::StaticData;
use eve_lpstore::esi::EsiClient;
use eve_lpstore::export;
use eve_lpstore::paths::Paths;
use eve_lpstore::report::{self, Field, LpStoreOptions, OptionError, Report, Resolved, RunEvent};
use eve_lpstore::saved_filters;
use ratatui::DefaultTerminal;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::{JoinError, JoinHandle};

const TICK: Duration = Duration::from_millis(250);

/// One run of the pipeline: its id, its client, and the task running it, which returns the
/// report. The task is taken when its report is read, so it is read once.
struct Run {
    id: u64,
    client: Arc<EsiClient>,
    pipeline: Option<JoinHandle<Report>>,
}

/// A static data download in progress, which returns the data or why it could not be loaded.
type Download = JoinHandle<Result<StaticData, String>>;

/// What the UI thread needs to start runs. `current` is `None` until the first run is applied, so
/// the view can open on the filters dialog without having fetched anything.
struct Session {
    handle: Handle,
    sdata: StaticData,
    paths: Paths,
    events: UnboundedSender<RunEvent>,
    next_id: u64,
    current: Option<Run>,
    download: Option<Download>,
}

impl Session {
    /// The status of the run in progress, or zeros before the first one. The footer reads this
    /// every frame, including when nothing has been started.
    fn stats(&self) -> eve_lpstore::esi::ClientStats {
        match &self.current {
            Some(run) => run.client.status(),
            None => Default::default(),
        }
    }
}

/// Starts `resolved` as a run with `id` on the runtime. Each run has its own client, so its
/// cache offset and status counters are its own.
fn spawn_run(
    handle: &Handle,
    events: &UnboundedSender<RunEvent>,
    cache: &Path,
    resolved: Resolved,
    id: u64,
) -> Run {
    let client = Arc::new(resolved.client(cache, id, events.clone()));
    let task_client = client.clone();
    let task_events = events.clone();
    let pipeline =
        handle.spawn(async move { report::run(&resolved, task_client, id, task_events).await });
    Run {
        id,
        client,
        pipeline: Some(pipeline),
    }
}

/// Fetches the static data archive on the runtime, replacing whatever is on disk. It belongs here
/// rather than on the UI thread because `StaticData::download` is async and `ui_loop` is sync inside
/// `spawn_blocking`.
fn spawn_download(session: &mut Session) {
    let data = session.paths.data.clone();
    let task = session.handle.spawn(async move {
        StaticData::download(&data)
            .await
            .map_err(|error| error.to_string())
    });
    session.download = Some(task);
}

/// What `task` returned, once it has finished, taking it so it is read only once. `None` while it
/// is still running, or after it was taken. `Err` when the task panicked or was aborted.
///
/// The UI loop is not async, so a finished task's output is collected with `block_on`, which
/// returns at once because the task is done. That is allowed here because the loop runs on a
/// blocking thread, outside any async context.
fn take_finished<T>(
    handle: &Handle,
    task: &mut Option<JoinHandle<T>>,
) -> Option<Result<T, JoinError>> {
    if !task.as_ref()?.is_finished() {
        return None;
    }
    let finished = task.take()?;
    Some(handle.block_on(finished))
}

/// The region name the default region id stands for, or the id itself when the static data does not
/// name it. Used to pre-fill the filters form, so a user sees which region is about to be priced in
/// rather than a blank field that means The Forge.
fn default_region(sdata: &StaticData) -> String {
    sdata
        .region_name(report::DEFAULT_REGION_ID)
        .cloned()
        .unwrap_or_else(|| report::DEFAULT_REGION_ID.to_string())
}

/// The filters form as the startup dialog shows it.
fn startup_form(app: &App, sdata: &StaticData) -> form::FilterForm {
    form::FilterForm::from_options_with_defaults(
        &app.applied,
        &default_region(sdata),
        report::DEFAULT_DEPTH,
    )
}

/// What the view does about the static data before it draws anything.
enum Opening {
    /// The data is on disk and read, so the filters can be asked for. Boxed because the data is
    /// far larger than the other case, and this is built once.
    Filters(Box<StaticData>),
    /// The data is not usable, so the user is asked whether to fetch it. `reason` says why when
    /// there is something specific to say, and is `None` when the files are simply not there.
    Ask { reason: Option<String> },
}

/// Decides how the view starts, from what loading the files returned, or `None` when they are not
/// on disk and loading was not tried. A missing cache is a question and never a silent download:
/// `load` fetches the whole static data archive, which takes long enough that it has to be asked
/// for rather than discovered.
fn opening(loaded: Option<Result<StaticData, String>>) -> Opening {
    match loaded {
        None => Opening::Ask { reason: None },
        Some(Ok(data)) => Opening::Filters(Box::new(data)),
        // The files are there and cannot be read, so a download is the only way past it.
        Some(Err(reason)) => Opening::Ask {
            reason: Some(reason),
        },
    }
}

/// Runs the view until the user quits. Static data is loaded here when it is on disk, and asked
/// for in the view when it is not, so a first run does not silently fetch the whole archive.
pub async fn run(cli: LpStoreOptions, paths: Paths) -> io::Result<()> {
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel::<RunEvent>();
    let handle = Handle::current();
    // The filters the last applied run used fill in whatever the command line left alone, so the
    // dialog opens on something the user recognises instead of on the defaults every time. A field
    // the command line did set keeps its own value, which is what makes the flags still work.
    let remembered = saved_filters::load(&paths.config);
    // One message, shown on the form and kept in the log, that names the consequence as well as the
    // cause: the saved filters were not used.
    let ignored = remembered
        .reason()
        .map(|reason| format!("Ignoring the saved filters: {}", reason));
    let mut app = App::new(remembered.merge(cli));
    if let Some(note) = &ignored {
        // The run still starts from the command line alone, so this is worth saying but not fatal.
        app.push_log(LogLine {
            text: note.clone(),
            is_error: true,
        });
    }

    // The roots are absolute now and shared between runs, so the view says where it writes. The log
    // is the only place a TUI user can read this, since stdout becomes the screen.
    app.set_data_dir(paths.data.display().to_string());
    log_roots(&mut app, &paths);

    // Only what is on disk is read here, so a missing cache is asked about before any download.
    let loaded = StaticData::load_cached(&paths.data)
        .map_err(|error| error.to_string())
        .transpose();
    let (sdata, ask) = match opening(loaded) {
        Opening::Filters(data) => (*data, None),
        Opening::Ask { reason } => (StaticData::default(), Some(reason)),
    };
    match ask {
        None => {
            let mut form = startup_form(&app, &sdata);
            // The form owns the keyboard and the log panel cannot be opened over it, so a file that
            // could not be read has to be said here or the user never learns their filters were
            // dropped.
            form.set_note(ignored);
            app.show_filters(form);
        }
        Some(reason) => app.open_static_data(reason),
    }

    let mut session = Session {
        handle,
        sdata,
        events,
        next_id: 0,
        current: None,
        download: None,
        paths,
    };
    // The loop has to stay on a blocking thread. It blocks while it waits for keys, so on a runtime
    // thread it would hold up the pipeline's tasks, and with --single-thread stop them entirely;
    // and `take_finished` uses `Handle::block_on`, which panics inside an async context.
    let ui = tokio::task::spawn_blocking(move || {
        let result =
            ratatui::run(|terminal| ui_loop(terminal, &mut app, &mut received, &mut session));
        if let Some(pipeline) = session.current.take().and_then(|run| run.pipeline) {
            pipeline.abort();
        }
        if let Some(download) = session.download.take() {
            download.abort();
        }
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
            let current_id = match &session.current {
                Some(run) => run.id,
                // Nothing has been started, so there is no run these events could belong to.
                None => continue,
            };
            if run_event.run != current_id {
                continue;
            }
            let line = output::message(&run_event.event, &session.sdata).map(|message| LogLine {
                text: message.text,
                is_error: message.is_error,
            });
            app.on_event(&run_event.event, line);
        }
        if let Some(run) = session.current.as_mut()
            && let Some(finished) = take_finished(&session.handle, &mut run.pipeline)
        {
            match finished {
                Ok(report) => app.finish(report),
                // The pipeline panicked. A replaced run is aborted, but it is no longer current.
                Err(_) => app.fail(),
            }
        }
        if let Some(finished) = take_finished(&session.handle, &mut session.download) {
            match finished {
                Ok(Ok(data)) => {
                    session.sdata = data;
                    let form = startup_form(app, &session.sdata);
                    app.download_finished(form);
                }
                Ok(Err(reason)) => app.download_failed(reason),
                Err(_) => app.download_failed("the download was stopped".to_string()),
            }
        }

        let stats = session.stats();
        let mut layout = ui::PaneLayout::default();
        terminal.draw(|frame| layout = ui::draw(frame, app, &session.sdata, &stats))?;
        app.set_detail_layout(layout.detail_rows, layout.detail_visible);
        app.set_destinations_layout(layout.destinations_rows, layout.destinations_visible);
        app.set_help_layout(layout.help_rows, layout.help_visible);

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
                Action::Download => {
                    app.start_download();
                    spawn_download(session);
                }
            }
        }
    }
}

/// Says where the two roots are, as the first log lines of the session. The view cannot print these
/// to stdout, because that becomes the screen, and the log panel cannot be opened over the startup
/// dialog, so they are the first thing readable once a run is applied.
fn log_roots(app: &mut App, paths: &Paths) {
    for (label, root) in [("Cache", &paths.cache), ("Static data", &paths.data)] {
        app.push_log(LogLine {
            text: format!("{label}: {}", root.display()),
            is_error: false,
        });
    }
}

/// The two region and system pairs the form holds: where prices come from, and where agents
/// stand. Applying checks both the same way, so each step below is written once and asked of
/// either pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pair {
    Market,
    Agent,
}

impl Pair {
    fn system_field(self) -> Field {
        match self {
            Pair::Market => Field::System,
            Pair::Agent => Field::AgentSystem,
        }
    }

    /// The region this pair named in the run in progress. The market pair always names one; the
    /// agent pair only when the run filtered agents by a region.
    fn previous_region(self, run: &RunInfo) -> Option<i32> {
        match self {
            Pair::Market => Some(run.region_id),
            Pair::Agent => run.agent_region_id,
        }
    }

    fn region(self, resolved: &Resolved) -> Option<i32> {
        match self {
            Pair::Market => Some(resolved.region_id),
            Pair::Agent => resolved.agent_region.as_ref().map(|found| found.id),
        }
    }

    fn region_label(self, resolved: &Resolved) -> Option<&str> {
        match self {
            Pair::Market => Some(&resolved.region_label),
            Pair::Agent => resolved
                .agent_region
                .as_ref()
                .map(|found| found.label.as_str()),
        }
    }

    /// The system as typed, and the id it resolved to.
    fn system(self, resolved: &Resolved) -> Option<(&str, i32)> {
        match self {
            Pair::Market => resolved
                .system
                .as_ref()
                .map(|system| (system.value.as_str(), system.system_id)),
            Pair::Agent => resolved
                .agent_system
                .as_ref()
                .map(|system| (system.value.as_str(), system.id)),
        }
    }

    fn cleared_message(self) -> &'static str {
        match self {
            Pair::Market => "Cleared the market system, which belonged to the previous region",
            Pair::Agent => "Cleared the agent system, which belonged to the previous agent region",
        }
    }
}

/// Whether applying `resolved` moves `pair`'s region away from the one the run in progress used,
/// as (from, to).
///
/// Compared on ids rather than the typed text, so `Domain` and `10000043` are the same region and
/// the system field survives. There is no run to compare against before the first Enter. Both
/// sides have to name a region: blanking the agent region lifts that filter rather than moving it,
/// and a system left over under no region filter is not a mistake.
fn region_moved(pair: Pair, run: Option<&RunInfo>, resolved: &Resolved) -> Option<(i32, i32)> {
    let previous = pair.previous_region(run?)?;
    let now = pair.region(resolved)?;
    (previous != now).then_some((previous, now))
}

/// Clears `pair`'s system when its region moved out from under it, and returns `false` if the
/// form has to show errors instead of running.
///
/// A market system prices from the NPC stations in it, so one left over from the previous region
/// matches no orders there and every lookup fails. An agent system from the previous agent region
/// can still be a real system, but the pair then keeps nothing. Either way the field the user did
/// not touch is the one that has to go. The second resolve is what makes the cleared field real:
/// the station list comes from that result, so keeping the first would run against the old
/// system's stations while the form showed an empty box.
fn clear_system_on_region_move(
    pair: Pair,
    app: &mut App,
    options: &mut LpStoreOptions,
    resolved: &mut Resolved,
    sdata: &StaticData,
) -> bool {
    if region_moved(pair, app.run.as_ref(), resolved).is_none() || pair.system(resolved).is_none() {
        return true;
    }
    *options.value_mut(pair.system_field()) = None;
    app.form.set_value(pair.system_field(), "");
    match options.resolve(sdata) {
        Ok(next) => {
            *resolved = next;
            app.push_log(LogLine {
                text: pair.cleared_message().to_string(),
                is_error: false,
            });
            true
        }
        // Clearing a filter cannot introduce an error, so this only keeps the compiler honest.
        Err(errors) => {
            app.form.set_errors(errors);
            false
        }
    }
}

/// Whether the system the run prices from is inside the region it prices in, as an error to show on
/// the field, or `None` when there is no problem or nothing to check.
///
/// Takes the system's region rather than the static data, so the rule is testable on its own. An
/// unknown region is not reported: `resolve` only accepts systems the static data names, so a
/// missing answer means the mapping is not there, as it is on a cache from an older build, and
/// complaining would stop a run for something the user did nothing wrong about.
///
/// Only the dialog asks. `resolve` is shared with `--lpstores`, and the command line keeps its
/// current behaviour of letting a mismatched pair through, which prices against nothing.
fn system_outside_region(
    system: Option<&str>,
    region_id: i32,
    region_label: &str,
    region_of_system: Option<i32>,
    field: Field,
) -> Option<OptionError> {
    let system = system?;
    let actual = region_of_system?;
    (actual != region_id).then(|| OptionError {
        field,
        message: format!(
            "'{}' is not in {}. Clear the system, or choose a system in it",
            system, region_label
        ),
    })
}

/// [`system_outside_region`] for one of the pairs of `resolved`. The error lands on the system
/// field, because the system is the value that cannot be reached from the region named.
fn pair_outside_region(pair: Pair, resolved: &Resolved, sdata: &StaticData) -> Option<OptionError> {
    let (system, system_id) = pair.system(resolved)?;
    system_outside_region(
        Some(system),
        pair.region(resolved)?,
        pair.region_label(resolved)?,
        sdata.region_of_system(system_id),
        pair.system_field(),
    )
}

/// Checks every field of the form. If they are all valid, a change to the depth only
/// re-renders, and any other change replaces the current run with one using the new filters.
/// Otherwise the errors are shown beside their fields and nothing runs.
fn apply(app: &mut App, session: &mut Session) {
    let mut options = app.form.to_options(&app.applied);
    let mut resolved = match options.resolve(&session.sdata) {
        Ok(resolved) => resolved,
        Err(errors) => {
            app.form.set_errors(errors);
            return;
        }
    };
    // Each pair's leftover system is cleared before the pair is checked, so a system the region
    // move already removed is not also reported: the user sees one change, not a complaint about
    // a value they no longer have.
    for pair in [Pair::Market, Pair::Agent] {
        if !clear_system_on_region_move(pair, app, &mut options, &mut resolved, &session.sdata) {
            return;
        }
        if let Some(error) = pair_outside_region(pair, &resolved, &session.sdata) {
            app.form.set_errors(vec![error]);
            return;
        }
    }
    if is_repeat(app.run.is_some(), &options, &app.applied) {
        app.apply_depth(resolved.depth);
    } else {
        let info = run_info(&resolved);
        let depth = resolved.depth;
        let id = session.next_id;
        session.next_id += 1;
        let next = spawn_run(
            &session.handle,
            &session.events,
            &session.paths.cache,
            resolved,
            id,
        );
        if let Some(pipeline) = session.current.replace(next).and_then(|run| run.pipeline) {
            pipeline.abort();
        }
        app.start_run(info, filter_summary(&options), depth);
    }
    app.applied = options;
    app.close_form();
    remember(app, &saved_filters::path(&session.paths.config));
}

/// Keeps the filters this run was applied with, so the next session's dialog starts from them. Both
/// branches reach it, including a depth-only apply, because "applied" is what the user pressed.
///
/// A failure is only worth a log line. The run is already going, and being unable to remember it is
/// not a reason to stop the view. Split from the loop and given its path so the write can be tested
/// without a terminal, the same way `write_export` is.
fn remember(app: &mut App, file: &Path) {
    if let Err(error) = saved_filters::save_to(file, &app.applied) {
        app.push_log(LogLine {
            text: format!("Could not save the filters: {}", error),
            is_error: true,
        });
    }
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
        region: app.region(),
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

fn run_info(resolved: &Resolved) -> RunInfo {
    RunInfo {
        region: resolved.region_label.clone(),
        region_id: resolved.region_id,
        agent_region_id: resolved.agent_region.as_ref().map(|found| found.id),
        corporations: resolved.corp_ids.len(),
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

/// Whether applying `options` only changes what is already on screen. The first Enter is never a
/// repeat, even when the form holds exactly the options the view started with, because nothing has
/// been fetched yet: matching on `same_run` alone would take the depth-only branch and leave the
/// view with no run at all.
fn is_repeat(has_run: bool, options: &LpStoreOptions, applied: &LpStoreOptions) -> bool {
    has_run && same_run(options, applied)
}

/// The filters the run was started with, written the way they are on the command line, in the
/// form's order. The market region has its own place in the header, and depth and the cache
/// minutes change no corporation or price, so those three are left out.
fn filter_summary(options: &LpStoreOptions) -> String {
    let set: Vec<String> = Field::ALL
        .into_iter()
        .filter(|field| field.in_form())
        .filter(|field| !matches!(field, Field::Region | Field::Depth | Field::CacheMinutes))
        .filter_map(|field| {
            let value = options.value(field).as_ref()?;
            Some(format!("{} {}", field.flag(), value))
        })
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
    use crate::tui::app::{Overlay, Stage};
    use crate::tui::test_support::{
        forge_run, open_prompt, open_prompt_mut, priced_offer, results_app,
    };
    use eve_lpstore::static_data::AgentFilter;
    use std::path::PathBuf;

    /// An app on the results view with the export prompt open and `path` typed into it.
    fn prompt(path: &str) -> App {
        let offer = priced_offer(Some(12.5), Some(51.2), Some((2_474_600, 3_980_000_000.0)));
        let mut app = results_app(vec![offer], 10);
        app.open_export();
        open_prompt_mut(&mut app).path = path.to_string();
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

    /// A path no write can succeed at, whatever the platform or user: its parent is a regular
    /// file. A directory that does not exist is not enough, since an administrator can create one.
    /// Returns the path and the file to remove afterwards.
    fn unwritable_path(name: &str) -> (PathBuf, PathBuf) {
        let blocker = PathBuf::from(temp_path(name));
        std::fs::write(&blocker, "").unwrap();
        (blocker.join("out.csv"), blocker)
    }

    #[test]
    fn an_export_writes_the_view_and_reports_it_in_the_log() {
        let path = format!("{}.csv", temp_path("csv"));
        let mut app = prompt(&path);
        write_export(&mut app, &StaticData::default());
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("group,average_isk_per_lp,"));
        assert!(written.contains("30486"), "the offer row is there");
        assert!(written.contains("1627.78"));
        assert_eq!(app.overlay, Overlay::None, "success closes the prompt");
        let line = app.log.back().expect("the write is reported");
        assert!(!line.is_error);
        assert!(line.text.contains("Wrote 1 offers"), "{}", line.text);
        std::fs::remove_file(&path).unwrap();
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
        let (path, blocker) = unwritable_path("blocked_export");
        let path = path.to_string_lossy().into_owned();
        let mut app = prompt(&path);
        write_export(&mut app, &StaticData::default());
        assert!(
            matches!(app.overlay, Overlay::Export(_)),
            "the name stays editable so it can be retried"
        );
        let error = open_prompt(&app)
            .error
            .clone()
            .expect("the reason shows beside the name");
        assert!(error.contains("Could not write"), "{}", error);
        assert!(error.contains(&path), "{}", error);
        std::fs::remove_file(&blocker).unwrap();
    }

    #[test]
    fn the_place_filters_appear_in_the_header_summary() {
        let options = LpStoreOptions {
            security: Some("0.5..0.7".to_string()),
            agent_region: Some("Black Rise".to_string()),
            agent_system: Some("4-3L53".to_string()),
            ..Default::default()
        };
        assert_eq!(
            filter_summary(&options),
            "--security 0.5..0.7 --agent-region Black Rise --agent-system 4-3L53",
            "the two sit beside the other agent filters, in the order the form shows them"
        );
    }

    #[test]
    fn a_mismatched_agent_pair_is_reported_on_the_agent_system() {
        // The same rule as the market pair, landing on the field that cannot be reached from the
        // region named: 4-3L53 is in Black Rise, so filtering agents by The Forge is the mistake.
        let error = system_outside_region(
            Some("4-3L53"),
            10000002,
            "The Forge (10000002)",
            Some(10000014),
            Field::AgentSystem,
        )
        .expect("a mismatch is reported");
        assert_eq!(error.field, Field::AgentSystem);
        assert!(
            error.message.contains("'4-3L53' is not in The Forge"),
            "{}",
            error.message
        );
        assert!(
            system_outside_region(
                Some("4-3L53"),
                10000014,
                "Black Rise (10000014)",
                Some(10000014),
                Field::AgentSystem
            )
            .is_none(),
            "a pair that agrees is not a mistake"
        );
    }

    #[test]
    fn an_agent_region_only_moves_when_both_sides_name_one() {
        let in_forge = eve_lpstore::report::NamedValue {
            value: "The Forge".to_string(),
            id: 10000002,
            label: "The Forge (10000002)".to_string(),
        };
        let in_domain = eve_lpstore::report::NamedValue {
            value: "Domain".to_string(),
            id: 10000043,
            label: "Domain (10000043)".to_string(),
        };
        let unfiltered = forge_run(1);
        let in_the_forge = RunInfo {
            agent_region_id: Some(10000002),
            ..forge_run(1)
        };
        let mut moved = resolved_fixture();
        assert_eq!(
            region_moved(Pair::Agent, Some(&unfiltered), &moved),
            None,
            "no previous region"
        );
        moved.agent_region = Some(in_forge.clone());
        assert_eq!(
            region_moved(Pair::Agent, Some(&unfiltered), &moved),
            None,
            "blanking the field lifts the filter rather than moving it"
        );
        assert_eq!(
            region_moved(Pair::Agent, Some(&in_the_forge), &moved),
            None,
            "the same region typed differently has not moved"
        );
        moved.agent_region = Some(in_domain.clone());
        assert_eq!(
            region_moved(Pair::Agent, Some(&in_the_forge), &moved),
            Some((10000002, 10000043)),
            "and a different one has"
        );
        assert_eq!(
            region_moved(Pair::Agent, None, &moved),
            None,
            "before the first run there is nothing to have moved"
        );
    }

    #[test]
    fn filter_summary_lists_only_the_filters_that_are_set() {
        assert_eq!(filter_summary(&LpStoreOptions::default()), "none");
        let options = LpStoreOptions {
            region: Some("Domain".to_string()),
            security: Some("0.5..0.7".to_string()),
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        assert_eq!(
            filter_summary(&options),
            "--security 0.5..0.7 --faction caldari"
        );
    }

    #[test]
    fn the_session_opens_by_saying_where_its_files_live() {
        let mut app = App::new(LpStoreOptions::default());
        let paths = Paths {
            cache: PathBuf::from("/home/user/.cache/eve_lpstore"),
            data: PathBuf::from("/home/user/.local/share/eve_lpstore"),
            config: PathBuf::from("/home/user/.config/eve_lpstore"),
        };
        log_roots(&mut app, &paths);
        app.set_data_dir(paths.data.display().to_string());
        let lines: Vec<&str> = app.log.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            lines,
            [
                "Cache: /home/user/.cache/eve_lpstore",
                "Static data: /home/user/.local/share/eve_lpstore",
            ]
        );
        assert!(
            app.log.iter().all(|line| !line.is_error),
            "neither root line is an error"
        );
        assert_eq!(app.data_dir, "/home/user/.local/share/eve_lpstore");
    }

    #[test]
    fn a_finished_task_is_read_once_and_a_panic_reads_as_an_error() {
        // `take_finished` is what replaced the oneshot channels, and it runs outside any async
        // context, as the UI loop does.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let handle = runtime.handle().clone();
        fn wait<T>(task: &Option<JoinHandle<T>>) {
            while !task.as_ref().unwrap().is_finished() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        let mut done = Some(handle.spawn(async { 5 }));
        wait(&done);
        assert!(matches!(take_finished(&handle, &mut done), Some(Ok(5))));
        assert!(done.is_none(), "taken");
        assert!(
            take_finished(&handle, &mut done).is_none(),
            "and read only once"
        );

        let mut panicked = Some(handle.spawn(async {
            if true {
                panic!("a pipeline that panics, on purpose");
            }
        }));
        wait(&panicked);
        assert!(matches!(
            take_finished(&handle, &mut panicked),
            Some(Err(_))
        ));

        let mut running = Some(handle.spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
        assert!(
            take_finished(&handle, &mut running).is_none(),
            "still running"
        );
        assert!(running.is_some(), "and not taken");
        running.unwrap().abort();
    }

    #[test]
    fn a_finished_task_is_read_on_a_current_thread_runtime_too() {
        // As `--single-thread` runs the view: the main thread drives the runtime inside
        // `block_on`, and the view's loop runs on a blocking thread, starting a task and reading
        // it with `take_finished`. The task sleeps, so it needs the timer, which only the main
        // thread's `block_on` drives on this runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let read = runtime.block_on(async {
            let handle = Handle::current();
            tokio::task::spawn_blocking(move || {
                let mut task = Some(handle.spawn(async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    5
                }));
                let give_up = std::time::Instant::now() + Duration::from_secs(5);
                loop {
                    if let Some(read) = take_finished(&handle, &mut task) {
                        return read;
                    }
                    assert!(
                        std::time::Instant::now() < give_up,
                        "the task never ran while the loop waited"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .await
            .unwrap()
        });
        assert!(matches!(read, Ok(5)));
    }

    #[test]
    fn a_missing_cache_asks_instead_of_downloading_silently() {
        // The regression this guards: the uncached branch used to report no reason, which read back
        // as success, so the view opened on the filters and never asked.
        assert!(matches!(opening(None), Opening::Ask { reason: None }));
    }

    #[test]
    fn readable_static_data_goes_straight_to_the_filters() {
        assert!(matches!(
            opening(Some(Ok(StaticData::default()))),
            Opening::Filters(_)
        ));
    }

    #[test]
    fn unreadable_static_data_asks_with_the_reason() {
        let reason = Some(Err("data/regions.json is not valid JSON".to_string()));
        let ask = opening(reason);
        assert!(matches!(ask, Opening::Ask { reason: Some(_) }));
    }

    #[test]
    fn an_applied_run_is_remembered() {
        let file = PathBuf::from(format!("{}.json", temp_path("remember")));
        let mut app = App::new(LpStoreOptions {
            faction: Some("caldari".to_string()),
            region: Some("Domain".to_string()),
            expire: true,
            ..Default::default()
        });
        remember(&mut app, &file);
        let loaded = saved_filters::load_from(&file);
        let saved_filters::Loaded::Filters(filters) = &loaded else {
            panic!("expected the run to be remembered");
        };
        assert_eq!(filters.value(Field::Faction).as_deref(), Some("caldari"));
        assert_eq!(filters.value(Field::Region).as_deref(), Some("Domain"));
        assert!(app.log.back().is_none(), "a save that worked says nothing");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn remembering_never_keeps_expire() {
        // A remembered --expire would refetch every offer on every later session, so the run keeps
        // it and the file does not.
        let file = PathBuf::from(format!("{}.json", temp_path("expire")));
        let mut app = App::new(LpStoreOptions {
            expire: true,
            ..Default::default()
        });
        remember(&mut app, &file);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("expire"), "the file kept it: {}", text);
        assert!(app.applied.expire, "the run still has it");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn a_failed_remember_is_only_a_log_line() {
        // The run is already going, so a file that cannot be written must not stop the view.
        let mut app = App::new(LpStoreOptions::default());
        app.start_run(forge_run(1), "none".to_string(), 10);
        let (path, blocker) = unwritable_path("blocked_remember");
        remember(&mut app, &path);
        assert_eq!(app.stage, Stage::Starting, "the run is untouched");
        let line = app.log.back().expect("the failure is reported");
        assert!(line.is_error);
        assert!(
            line.text.contains("Could not save the filters"),
            "{}",
            line.text
        );
        std::fs::remove_file(&blocker).unwrap();
    }

    #[test]
    fn remembered_filters_fill_the_startup_form() {
        // The dialog is built from `app.applied`, so a merged run has to show what it will actually
        // do rather than what the command line alone asked for.
        let cli = LpStoreOptions {
            region: Some("Sinq Laison".to_string()),
            ..Default::default()
        };
        let remembered = saved_filters::SavedFilters::from(&LpStoreOptions {
            lpstore: Some("Caldari Navy".to_string()),
            depth: Some("20".to_string()),
            ..Default::default()
        });
        let mut app = App::new(remembered.overlay(&cli));
        let form = startup_form(&app, &StaticData::default());
        assert_eq!(form.value(Field::Region), "Sinq Laison", "the flag wins");
        assert_eq!(
            form.value(Field::Lpstore),
            "Caldari Navy",
            "the file fills the rest"
        );
        assert_eq!(form.value(Field::Depth), "20");
        assert_eq!(
            app.filters, "--lpstore Caldari Navy",
            "the remembered filter shows in the header"
        );
        app.show_filters(form);
        assert_eq!(app.overlay, Overlay::Filters);
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

    #[test]
    fn the_first_enter_starts_a_run_even_unchanged() {
        let base = LpStoreOptions::default();
        let deeper = LpStoreOptions {
            depth: Some("25".to_string()),
            ..Default::default()
        };
        let other_region = LpStoreOptions {
            region: Some("Domain".to_string()),
            ..Default::default()
        };
        // The startup form holds the command line options with two defaults filled in, so an
        // unchanged Enter matches `same_run` against `applied`. Without the has_run guard the view
        // would take the depth-only branch and never fetch anything.
        assert!(!is_repeat(false, &base, &base), "nothing has run yet");
        assert!(!is_repeat(false, &deeper, &base));
        assert!(is_repeat(true, &base, &base), "a repeat re-renders");
        assert!(
            is_repeat(true, &deeper, &base),
            "depth only changes what is shown"
        );
        assert!(
            !is_repeat(true, &other_region, &base),
            "a new region is a new run"
        );
    }

    #[test]
    fn a_system_outside_the_priced_region_is_reported_on_its_own_field() {
        let system = eve_lpstore::report::SystemSummary {
            value: "Jita".to_string(),
            system_id: 30000142,
            stations: 1,
        };
        // Jita is in The Forge, so pricing in Domain is the mistake.
        let error = system_outside_region(
            Some(system.value.as_str()),
            10000043,
            "Domain (10000043)",
            Some(10000002),
            Field::System,
        )
        .expect("a mismatch is reported");
        assert_eq!(error.field, Field::System);
        assert!(
            error.message.contains("'Jita' is not in Domain"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("Clear the system"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_system_inside_the_region_and_no_system_both_pass() {
        let system = eve_lpstore::report::SystemSummary {
            value: "Jita".to_string(),
            system_id: 30000142,
            stations: 1,
        };
        assert!(
            system_outside_region(
                Some(system.value.as_str()),
                10000002,
                "The Forge",
                Some(10000002),
                Field::System,
            )
            .is_none(),
            "the same region is not a mismatch"
        );
        assert!(system_outside_region(None, 10000002, "The Forge", None, Field::System).is_none());
    }

    #[test]
    fn an_unknown_system_region_is_not_reported_as_a_mismatch() {
        // A cache from an older build has no mapping, and stopping the run for that would blame the
        // user for a file they did not choose.
        let system = eve_lpstore::report::SystemSummary {
            value: "Jita".to_string(),
            system_id: 30000142,
            stations: 1,
        };
        assert!(
            system_outside_region(
                Some(system.value.as_str()),
                10000043,
                "Domain",
                None,
                Field::System
            )
            .is_none()
        );
    }

    #[test]
    fn a_moved_region_clears_the_leftover_system_and_rebuilds_the_run() {
        let mut app = App::new(LpStoreOptions {
            system: Some("Jita".to_string()),
            ..Default::default()
        });
        app.run = Some(forge_run(1)); // The Forge, where Jita is
        app.form.set_value(Field::System, "Jita");
        let mut options = app.form.to_options(&app.applied);
        // The stations of a system are what `resolve` finds there, so the fixture carries them the
        // way a real resolved run does.
        let stations: std::collections::HashSet<i64> = [60003760].into_iter().collect();
        let mut resolved = Resolved {
            region_id: 10000043,
            system: Some(eve_lpstore::report::SystemSummary {
                value: "Jita".to_string(),
                system_id: 30000142,
                stations: 1,
            }),
            locations: Some(stations.into()),
            ..resolved_fixture()
        };

        let keep_going = clear_system_on_region_move(
            Pair::Market,
            &mut app,
            &mut options,
            &mut resolved,
            &sdata(),
        );

        assert!(keep_going, "clearing a filter cannot fail to resolve");
        assert_eq!(options.system, None, "the run drops the old system");
        assert_eq!(
            app.form.value(Field::System),
            "",
            "the form shows it dropped"
        );
        assert!(
            resolved.system.is_none(),
            "the rebuilt run has no station filter"
        );
        assert!(resolved.locations.is_none(), "and no station list");
        let line = app.log.back().expect("the clearing is logged");
        assert!(
            line.text.contains("Cleared the market system"),
            "{}",
            line.text
        );
        assert!(!line.is_error, "nothing went wrong");
    }

    #[test]
    fn a_system_survives_when_the_region_did_not_move() {
        let mut app = App::new(LpStoreOptions::default());
        app.run = Some(forge_run(1));
        app.form.set_value(Field::System, "Jita");
        let mut options = app.form.to_options(&app.applied);
        let mut resolved = Resolved {
            system: Some(eve_lpstore::report::SystemSummary {
                value: "Jita".to_string(),
                system_id: 30000142,
                stations: 1,
            }),
            ..resolved_fixture()
        };
        let before = resolved.clone();

        assert!(clear_system_on_region_move(
            Pair::Market,
            &mut app,
            &mut options,
            &mut resolved,
            &sdata(),
        ));
        assert_eq!(
            resolved.system.as_ref().map(|s| s.value.clone()),
            before.system.as_ref().map(|s| s.value.clone())
        );
        assert_eq!(options.system.as_deref(), Some("Jita"));
        assert_eq!(app.form.value(Field::System), "Jita");
        assert!(
            app.log.is_empty(),
            "nothing was cleared, so nothing is logged"
        );
    }

    #[test]
    fn the_first_enter_never_clears_because_there_is_no_previous_region() {
        let mut app = App::new(LpStoreOptions::default());
        app.form.set_value(Field::System, "Jita");
        let mut options = app.form.to_options(&app.applied);
        let mut resolved = Resolved {
            region_id: 10000043,
            system: Some(eve_lpstore::report::SystemSummary {
                value: "Jita".to_string(),
                system_id: 30000142,
                stations: 1,
            }),
            ..resolved_fixture()
        };

        assert!(clear_system_on_region_move(
            Pair::Market,
            &mut app,
            &mut options,
            &mut resolved,
            &sdata(),
        ));
        assert_eq!(
            options.system.as_deref(),
            Some("Jita"),
            "kept for the first run"
        );
        assert!(app.log.is_empty());
    }

    /// An empty `StaticData`, which resolves a blank system to none. The clearing path only needs
    /// `system` to be unset there, so an empty cache exercises it.
    fn sdata() -> StaticData {
        StaticData::default()
    }

    #[test]
    fn a_region_change_is_judged_on_the_id_not_the_typed_name() {
        let resolved = |region_id: i32| Resolved {
            region_id,
            ..resolved_fixture()
        };
        let in_domain = RunInfo {
            region_id: 10000043,
            ..forge_run(1)
        };
        assert!(
            region_moved(Pair::Market, None, &resolved(10000043)).is_none(),
            "the first Enter has no previous region"
        );
        assert_eq!(
            region_moved(Pair::Market, Some(&forge_run(1)), &resolved(10000043)),
            Some((10000002, 10000043))
        );
        assert!(
            region_moved(Pair::Market, Some(&in_domain), &resolved(10000043)).is_none(),
            "the same region typed differently has not moved"
        );
    }

    #[test]
    fn each_pair_reads_its_own_region_and_system() {
        let resolved = Resolved {
            system: Some(eve_lpstore::report::SystemSummary {
                value: "Jita".to_string(),
                system_id: 30000142,
                stations: 1,
            }),
            agent_region: Some(eve_lpstore::report::NamedValue {
                value: "Black Rise".to_string(),
                id: 10000069,
                label: "Black Rise (10000069)".to_string(),
            }),
            agent_system: Some(eve_lpstore::report::NamedValue {
                value: "4-3L53".to_string(),
                id: 30002812,
                label: "4-3L53".to_string(),
            }),
            ..resolved_fixture()
        };
        assert_eq!(Pair::Market.region(&resolved), Some(10000002));
        assert_eq!(Pair::Market.region_label(&resolved), Some("The Forge"));
        assert_eq!(Pair::Market.system(&resolved), Some(("Jita", 30000142)));
        assert_eq!(Pair::Market.system_field(), Field::System);
        assert_eq!(Pair::Agent.region(&resolved), Some(10000069));
        assert_eq!(
            Pair::Agent.region_label(&resolved),
            Some("Black Rise (10000069)")
        );
        assert_eq!(Pair::Agent.system(&resolved), Some(("4-3L53", 30002812)));
        assert_eq!(Pair::Agent.system_field(), Field::AgentSystem);
        let unfiltered = resolved_fixture();
        assert_eq!(
            Pair::Agent.region(&unfiltered),
            None,
            "no agent region filter"
        );
        assert!(
            pair_outside_region(Pair::Agent, &unfiltered, &StaticData::default()).is_none(),
            "nothing to check without a pair"
        );
    }

    /// A `Resolved` for the tests above, with only the region id varying.
    fn resolved_fixture() -> Resolved {
        Resolved {
            corp_ids: vec![1000002],
            filter_summary: None,
            agent_filter: AgentFilter::default(),
            agent_region: None,
            agent_system: None,
            region_id: 10000002,
            region_label: "The Forge".to_string(),
            system: None,
            locations: None,
            cache_offset: Duration::from_secs(60),
            tasks: 2,
            max_inflight: 4,
            depth: 10,
            expire: false,
            verbose: false,
        }
    }

    #[test]
    fn the_default_region_falls_back_to_its_id_when_static_data_does_not_name_it() {
        // An empty StaticData is what the view starts on before a download, so the pre-fill has to
        // cope with the region id having no name beside it.
        assert_eq!(default_region(&StaticData::default()), "10000002");
    }

    #[test]
    fn the_startup_form_carries_the_command_line_filters_and_the_defaults() {
        let options = LpStoreOptions {
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        let app = App::new(options);
        let form = startup_form(&app, &StaticData::default());
        assert_eq!(form.value(Field::Faction), "caldari");
        assert_eq!(form.value(Field::Region), "10000002");
        assert_eq!(form.value(Field::Depth), "10");
        assert_eq!(
            form.value(Field::Lpstore),
            "",
            "no filter is invented for the blank fields"
        );
    }
}

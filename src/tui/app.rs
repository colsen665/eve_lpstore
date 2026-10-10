//! The state of the `--tui` view. Run events, key presses, and applied filters go in, and the
//! new state comes out. Nothing here touches the terminal, so these rules can be tested
//! directly.

use super::filter_summary;
use super::form::FilterForm;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eve_lpstore::OfferResult;
use eve_lpstore::export::ExportRequest;
use eve_lpstore::report::{DEFAULT_DEPTH, Event, Group, LpStoreOptions, Report};
use eve_lpstore::static_data::AgentFilter;
use std::collections::VecDeque;

const LOG_LIMIT: usize = 500;
const PAGE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// No run has been started. The startup filters dialog shows this until Enter is pressed.
    Idle,
    Starting,
    Fetching {
        done: usize,
        total: usize,
    },
    Pricing {
        done: usize,
        total: usize,
    },
    Finished {
        groups: usize,
    },
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Offers,
    Details,
}

impl Focus {
    fn next(self) -> Focus {
        match self {
            Focus::Groups => Focus::Offers,
            Focus::Offers => Focus::Details,
            Focus::Details => Focus::Groups,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Log,
    Help,
    Filters,
    Donate,
    Export,
    /// Asking whether to download the static data, shown when it is not on disk.
    StaticData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
    Apply,
    /// Write the report on screen to the file the export prompt names.
    Export,
    /// Download the static data archive after the user chose it in the dialog.
    Download,
}

/// What a run was started with, as the header and the detail pane show it.
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub region: String,
    pub corporations: usize,
    pub filters: String,
    pub depth: usize,
    pub agent_filter: AgentFilter,
}

pub struct App {
    pub region: String,
    pub corporations: usize,
    pub filters: String,
    pub depth: usize,
    pub agent_filter: AgentFilter,
    /// The options the current run was started with.
    pub applied: LpStoreOptions,
    pub form: FilterForm,
    /// Whether the report on screen comes from an earlier run than the one in progress.
    pub stale: bool,
    pub stage: Stage,
    pub log: VecDeque<LogLine>,
    pub report: Option<Report>,
    pub focus: Focus,
    pub group: usize,
    pub offer: usize,
    pub detail_scroll: usize,
    pub detail_rows: usize,
    pub detail_visible: usize,
    pub log_scroll: usize,
    pub overlay: Overlay,
    /// The file name typed into the export prompt.
    pub export_path: String,
    /// Why the last export attempt was refused, shown beside the prompt.
    pub export_error: Option<String>,
    /// Whether the static data archive is being fetched right now.
    pub downloading: bool,
    /// Why the last static data download failed, shown in its dialog.
    pub download_error: Option<String>,
}

impl App {
    /// An app that has not started a run yet. The region and corporation counts are unknown until
    /// the filters are applied, so the header shows what the command line asked for and nothing
    /// more.
    pub fn new(applied: LpStoreOptions) -> Self {
        let filters = filter_summary(&applied);
        let form = FilterForm::from_options(&applied);
        Self {
            region: String::new(),
            corporations: 0,
            filters,
            depth: DEFAULT_DEPTH,
            agent_filter: AgentFilter::default(),
            applied,
            form,
            stale: false,
            stage: Stage::Idle,
            log: VecDeque::new(),
            report: None,
            focus: Focus::Groups,
            group: 0,
            offer: 0,
            detail_scroll: 0,
            detail_rows: 0,
            detail_visible: 0,
            log_scroll: 0,
            overlay: Overlay::None,
            export_path: String::new(),
            export_error: None,
            downloading: false,
            download_error: None,
        }
    }

    /// Moves the stage forward for `event`, and keeps its line in the log if it has one.
    pub fn on_event(&mut self, event: &Event, line: Option<LogLine>) {
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

    /// Stores the finished report and logs its offer errors, as the CLI prints them.
    pub fn finish(&mut self, report: Report) {
        for group in &report.groups {
            for error in &group.errors {
                self.push_log(LogLine {
                    text: format!("Failed to price offer {}: {}", error.offer_id, error.error),
                    is_error: true,
                });
            }
        }
        self.stage = Stage::Finished {
            groups: report.groups.len(),
        };
        self.stale = false;
        self.report = Some(report);
        self.focus = Focus::Groups;
        self.group = 0;
        self.offer = 0;
        self.detail_scroll = 0;
    }

    pub fn fail(&mut self) {
        self.stage = Stage::Failed;
    }

    /// Takes in what a run was resolved to, without moving the stage on or logging it.
    pub(crate) fn begin_run(&mut self, info: RunInfo) {
        self.region = info.region;
        self.corporations = info.corporations;
        self.filters = info.filters;
        self.agent_filter = info.agent_filter;
        self.apply_depth(info.depth);
    }

    /// Switches the view to a new run. Earlier results stay on screen, marked stale, until
    /// this run finishes.
    pub fn start_run(&mut self, info: RunInfo) {
        self.begin_run(info);
        self.stale = self.report.is_some();
        self.stage = Stage::Starting;
        self.close_form();
        self.close_export();
        let text = format!("Started a run with {}", self.filters);
        self.push_log(LogLine {
            text,
            is_error: false,
        });
    }

    /// Changes how many offers each group lists. The run itself is unchanged, so the group
    /// order is re-sorted for the new depth and the selected group stays selected.
    pub fn apply_depth(&mut self, depth: usize) {
        let selected = self.selected_group().map(|group| group.corp_ids.clone());
        self.depth = depth;
        if let Some(report) = &mut self.report {
            report.sort_by_depth(depth);
            if let Some(selected) = selected
                && let Some(index) = report.groups.iter().position(|g| g.corp_ids == selected)
            {
                self.group = index;
            }
        }
        let max = self.offer_count().saturating_sub(1);
        self.offer = self.offer.min(max);
    }

    pub fn set_applied(&mut self, applied: LpStoreOptions) {
        self.applied = applied;
    }

    pub fn open_form(&mut self) {
        self.form = FilterForm::from_options(&self.applied);
        self.overlay = Overlay::Filters;
    }

    pub fn close_form(&mut self) {
        if self.overlay == Overlay::Filters {
            self.overlay = Overlay::None;
        }
    }

    /// Opens the filters dialog with `form` already filled in, which is how the region and depth
    /// defaults get pre-filled without living in the options the run was asked for.
    pub fn show_filters(&mut self, form: FilterForm) {
        self.form = form;
        self.overlay = Overlay::Filters;
    }

    /// Asks whether to fetch the static data. Reached when the files are not on disk, or when they
    /// are and did not parse, in which case `reason` says so and a download replaces them.
    pub fn open_static_data(&mut self, reason: Option<String>) {
        self.download_error = reason;
        self.downloading = false;
        self.overlay = Overlay::StaticData;
    }

    /// The user chose to download. The fetch itself belongs to the caller, which owns the runtime.
    pub fn start_download(&mut self) {
        self.downloading = true;
        self.download_error = None;
    }

    /// The fetch came back with data, so the filters dialog can be opened over it.
    pub fn download_finished(&mut self, form: FilterForm) {
        self.downloading = false;
        self.download_error = None;
        self.push_log(LogLine {
            text: "Downloaded the static data".to_string(),
            is_error: false,
        });
        self.show_filters(form);
    }

    /// The fetch failed. The dialog stays open so it can be tried again, with the reason beside it.
    pub fn download_failed(&mut self, reason: String) {
        self.downloading = false;
        self.push_log(LogLine {
            text: format!("Static data download failed: {}", reason),
            is_error: true,
        });
        self.download_error = Some(reason);
    }

    /// Opens the export prompt with a name built from the region, so Enter on its own writes
    /// somewhere sensible. Only reachable once there are results to write.
    pub fn open_export(&mut self) {
        self.export_path = self.suggested_path();
        self.export_error = None;
        self.overlay = Overlay::Export;
    }

    pub fn close_export(&mut self) {
        if self.overlay == Overlay::Export {
            self.overlay = Overlay::None;
        }
    }

    /// Keeps a refusal beside the prompt, so the name stays editable.
    pub fn refuse_export(&mut self, reason: impl Into<String>) {
        self.export_error = Some(reason.into());
    }

    /// Where the report on screen would be written, as the prompt names it. `None` when the name
    /// has no known extension, which is the same rule the exporter applies.
    pub fn export_request(&self) -> Option<ExportRequest> {
        ExportRequest::parse(&self.export_path)
    }

    /// A file name for this run: the mode, the region, and the filters, in lower case with
    /// anything that is not a letter or a digit turned into a dash. The filter part is short,
    /// because a name that long stops being readable, and it is only a starting point the user
    /// edits before pressing Enter.
    fn suggested_path(&self) -> String {
        fn slug(text: &str, limit: usize) -> String {
            text.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .take(limit)
                .collect()
        }
        let mut name = format!("lpstores-{}", slug(&self.region, 64));
        if self.filters != "none" {
            name.push_str(&format!("-{}", slug(&self.filters, 32)));
        }
        name.push_str(".csv");
        name
    }

    pub fn push_log(&mut self, line: LogLine) {
        if self.log.len() == LOG_LIMIT {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Records how the details pane was drawn: its total rows, and how many fit on screen.
    /// Scrolling uses these, so the last row can always be reached.
    pub fn set_detail_layout(&mut self, rows: usize, visible: usize) {
        self.detail_rows = rows;
        self.detail_visible = visible;
    }

    pub fn stage_text(&self) -> String {
        match self.stage {
            Stage::Idle => "Not started".to_string(),
            Stage::Starting => "Starting...".to_string(),
            Stage::Fetching { done, total } => {
                format!("Fetching loyalty offers: {} / {} corporations", done, total)
            }
            Stage::Pricing { done, total } => format!("Pricing groups: {} / {}", done, total),
            Stage::Finished { groups } => format!("Finished: {} groups priced", groups),
            Stage::Failed => "The run stopped unexpectedly".to_string(),
        }
    }

    /// The (done, total) of the fetch gauge and of the pricing gauge.
    pub fn gauges(&self) -> ((usize, usize), (usize, usize)) {
        match self.stage {
            Stage::Idle | Stage::Starting | Stage::Failed => ((0, 0), (0, 0)),
            Stage::Fetching { done, total } => ((done, total), (0, 0)),
            Stage::Pricing { done, total } => {
                ((self.corporations, self.corporations), (done, total))
            }
            Stage::Finished { groups } => {
                ((self.corporations, self.corporations), (groups, groups))
            }
        }
    }

    pub fn groups(&self) -> &[Group] {
        match &self.report {
            Some(report) => &report.groups,
            None => &[],
        }
    }

    pub fn selected_group(&self) -> Option<&Group> {
        self.groups().get(self.group)
    }

    /// The offer under the selection, limited to the top `depth` offers.
    pub fn selected_offer(&self) -> Option<&OfferResult> {
        self.selected_group()?
            .results
            .get(self.offer)
            .filter(|_| self.offer < self.depth)
    }

    /// The number of offers the offers pane lists for the selected group.
    pub fn offer_count(&self) -> usize {
        self.selected_group()
            .map_or(0, |group| group.results.len().min(self.depth))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::Continue;
        }
        if self.overlay == Overlay::Filters {
            return self.on_form_key(key);
        }
        if self.overlay == Overlay::Export {
            return self.on_export_key(key);
        }
        if is_quit(&key) {
            return Action::Quit;
        }
        if self.overlay == Overlay::StaticData {
            return self.on_static_data_key(key);
        }
        if self.overlay == Overlay::Donate {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('d')) {
                self.overlay = Overlay::None;
            }
            return Action::Continue;
        }
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('?') => self.toggle(Overlay::Help),
            KeyCode::Char('l') => self.toggle(Overlay::Log),
            KeyCode::Char('f') => self.open_form(),
            KeyCode::Char('e') if self.report.is_some() => self.open_export(),
            KeyCode::Char('d') => self.toggle(Overlay::Donate),
            _ if self.overlay == Overlay::Help => {}
            KeyCode::Tab => {
                if self.overlay == Overlay::None && self.report.is_some() {
                    self.focus = self.focus.next();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.navigate(-1),
            KeyCode::Down | KeyCode::Char('j') => self.navigate(1),
            KeyCode::PageUp => self.navigate(-(PAGE as isize)),
            KeyCode::PageDown => self.navigate(PAGE as isize),
            _ => {}
        }
        Action::Continue
    }

    /// While the form is open, keys go to it, so `q` types a letter. Only Ctrl+C quits.
    fn on_form_key(&mut self, key: KeyEvent) -> Action {
        if is_ctrl_c(&key) {
            return Action::Quit;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                // The form the view opens on is the only thing between the user and a run, so
                // cancelling it has nowhere to go back to and stops instead. Once a run has been
                // started, Esc returns to the results it is showing.
                if self.stage == Stage::Idle {
                    return Action::Quit;
                }
                self.close_form();
            }
            KeyCode::Enter => return Action::Apply,
            KeyCode::Tab | KeyCode::Down => self.form.next_field(),
            KeyCode::BackTab | KeyCode::Up => self.form.previous_field(),
            KeyCode::Backspace => self.form.backspace(),
            KeyCode::Char('u') if ctrl => self.form.clear_focused(),
            KeyCode::Char(c) if plain => self.form.type_char(c),
            _ => {}
        }
        Action::Continue
    }

    /// The static data dialog. While the fetch is running every key is ignored, since there is
    /// nothing to interrupt but the process. Before it, Enter or `d` downloads and Esc or `x`
    /// stops the application. `q` and Ctrl+C quit through the usual rule.
    fn on_static_data_key(&mut self, key: KeyEvent) -> Action {
        if self.downloading {
            return Action::Continue;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('d') => Action::Download,
            KeyCode::Esc | KeyCode::Char('x') => Action::Quit,
            _ => Action::Continue,
        }
    }

    /// While the export prompt is open, keys go to it, so `q` types a letter. Only Ctrl+C quits.
    fn on_export_key(&mut self, key: KeyEvent) -> Action {
        if is_ctrl_c(&key) {
            return Action::Quit;
        }
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => self.close_export(),
            KeyCode::Enter => {
                return match self.export_request() {
                    Some(_) => {
                        self.export_error = None;
                        Action::Export
                    }
                    None => {
                        self.refuse_export("Use a file name ending in .csv or .json");
                        Action::Continue
                    }
                };
            }
            KeyCode::Backspace => {
                self.export_path.pop();
                self.export_error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.export_path.clear();
                self.export_error = None;
            }
            KeyCode::Char(c) if plain => {
                self.export_path.push(c);
                self.export_error = None;
            }
            _ => {}
        }
        Action::Continue
    }

    fn toggle(&mut self, overlay: Overlay) {
        self.overlay = if self.overlay == overlay {
            Overlay::None
        } else {
            overlay
        };
    }

    /// Moves the selection in the focused pane. Up and Down pass -1 and 1, and the page keys
    /// pass a whole page. In the log, moving up shows older lines.
    fn navigate(&mut self, delta: isize) {
        if self.overlay == Overlay::Log {
            let max = self.log.len().saturating_sub(1);
            self.log_scroll = shift(self.log_scroll, -delta, max);
            return;
        }
        if self.overlay != Overlay::None || self.report.is_none() {
            return;
        }
        match self.focus {
            Focus::Groups => {
                let max = self.groups().len().saturating_sub(1);
                let next = shift(self.group, delta, max);
                if next != self.group {
                    self.group = next;
                    self.offer = 0;
                    self.detail_scroll = 0;
                }
            }
            Focus::Offers => {
                let max = self.offer_count().saturating_sub(1);
                let next = shift(self.offer, delta, max);
                if next != self.offer {
                    self.offer = next;
                    self.detail_scroll = 0;
                }
            }
            Focus::Details => {
                let max = self.detail_rows.saturating_sub(self.detail_visible);
                self.detail_scroll = shift(self.detail_scroll, delta, max);
            }
        }
    }
}

fn shift(value: usize, delta: isize, max: usize) -> usize {
    (value as isize + delta).clamp(0, max as isize) as usize
}

/// Whether a key press quits outside the form: `q`, or Ctrl+C.
fn is_quit(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => key.modifiers.is_empty(),
        _ => is_ctrl_c(key),
    }
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;
    use eve_lpstore::esi::error::EsiError;
    use eve_lpstore::esi::loyalty::LoyaltyOffer;
    use eve_lpstore::{ExpenseBreakdown, OfferError};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn info(depth: usize) -> RunInfo {
        RunInfo {
            region: "The Forge".to_string(),
            corporations: 3,
            filters: "none".to_string(),
            depth,
            agent_filter: AgentFilter::default(),
        }
    }

    fn new_app(depth: usize) -> App {
        let mut app = App::new(LpStoreOptions::default());
        app.begin_run(info(depth));
        app.stage = Stage::Starting;
        app
    }

    fn offer(isk_per_lp: f64) -> OfferResult {
        OfferResult {
            offer: LoyaltyOffer {
                offer_id: 1,
                type_id: 1,
                quantity: 1,
                lp_cost: 1,
                isk_cost: 0.0,
                required_items: Vec::new(),
            },
            breakdown: ExpenseBreakdown {
                offer_id: 1,
                lp_cost: 1,
                isk_cost: 0.0,
                item_expenses: Vec::new(),
                total_isk: 0.0,
            },
            reward: None,
            unit_cost: None,
            isk_per_lp: Some(isk_per_lp),
            best_ask: None,
            spread: None,
            book_depth: None,
            imbalance: None,
            within_10: None,
        }
    }

    fn group(corp_id: i32, isk_values: &[f64]) -> Group {
        let results: Vec<OfferResult> = isk_values.iter().map(|&value| offer(value)).collect();
        Group {
            corp_ids: vec![corp_id],
            offer_count: results.len(),
            results,
            errors: Vec::new(),
        }
    }

    fn app_with(groups: Vec<Group>, depth: usize) -> App {
        let mut app = new_app(depth);
        app.finish(Report { groups });
        app
    }

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
        let mut app = app_with(Vec::new(), 10);
        let release = KeyEvent {
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
            ..press(KeyCode::Char('q'))
        };
        assert_eq!(app.on_key(release), Action::Continue);
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn progress_events_move_the_stage_forward() {
        let mut app = new_app(10);
        assert_eq!(app.stage, Stage::Starting);
        app.on_event(&Event::FetchStarted { corporations: 3 }, None);
        assert_eq!(app.stage, Stage::Fetching { done: 0, total: 3 });
        app.on_event(&Event::FetchProgress { done: 2, total: 3 }, None);
        assert_eq!(app.stage, Stage::Fetching { done: 2, total: 3 });
        app.on_event(&Event::PricingStarted { groups: 2 }, None);
        assert_eq!(app.stage, Stage::Pricing { done: 0, total: 2 });
        assert_eq!(app.gauges(), ((3, 3), (0, 2)));
    }

    #[test]
    fn a_failed_run_says_so() {
        let mut app = new_app(10);
        app.fail();
        assert_eq!(app.stage, Stage::Failed);
        assert_eq!(app.stage_text(), "The run stopped unexpectedly");
    }

    #[test]
    fn log_keeps_only_the_newest_lines() {
        let mut app = new_app(10);
        for i in 0..LOG_LIMIT + 10 {
            app.push_log(LogLine {
                text: format!("line {}", i),
                is_error: false,
            });
        }
        assert_eq!(app.log.len(), LOG_LIMIT);
        assert_eq!(
            app.log.front().map(|line| line.text.as_str()),
            Some("line 10")
        );
    }

    #[test]
    fn finish_stores_the_report_and_logs_offer_errors() {
        let mut failing = group(1, &[5.0]);
        failing.errors.push(OfferError {
            offer_id: 7,
            error: EsiError::Esi {
                status: 404,
                message: "no sell orders".to_string(),
            },
        });
        let app = app_with(vec![failing], 10);
        assert_eq!(app.stage, Stage::Finished { groups: 1 });
        assert_eq!(app.groups().len(), 1);
        let last = app.log.back().expect("the offer error is logged");
        assert!(last.is_error);
        assert!(last.text.starts_with("Failed to price offer 7"));
    }

    #[test]
    fn arrows_move_between_groups_and_reset_the_offer() {
        let mut app = app_with(vec![group(1, &[5.0, 3.0]), group(2, &[2.0])], 10);
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.offer, 1);
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.group, 1);
        assert_eq!(app.offer, 0);
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.group, 1, "the last group stays selected");
        app.on_key(press(KeyCode::Char('k')));
        assert_eq!(app.group, 0);
    }

    #[test]
    fn offers_stop_at_the_depth_limit() {
        let mut app = app_with(vec![group(1, &[5.0, 4.0, 3.0, 2.0])], 2);
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.offer, 1);
        assert_eq!(app.selected_offer().map(|o| o.isk_per_lp), Some(Some(4.0)));
    }

    #[test]
    fn help_swallows_navigation_until_it_is_closed() {
        let mut app = app_with(vec![group(1, &[5.0]), group(2, &[2.0])], 10);
        app.on_key(press(KeyCode::Char('?')));
        assert_eq!(app.overlay, Overlay::Help);
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.group, 0);
        app.on_key(press(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.group, 1);
    }

    #[test]
    fn log_view_scrolls_back_through_the_log() {
        let mut app = new_app(10);
        for i in 0..5 {
            app.push_log(LogLine {
                text: format!("line {}", i),
                is_error: false,
            });
        }
        app.on_key(press(KeyCode::Char('l')));
        assert_eq!(app.overlay, Overlay::Log);
        app.on_key(press(KeyCode::Up));
        assert_eq!(app.log_scroll, 1);
        app.on_key(press(KeyCode::PageUp));
        assert_eq!(app.log_scroll, 4, "scrolling stops at the oldest line");
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.log_scroll, 0);
    }

    #[test]
    fn detail_scroll_stops_when_the_last_row_is_visible() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.set_detail_layout(9, 4);
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Tab));
        for _ in 0..20 {
            app.on_key(press(KeyCode::Down));
        }
        assert_eq!(app.detail_scroll, 5);
    }

    #[test]
    fn tab_only_switches_panes_in_the_results() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Groups);
        app.finish(Report { groups: Vec::new() });
        app.on_key(press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Offers);
    }

    #[test]
    fn q_is_a_letter_in_the_form() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.overlay, Overlay::Filters);
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Continue);
        assert_eq!(app.form.corporations, "q");
        assert_eq!(app.on_key(ctrl('c')), Action::Quit);
    }

    #[test]
    fn d_opens_the_donation_overlay_until_esc_or_d_closes_it() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.on_key(press(KeyCode::Char('d')));
        assert_eq!(app.overlay, Overlay::Donate);
        app.on_key(press(KeyCode::Down));
        app.on_key(press(KeyCode::Char('?')));
        assert_eq!(
            app.overlay,
            Overlay::Donate,
            "other keys wait until it closes"
        );
        app.on_key(press(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(press(KeyCode::Char('d')));
        app.on_key(press(KeyCode::Char('d')));
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn q_still_quits_from_the_donation_overlay() {
        let mut app = app_with(Vec::new(), 10);
        app.on_key(press(KeyCode::Char('d')));
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn d_is_a_letter_in_the_form() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        app.on_key(press(KeyCode::Char('d')));
        assert_eq!(app.form.corporations, "d");
    }

    #[test]
    fn e_opens_the_export_prompt_with_a_name_from_the_region() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        assert_eq!(app.on_key(press(KeyCode::Char('e'))), Action::Continue);
        assert_eq!(app.overlay, Overlay::Export);
        assert_eq!(app.export_path, "lpstores-the-forge.csv");
        assert_eq!(
            app.export_request().map(|request| request.format),
            Some(eve_lpstore::export::ExportFormat::Csv)
        );
    }

    #[test]
    fn e_does_nothing_until_there_are_results() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('e')));
        assert_eq!(app.overlay, Overlay::None, "there is nothing to write yet");
    }

    #[test]
    fn the_suggested_name_carries_the_filters() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.filters = "faction Caldari".to_string();
        app.open_export();
        assert_eq!(app.export_path, "lpstores-the-forge-faction-caldari.csv");
        app.filters = "a".repeat(80);
        app.open_export();
        assert!(
            app.export_path.len() < 80,
            "the filter part is cut short: {}",
            app.export_path
        );
    }

    #[test]
    fn enter_in_the_export_prompt_asks_for_the_write() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        app.export_path = "out/offers.json".to_string();
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Export);
        assert_eq!(
            app.overlay,
            Overlay::Export,
            "the prompt stays open until it is written"
        );
    }

    #[test]
    fn enter_refuses_an_unknown_extension() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        app.export_path = "offers.txt".to_string();
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Continue);
        assert!(app.export_error.unwrap().contains(".csv or .json"));
        assert_eq!(app.overlay, Overlay::Export, "the name stays editable");
    }

    #[test]
    fn editing_the_export_name_clears_the_refusal() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        app.export_path = "offers".to_string();
        app.on_key(press(KeyCode::Enter));
        assert!(app.export_error.is_some());
        app.on_key(press(KeyCode::Char('.')));
        assert_eq!(app.export_error, None, "the refusal is about the old name");
        app.on_key(press(KeyCode::Char('c')));
        app.on_key(press(KeyCode::Char('s')));
        app.on_key(press(KeyCode::Char('v')));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Export);
    }

    #[test]
    fn q_is_a_letter_in_the_export_prompt() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        app.export_path.clear();
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Continue);
        assert_eq!(app.export_path, "q");
    }

    #[test]
    fn ctrl_c_quits_from_the_export_prompt() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        assert_eq!(app.on_key(ctrl('c')), Action::Quit);
    }

    #[test]
    fn backspace_and_ctrl_u_edit_the_export_name() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        for _ in 0..4 {
            app.on_key(press(KeyCode::Backspace));
        }
        assert_eq!(app.export_path, "lpstores-the-forge");
        app.on_key(press(KeyCode::Char('u')));
        assert_eq!(
            app.export_path, "lpstores-the-forgeu",
            "plain u types a letter"
        );
        app.on_key(ctrl('u'));
        assert_eq!(app.export_path, "", "Ctrl+U clears the whole name");
    }

    #[test]
    fn escape_closes_the_export_prompt() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn starting_a_run_closes_the_export_prompt() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        app.start_run(info(10));
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn the_static_data_dialog_offers_download_or_exit() {
        let mut app = new_app(10);
        app.open_static_data(None);
        assert_eq!(app.overlay, Overlay::StaticData);
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Download);
        app.open_static_data(None);
        assert_eq!(app.on_key(press(KeyCode::Char('d'))), Action::Download);
        app.open_static_data(None);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Quit);
        app.open_static_data(None);
        assert_eq!(app.on_key(press(KeyCode::Char('x'))), Action::Quit);
    }

    #[test]
    fn q_still_quits_from_the_static_data_dialog() {
        let mut app = new_app(10);
        app.open_static_data(None);
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn keys_do_nothing_while_the_static_data_downloads() {
        let mut app = new_app(10);
        app.open_static_data(None);
        app.start_download();
        assert!(app.downloading);
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Continue);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.on_key(press(KeyCode::Char('d'))), Action::Continue);
        assert_eq!(app.overlay, Overlay::StaticData, "the dialog stays up");
    }

    #[test]
    fn a_finished_download_opens_the_filters_dialog() {
        let mut app = new_app(10);
        app.open_static_data(None);
        app.start_download();
        let form = FilterForm::from_options(&LpStoreOptions::default());
        app.download_finished(form);
        assert_eq!(app.overlay, Overlay::Filters, "the filters come next");
        assert!(!app.downloading);
        assert_eq!(app.download_error, None);
        let line = app.log.back().expect("the download is reported");
        assert!(!line.is_error);
        assert!(line.text.contains("Downloaded the static data"));
    }

    #[test]
    fn a_failed_download_keeps_the_dialog_open_with_the_reason() {
        let mut app = new_app(10);
        app.open_static_data(None);
        app.start_download();
        app.download_failed("connection refused".to_string());
        assert_eq!(
            app.overlay,
            Overlay::StaticData,
            "the dialog stays up so it can be retried"
        );
        assert!(!app.downloading);
        assert_eq!(app.download_error.as_deref(), Some("connection refused"));
        let line = app.log.back().expect("the failure is logged");
        assert!(line.is_error);
        assert!(line.text.contains("connection refused"));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Download);
    }

    #[test]
    fn unreadable_static_data_is_shown_with_the_reason_it_cannot_be_used() {
        let mut app = new_app(10);
        app.open_static_data(Some("data/regions.json is not valid JSON".to_string()));
        assert_eq!(app.overlay, Overlay::StaticData);
        assert_eq!(
            app.download_error.as_deref(),
            Some("data/regions.json is not valid JSON")
        );
    }

    #[test]
    fn the_view_starts_idle_with_only_what_the_command_line_asked_for() {
        let options = LpStoreOptions {
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        let app = App::new(options);
        assert_eq!(app.stage, Stage::Idle);
        assert!(app.report.is_none());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.filters, "--faction caldari");
        assert_eq!(app.region, "", "the region is not known until a run starts");
        assert_eq!(app.corporations, 0);
    }

    #[test]
    fn escape_from_the_startup_filters_dialog_quits() {
        // Nothing has been fetched yet, so there is no view to go back to.
        let options = LpStoreOptions::default();
        let mut app = App::new(options);
        let form = FilterForm::from_options(&LpStoreOptions::default());
        app.show_filters(form);
        assert_eq!(app.stage, Stage::Idle);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Quit);
    }

    #[test]
    fn escape_from_the_filters_dialog_after_a_run_cancels_it() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.overlay, Overlay::Filters);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.overlay, Overlay::None, "the results stay on screen");
    }

    #[test]
    fn escape_closes_the_form_and_discards_the_edits() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        app.on_key(press(KeyCode::Char('x')));
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.form.corporations, "");
    }

    #[test]
    fn enter_asks_for_the_form_to_be_applied() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Apply);
    }

    #[test]
    fn f_opens_the_form_from_any_view() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('l')));
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.overlay, Overlay::Filters);
    }

    #[test]
    fn a_new_run_keeps_the_old_results_stale_until_it_finishes() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(info(10));
        assert!(app.stale);
        assert_eq!(app.stage, Stage::Starting);
        assert_eq!(app.groups().len(), 1, "the old results stay on screen");
        app.finish(Report {
            groups: vec![group(2, &[3.0])],
        });
        assert!(!app.stale);
        assert_eq!(app.groups()[0].corp_ids, vec![2]);
    }

    #[test]
    fn a_depth_change_keeps_the_offer_in_range() {
        let mut app = app_with(vec![group(1, &[5.0, 4.0, 3.0, 2.0])], 4);
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.offer, 3);
        app.apply_depth(2);
        assert_eq!(app.offer, 1);
        assert_eq!(app.offer_count(), 2);
    }

    #[test]
    fn a_depth_change_reorders_and_keeps_the_group_selected() {
        // Depth 1 averages 10 for corp 1 and 6 for corp 2, so corp 1 sorts first. Depth 2
        // averages 5 for corp 1 and 6 for corp 2, so the order flips.
        let mut app = app_with(vec![group(1, &[10.0, 0.0]), group(2, &[6.0, 6.0])], 1);
        app.apply_depth(1);
        assert_eq!(app.groups()[0].corp_ids, vec![1]);
        assert_eq!(app.group, 0);

        app.apply_depth(2);
        assert_eq!(app.groups()[0].corp_ids, vec![2]);
        assert_eq!(app.groups()[1].corp_ids, vec![1]);
        assert_eq!(
            app.group, 1,
            "the same group stays selected after the reorder"
        );
    }
}

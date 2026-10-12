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
        offers: usize,
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

/// What is drawn over the results, with the state that only exists while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    None,
    Log,
    Help,
    Filters,
    Donate,
    Export(ExportPrompt),
    /// Where the missions of the selected group's agent systems can take you.
    Destinations,
    /// Asking whether to download the static data, shown when it is not on disk.
    StaticData(StaticDataDialog),
}

/// The export prompt: the file name typed into it, and why the last attempt was refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportPrompt {
    pub path: String,
    pub error: Option<String>,
}

/// The static data dialog: whether the archive is being fetched right now, and why the last
/// download failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticDataDialog {
    pub downloading: bool,
    pub error: Option<String>,
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

/// What a run was resolved to, as the header and the detail pane show it. The filter summary and
/// the depth are handed to the view separately, because a depth-only apply changes the depth
/// without starting a run.
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub region: String,
    /// The region prices are read from, as an id. The label above is for the header; this is what
    /// says whether a new run moved regions.
    pub region_id: i32,
    /// The region the last run filtered agents by, if it filtered by one. `None` means no agent
    /// region filter, which is not a place a system can have moved out of.
    pub agent_region_id: Option<i32>,
    pub corporations: usize,
    pub agent_filter: AgentFilter,
}

pub struct App {
    /// The run in progress, or the one last started. `None` until the first one.
    pub run: Option<RunInfo>,
    /// The filters the header names: the run's, or before any run the command line's.
    pub filters: String,
    pub depth: usize,
    /// The options the current run was started with.
    pub applied: LpStoreOptions,
    pub form: FilterForm,
    /// Whether the report on screen comes from an earlier run than the one in progress.
    pub stale: bool,
    /// Whether a run that superseded the results on screen has ended and its outcome is still on
    /// the screen. The loop drains run events before it draws, so a warm run can finish inside one
    /// 250 ms tick and never be seen in flight; holding the summary until any key is pressed is what
    /// makes the end of such a run visible at all.
    pub awaiting_run_summary: bool,
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
    /// Scrolling within the mission destinations pane: how many rows it produced and how many fit on
    /// screen, so the keys bound the scroll the same way the details pane does.
    pub destinations_rows: usize,
    pub destinations_visible: usize,
    pub destinations_scroll: usize,
    /// Scrolling within the help, bounded the same way.
    pub help_rows: usize,
    pub help_visible: usize,
    pub help_scroll: usize,
    pub overlay: Overlay,
    /// Where the static data files live, for the dialog's text. The view is given this after the
    /// platform roots are found; until then it says only that there is a data directory.
    pub data_dir: String,
}

impl App {
    /// An app that has not started a run yet. The region and corporation counts are unknown until
    /// the filters are applied, so the header shows what the command line asked for and nothing
    /// more.
    pub fn new(applied: LpStoreOptions) -> Self {
        let filters = filter_summary(&applied);
        let form = FilterForm::from_options(&applied);
        Self {
            run: None,
            filters,
            depth: DEFAULT_DEPTH,
            applied,
            form,
            stale: false,
            awaiting_run_summary: false,
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
            destinations_rows: 0,
            destinations_visible: 0,
            destinations_scroll: 0,
            help_rows: 0,
            help_visible: 0,
            help_scroll: 0,
            overlay: Overlay::None,
            data_dir: super::ui::UNKNOWN_DATA_DIR.to_string(),
        }
    }

    /// The region the run prices in, as the header names it. Empty before the first run.
    pub fn region(&self) -> &str {
        self.run.as_ref().map_or("", |run| run.region.as_str())
    }

    /// How many corporations the run covers. Zero before the first run.
    pub fn corporations(&self) -> usize {
        self.run.as_ref().map_or(0, |run| run.corporations)
    }

    /// The agent filter of the run, which decides the systems the panes list. Before the first run
    /// it is the empty filter, which matches every agent.
    pub fn agent_filter(&self) -> AgentFilter {
        self.run
            .as_ref()
            .map(|run| run.agent_filter)
            .unwrap_or_default()
    }

    /// Tells the view where its files live, once the platform roots have been found.
    pub fn set_data_dir(&mut self, data_dir: impl Into<String>) {
        self.data_dir = data_dir.into();
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
            Event::PricingStarted { offers } => {
                self.stage = Stage::Pricing {
                    done: 0,
                    total: *offers,
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

    /// Stores the finished report and logs its offer errors, as the CLI prints them. The report
    /// is regrouped for the view's depth, which a depth change during the run may have moved
    /// away from the run's own.
    pub fn finish(&mut self, mut report: Report) {
        // Read before `stale` is cleared below: a run that left no earlier results behind had the
        // progress view to itself, and needs no summary over the pane it just filled.
        let superseded = self.stale;
        for error in &report.errors {
            self.push_log(LogLine {
                text: format!("Failed to price offer {}: {}", error.offer_id, error.error),
                is_error: true,
            });
        }
        report.regroup(self.depth);
        self.stage = Stage::Finished {
            offers: report.offer_count(),
            groups: report.groups.len(),
        };
        self.stale = false;
        self.awaiting_run_summary = superseded;
        self.report = Some(report);
        self.focus = Focus::Groups;
        self.group = 0;
        self.offer = 0;
        self.detail_scroll = 0;
    }

    pub fn fail(&mut self) {
        self.stage = Stage::Failed;
        // `stale` deliberately stays set: the results on screen are still from a superseded run. So
        // the same flag says whether this failure had results behind it, and a failed first run is
        // left to the progress view, which already reports it in red.
        self.awaiting_run_summary = self.stale;
    }

    /// Takes in what a run was resolved to, without moving the stage on or logging it.
    pub(crate) fn begin_run(&mut self, info: RunInfo, filters: String, depth: usize) {
        self.run = Some(info);
        self.filters = filters;
        self.apply_depth(depth);
    }

    /// Switches the view to a new run. Earlier results stay on screen, marked stale, until
    /// this run finishes.
    pub fn start_run(&mut self, info: RunInfo, filters: String, depth: usize) {
        self.begin_run(info, filters, depth);
        self.stale = self.report.is_some();
        // The previous run's summary, if one is still up, is not this run's news.
        self.awaiting_run_summary = false;
        self.stage = Stage::Starting;
        self.close_form();
        self.close_export();
        let text = format!("Started a run with {}", self.filters);
        self.push_log(LogLine {
            text,
            is_error: false,
        });
    }

    /// Changes how many offers each group lists. The run itself is unchanged, so the groups are
    /// rebuilt for the new depth from the offers already priced, which can join or split them,
    /// and the group holding the selected group's first corporation stays selected.
    pub fn apply_depth(&mut self, depth: usize) {
        let selected = self.selected_group().map(|group| group.corp_ids[0]);
        self.depth = depth;
        if let Some(report) = &mut self.report {
            report.regroup(depth);
            if let Some(selected) = selected
                && let Some(index) = report
                    .groups
                    .iter()
                    .position(|g| g.corp_ids.contains(&selected))
            {
                self.group = index;
            }
        }
        let max = self.offer_count().saturating_sub(1);
        self.offer = self.offer.min(max);
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
        self.overlay = Overlay::StaticData(StaticDataDialog {
            downloading: false,
            error: reason,
        });
    }

    /// The user chose to download. The fetch itself belongs to the caller, which owns the runtime.
    pub fn start_download(&mut self) {
        if let Overlay::StaticData(dialog) = &mut self.overlay {
            dialog.downloading = true;
            dialog.error = None;
        }
    }

    /// The fetch came back with data, so the filters dialog can be opened over it.
    pub fn download_finished(&mut self, form: FilterForm) {
        self.push_log(LogLine {
            text: "Downloaded the static data".to_string(),
            is_error: false,
        });
        self.show_filters(form);
    }

    /// The fetch failed. The dialog stays open so it can be tried again, with the reason beside it.
    pub fn download_failed(&mut self, reason: String) {
        self.push_log(LogLine {
            text: format!("Static data download failed: {}", reason),
            is_error: true,
        });
        if let Overlay::StaticData(dialog) = &mut self.overlay {
            dialog.downloading = false;
            dialog.error = Some(reason);
        }
    }

    /// Opens the export prompt with a name built from the region, so Enter on its own writes
    /// somewhere sensible. Only reachable once there are results to write.
    pub fn open_export(&mut self) {
        self.overlay = Overlay::Export(ExportPrompt {
            path: self.suggested_path(),
            error: None,
        });
    }

    /// The export prompt, while it is open.
    pub fn export_prompt(&self) -> Option<&ExportPrompt> {
        match &self.overlay {
            Overlay::Export(prompt) => Some(prompt),
            _ => None,
        }
    }

    pub fn close_export(&mut self) {
        if matches!(self.overlay, Overlay::Export(_)) {
            self.overlay = Overlay::None;
        }
    }

    /// Keeps a refusal beside the prompt, so the name stays editable.
    pub fn refuse_export(&mut self, reason: impl Into<String>) {
        if let Overlay::Export(prompt) = &mut self.overlay {
            prompt.error = Some(reason.into());
        }
    }

    /// Where the report on screen would be written, as the prompt names it. `None` when the name
    /// has no known extension, which is the same rule the exporter applies, and when the prompt is
    /// not open.
    pub fn export_request(&self) -> Option<ExportRequest> {
        ExportRequest::parse(&self.export_prompt()?.path)
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
        let mut name = format!("lpstores-{}", slug(self.region(), 64));
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

    /// Records how tall the destinations pane was last frame, which bounds its scrolling. The pane is
    /// only measured while it is open, so a closed pane keeps the last count rather than resetting it;
    /// the scroll is clamped against it either way.
    pub fn set_destinations_layout(&mut self, rows: usize, visible: usize) {
        self.destinations_rows = rows;
        self.destinations_visible = visible;
    }

    pub fn set_help_layout(&mut self, rows: usize, visible: usize) {
        self.help_rows = rows;
        self.help_visible = visible;
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
            Stage::Pricing { done, total } => format!("Pricing offers: {} / {}", done, total),
            Stage::Finished { offers, groups } => {
                format!("Finished: {} offers priced, {} groups", offers, groups)
            }
            Stage::Failed => "The run stopped unexpectedly".to_string(),
        }
    }

    /// The (done, total) of the fetch gauge and of the pricing gauge.
    pub fn gauges(&self) -> ((usize, usize), (usize, usize)) {
        match self.stage {
            Stage::Idle | Stage::Starting | Stage::Failed => ((0, 0), (0, 0)),
            Stage::Fetching { done, total } => ((done, total), (0, 0)),
            Stage::Pricing { done, total } => {
                let corporations = self.corporations();
                ((corporations, corporations), (done, total))
            }
            Stage::Finished { offers, .. } => {
                let corporations = self.corporations();
                ((corporations, corporations), (offers, offers))
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
            .map(|result| &**result)
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
        // Any key acknowledges a run summary. It is not a mode and takes no key for itself: what the
        // press goes on to do — move a selection, open the log, quit — is unchanged.
        self.awaiting_run_summary = false;
        // In order: the two text prompts take every key, so `q` types a letter there; everywhere
        // else `q` and Ctrl+C quit before the overlay sees the key.
        match &self.overlay {
            Overlay::Filters => self.on_form_key(key),
            Overlay::Export(_) => self.on_export_key(key),
            _ if is_quit(&key) => Action::Quit,
            Overlay::StaticData(dialog) => on_static_data_key(dialog, key),
            Overlay::Donate => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('d')) {
                    self.overlay = Overlay::None;
                }
                Action::Continue
            }
            Overlay::None | Overlay::Log | Overlay::Help | Overlay::Destinations => {
                self.on_view_key(key)
            }
        }
    }

    /// The keys of the results view and of the overlays that only show something: the log, the
    /// help, and the mission destinations.
    fn on_view_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('?') => {
                // The help opens at its top, on the keys, whatever was scrolled to last time.
                self.help_scroll = 0;
                self.toggle(Overlay::Help);
            }
            KeyCode::Char('l') => self.toggle(Overlay::Log),
            KeyCode::Char('f') => self.open_form(),
            KeyCode::Char('e') if self.report.is_some() => self.open_export(),
            KeyCode::Char('d') => self.toggle(Overlay::Donate),
            KeyCode::Char('m') if self.report.is_some() => self.toggle(Overlay::Destinations),
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
        if let Some(edit) = Edit::of(&key) {
            match edit {
                Edit::Type(c) => self.form.type_char(c),
                Edit::Backspace => self.form.backspace(),
                Edit::Clear => self.form.clear_focused(),
            }
            return Action::Continue;
        }
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
            _ => {}
        }
        Action::Continue
    }

    /// While the export prompt is open, keys go to it, so `q` types a letter. Only Ctrl+C quits.
    fn on_export_key(&mut self, key: KeyEvent) -> Action {
        if is_ctrl_c(&key) {
            return Action::Quit;
        }
        if key.code == KeyCode::Esc {
            self.close_export();
            return Action::Continue;
        }
        let Overlay::Export(prompt) = &mut self.overlay else {
            return Action::Continue;
        };
        if let Some(edit) = Edit::of(&key) {
            edit.apply(&mut prompt.path);
            prompt.error = None;
        } else if key.code == KeyCode::Enter {
            if ExportRequest::parse(&prompt.path).is_some() {
                prompt.error = None;
                return Action::Export;
            }
            prompt.error = Some("Use a file name ending in .csv or .json".to_string());
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
        if self.overlay == Overlay::Help {
            let max = self.help_rows.saturating_sub(self.help_visible);
            self.help_scroll = shift(self.help_scroll, delta, max);
            return;
        }
        if self.overlay == Overlay::Destinations {
            // The details-pane direction and the details-pane bound: this table reads top to bottom,
            // so Down moves to later rows and the last row stays on screen. The log is inverted
            // because its newest line is at the bottom.
            let max = self
                .destinations_rows
                .saturating_sub(self.destinations_visible);
            self.destinations_scroll = shift(self.destinations_scroll, delta, max);
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

/// What a key does to a line of text being typed, shared by the filter form and the export prompt:
/// Backspace removes the last character, Ctrl+U clears the line, and a character typed without
/// Ctrl or Alt is added. `None` for every other key, which the prompt handles itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Type(char),
    Backspace,
    Clear,
}

impl Edit {
    fn of(key: &KeyEvent) -> Option<Edit> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        match key.code {
            KeyCode::Backspace => Some(Edit::Backspace),
            KeyCode::Char('u') if ctrl => Some(Edit::Clear),
            KeyCode::Char(c) if plain => Some(Edit::Type(c)),
            _ => None,
        }
    }

    fn apply(self, text: &mut String) {
        match self {
            Edit::Type(c) => text.push(c),
            Edit::Backspace => {
                text.pop();
            }
            Edit::Clear => text.clear(),
        }
    }
}

/// The static data dialog. While the fetch is running every key is ignored, since there is
/// nothing to interrupt but the process. Before it, Enter or `d` downloads and Esc or `x`
/// stops the application. `q` and Ctrl+C quit through the usual rule, which `on_key` applies first.
fn on_static_data_key(dialog: &StaticDataDialog, key: KeyEvent) -> Action {
    if dialog.downloading {
        return Action::Continue;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Char('d') => Action::Download,
        KeyCode::Esc | KeyCode::Char('x') => Action::Quit,
        _ => Action::Continue,
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
    use crate::tui::test_support::{dialog, forge_run, open_prompt, open_prompt_mut, press};
    use crossterm::event::KeyEventState;
    use eve_lpstore::report::{Field, Store};
    use eve_lpstore::{OfferError, PricingError};
    use std::sync::Arc;

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn new_app(depth: usize) -> App {
        let mut app = App::new(LpStoreOptions::default());
        app.begin_run(forge_run(3), "none".to_string(), depth);
        app.stage = Stage::Starting;
        app
    }

    fn offer(isk_per_lp: f64) -> OfferResult {
        OfferResult {
            isk_per_lp: Some(isk_per_lp),
            ..Default::default()
        }
    }

    /// The store of `corp_id`, whose offers have these ISK per LP values. Offer IDs are the
    /// corporation's times a hundred plus the offer's place, so no two stores share an offer.
    fn group(corp_id: i32, isk_values: &[f64]) -> Store {
        let results: Vec<Arc<OfferResult>> = isk_values
            .iter()
            .enumerate()
            .map(|(index, &value)| {
                let mut result = offer(value);
                result.offer.offer_id = corp_id * 100 + index as i32;
                Arc::new(result)
            })
            .collect();
        let offer_ids = results.iter().map(|result| result.offer.offer_id).collect();
        Store::new(vec![corp_id], offer_ids, results)
    }

    fn report(stores: Vec<Store>, errors: Vec<OfferError>) -> Report {
        Report::new(&[], stores, errors, 10)
    }

    fn app_with(stores: Vec<Store>, depth: usize) -> App {
        let mut app = new_app(depth);
        app.finish(report(stores, Vec::new()));
        app
    }

    #[test]
    fn the_editing_keys_are_read_the_same_way_in_both_prompts() {
        let with = |code, modifiers| Edit::of(&KeyEvent::new(code, modifiers));
        assert_eq!(
            with(KeyCode::Char('q'), KeyModifiers::NONE),
            Some(Edit::Type('q'))
        );
        assert_eq!(
            with(KeyCode::Char('Q'), KeyModifiers::SHIFT),
            Some(Edit::Type('Q')),
            "Shift still types"
        );
        assert_eq!(
            with(KeyCode::Char('u'), KeyModifiers::CONTROL),
            Some(Edit::Clear)
        );
        assert_eq!(
            with(KeyCode::Backspace, KeyModifiers::NONE),
            Some(Edit::Backspace)
        );
        assert_eq!(
            with(KeyCode::Char('x'), KeyModifiers::ALT),
            None,
            "Alt types nothing"
        );
        assert_eq!(
            with(KeyCode::Enter, KeyModifiers::NONE),
            None,
            "Enter is the prompt's own"
        );
        let mut text = "Jita".to_string();
        Edit::Backspace.apply(&mut text);
        Edit::Type('x').apply(&mut text);
        assert_eq!(text, "Jitx");
        Edit::Clear.apply(&mut text);
        assert_eq!(text, "");
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
        app.on_event(&Event::PricingStarted { offers: 2 }, None);
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
        let error = OfferError {
            offer_id: 7,
            error: PricingError::NoSellOrders {
                type_id: 34,
                region_id: 10000002,
            },
        };
        let mut app = new_app(10);
        app.finish(report(vec![group(1, &[5.0])], vec![error]));
        assert_eq!(
            app.stage,
            Stage::Finished {
                offers: 1,
                groups: 1
            }
        );
        assert_eq!(app.groups().len(), 1);
        let last = app.log.back().expect("the offer error is logged");
        assert!(last.is_error);
        assert_eq!(
            last.text,
            "Failed to price offer 7: no sell orders found for type_id 34 in region 10000002",
            "no ESI status, since no request failed"
        );
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
        app.finish(report(Vec::new(), Vec::new()));
        app.on_key(press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Offers);
    }

    #[test]
    fn q_is_a_letter_in_the_form() {
        let mut app = new_app(10);
        app.on_key(press(KeyCode::Char('f')));
        assert_eq!(app.overlay, Overlay::Filters);
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Continue);
        assert_eq!(app.form.value(Field::Lpstore), "q");
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
        assert_eq!(app.form.value(Field::Lpstore), "d");
    }

    #[test]
    fn e_opens_the_export_prompt_with_a_name_from_the_region() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        assert_eq!(app.on_key(press(KeyCode::Char('e'))), Action::Continue);
        assert!(matches!(app.overlay, Overlay::Export(_)));
        assert_eq!(open_prompt(&app).path, "lpstores-the-forge.csv");
        assert_eq!(
            app.export_request().map(|request| request.format),
            Some(eve_lpstore::export::ExportFormat::Csv)
        );
    }

    #[test]
    fn m_opens_the_missions_pane_only_once_there_are_results() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.on_key(press(KeyCode::Char('m')));
        assert_eq!(app.overlay, Overlay::Destinations);
        // Before the first run there is no selected group to describe.
        let mut idle = App::new(LpStoreOptions::default());
        idle.on_key(press(KeyCode::Char('m')));
        assert_eq!(idle.overlay, Overlay::None, "nothing to show yet");
    }

    #[test]
    fn the_missions_pane_closes_on_escape_and_quits_on_q() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.on_key(press(KeyCode::Char('m')));
        assert_eq!(app.overlay, Overlay::Destinations);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(press(KeyCode::Char('m')));
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn the_missions_pane_scrolls_without_moving_the_selection() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.on_key(press(KeyCode::Char('m')));
        app.set_destinations_layout(30, 10);
        let group_before = app.group;
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.destinations_scroll, 1, "down shows lower rows");
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.destinations_scroll, 11);
        app.on_key(press(KeyCode::Up));
        assert_eq!(app.destinations_scroll, 10);
        assert_eq!(app.group, group_before, "scrolling is not selection");
        for _ in 0..40 {
            app.on_key(press(KeyCode::Down));
        }
        // 30 rows with 10 on screen: the last window shows rows 20 to 29.
        assert_eq!(
            app.destinations_scroll, 20,
            "stops with the last row on screen"
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
        assert_eq!(
            open_prompt(&app).path,
            "lpstores-the-forge-faction-caldari.csv"
        );
        app.filters = "a".repeat(80);
        app.open_export();
        assert!(
            open_prompt(&app).path.len() < 80,
            "the filter part is cut short: {}",
            open_prompt(&app).path
        );
    }

    #[test]
    fn enter_in_the_export_prompt_asks_for_the_write() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        open_prompt_mut(&mut app).path = "out/offers.json".to_string();
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Export);
        assert!(
            matches!(app.overlay, Overlay::Export(_)),
            "the prompt stays open until it is written"
        );
    }

    #[test]
    fn enter_refuses_an_unknown_extension() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        open_prompt_mut(&mut app).path = "offers.txt".to_string();
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Continue);
        assert!(
            open_prompt(&app)
                .error
                .clone()
                .unwrap()
                .contains(".csv or .json")
        );
        assert!(
            matches!(app.overlay, Overlay::Export(_)),
            "the name stays editable"
        );
    }

    #[test]
    fn editing_the_export_name_clears_the_refusal() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        open_prompt_mut(&mut app).path = "offers".to_string();
        app.on_key(press(KeyCode::Enter));
        assert!(open_prompt(&app).error.is_some());
        app.on_key(press(KeyCode::Char('.')));
        assert_eq!(
            open_prompt(&app).error,
            None,
            "the refusal is about the old name"
        );
        app.on_key(press(KeyCode::Char('c')));
        app.on_key(press(KeyCode::Char('s')));
        app.on_key(press(KeyCode::Char('v')));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Export);
    }

    #[test]
    fn q_is_a_letter_in_the_export_prompt() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.open_export();
        open_prompt_mut(&mut app).path.clear();
        assert_eq!(app.on_key(press(KeyCode::Char('q'))), Action::Continue);
        assert_eq!(open_prompt(&app).path, "q");
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
        assert_eq!(open_prompt(&app).path, "lpstores-the-forge");
        app.on_key(press(KeyCode::Char('u')));
        assert_eq!(
            open_prompt(&app).path,
            "lpstores-the-forgeu",
            "plain u types a letter"
        );
        app.on_key(ctrl('u'));
        assert_eq!(open_prompt(&app).path, "", "Ctrl+U clears the whole name");
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
        app.start_run(forge_run(3), "none".to_string(), 10);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn the_static_data_dialog_offers_download_or_exit() {
        let mut app = new_app(10);
        app.open_static_data(None);
        assert!(matches!(app.overlay, Overlay::StaticData(_)));
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
        assert!(dialog(&app).downloading);
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Continue);
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.on_key(press(KeyCode::Char('d'))), Action::Continue);
        assert!(
            matches!(app.overlay, Overlay::StaticData(_)),
            "the dialog stays up"
        );
    }

    #[test]
    fn a_finished_download_opens_the_filters_dialog() {
        let mut app = new_app(10);
        app.open_static_data(None);
        app.start_download();
        let form = FilterForm::from_options(&LpStoreOptions::default());
        app.download_finished(form);
        // The dialog's state closes with it, so there is no download flag left to clear.
        assert_eq!(app.overlay, Overlay::Filters, "the filters come next");
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
        assert!(
            matches!(app.overlay, Overlay::StaticData(_)),
            "the dialog stays up so it can be retried"
        );
        assert!(!dialog(&app).downloading);
        assert_eq!(dialog(&app).error.as_deref(), Some("connection refused"));
        let line = app.log.back().expect("the failure is logged");
        assert!(line.is_error);
        assert!(line.text.contains("connection refused"));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Download);
    }

    #[test]
    fn unreadable_static_data_is_shown_with_the_reason_it_cannot_be_used() {
        let mut app = new_app(10);
        app.open_static_data(Some("data/regions.json is not valid JSON".to_string()));
        assert!(matches!(app.overlay, Overlay::StaticData(_)));
        assert_eq!(
            dialog(&app).error.as_deref(),
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
        assert_eq!(
            app.region(),
            "",
            "the region is not known until a run starts"
        );
        assert_eq!(app.corporations(), 0);
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
        assert_eq!(app.form.value(Field::Lpstore), "");
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
    fn starting_a_run_shows_no_summary() {
        // A summary belongs to the end of a run, so no start may leave one up. A re-run also has to
        // clear the summary the previous run left behind, which is the case set by hand below.
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.awaiting_run_summary = true;
        app.start_run(forge_run(3), "none".to_string(), 10);
        assert!(!app.awaiting_run_summary, "the old outcome is stale news");

        let mut fresh = App::new(LpStoreOptions::default());
        fresh.start_run(forge_run(3), "none".to_string(), 10);
        assert!(
            !fresh.awaiting_run_summary,
            "nothing was superseded, so there is nothing to acknowledge"
        );
    }

    #[test]
    fn finishing_a_re_run_leaves_the_summary_and_a_key_clears_it() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), 10);
        app.finish(report(Vec::new(), Vec::new()));
        assert!(app.awaiting_run_summary, "the run ended over a report");
        app.on_key(press(KeyCode::Esc));
        assert!(!app.awaiting_run_summary, "any key acknowledged it");
    }

    #[test]
    fn a_stopped_re_run_summarises_and_a_stopped_first_run_does_not() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), 10);
        app.fail();
        assert!(
            app.awaiting_run_summary,
            "stale results are still behind it"
        );
        let mut fresh = App::new(LpStoreOptions::default());
        fresh.start_run(forge_run(3), "none".to_string(), 10);
        fresh.fail();
        assert!(
            !fresh.awaiting_run_summary,
            "the progress view already shows the failure in red"
        );
    }

    #[test]
    fn a_newer_run_replaces_the_summary_it_was_given() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), 10);
        app.finish(report(Vec::new(), Vec::new()));
        assert!(app.awaiting_run_summary);
        app.start_run(forge_run(3), "none".to_string(), 10);
        assert!(
            !app.awaiting_run_summary,
            "the old run's outcome is not news about this one"
        );
    }

    #[test]
    fn a_new_run_keeps_the_old_results_stale_until_it_finishes() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), 10);
        assert!(app.stale);
        assert_eq!(app.stage, Stage::Starting);
        assert_eq!(app.groups().len(), 1, "the old results stay on screen");
        app.finish(report(vec![group(2, &[3.0])], Vec::new()));
        assert!(!app.stale);
        assert_eq!(app.groups()[0].corp_ids, vec![2]);
    }

    #[test]
    fn a_report_is_grouped_for_the_views_depth_when_it_arrives() {
        // The run was resolved at depth 1, where corp 1 averages 10 and sorts first, but the view
        // moved to depth 2 while it ran, where corp 2's 6 beats corp 1's 5.
        let mut app = new_app(2);
        app.finish(Report::new(
            &[],
            vec![group(1, &[10.0, 0.0]), group(2, &[6.0, 6.0])],
            Vec::new(),
            1,
        ));
        assert_eq!(app.groups()[0].corp_ids, vec![2]);
    }

    #[test]
    fn a_depth_change_that_joins_and_splits_groups_keeps_the_selected_corporation() {
        // Both list offer 1 first, at 9 ISK per LP, and differ below it.
        let store = |corp_id: i32, second: f64| {
            let mut best = offer(9.0);
            best.offer.offer_id = 1;
            let mut next = offer(second);
            next.offer.offer_id = corp_id * 100;
            Store::new(
                vec![corp_id],
                vec![1, corp_id * 100],
                vec![Arc::new(best), Arc::new(next)],
            )
        };
        let mut app = app_with(vec![store(1, 5.0), store(2, 8.0)], 2);
        assert_eq!(app.groups()[1].corp_ids, vec![1]);
        app.group = 1;
        app.apply_depth(1);
        assert_eq!(app.groups().len(), 1, "both list only offer 1");
        assert_eq!(app.groups()[0].corp_ids, vec![1, 2]);
        assert_eq!(app.group, 0);
        app.apply_depth(2);
        assert_eq!(app.groups()[1].corp_ids, vec![1]);
        assert_eq!(
            app.group, 1,
            "the group holding corporation 1 is selected again"
        );
    }

    #[test]
    fn the_keys_scroll_the_help_and_it_reopens_at_the_top() {
        let mut app = app_with(vec![group(1, &[5.0, 4.0])], 10);
        app.on_key(press(KeyCode::Char('?')));
        assert_eq!(app.overlay, Overlay::Help);
        // As the draw reports it at 80x24: 24 lines, 22 of them showing.
        app.set_help_layout(24, 22);
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.help_scroll, 1);
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.help_scroll, 2, "no further than the last line");
        app.on_key(press(KeyCode::Char('k')));
        assert_eq!(app.help_scroll, 1);
        app.on_key(press(KeyCode::Tab));
        assert_eq!(
            app.focus,
            Focus::Groups,
            "Tab does not move the panes behind it"
        );
        assert_eq!(app.offer, 0, "nor do the arrows");
        app.on_key(press(KeyCode::Char('?')));
        assert_eq!(app.overlay, Overlay::None);
        app.on_key(press(KeyCode::Char('?')));
        assert_eq!(app.help_scroll, 0, "back at the keys");
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

//! The state of the `--tui` view. Run events, key presses, and applied filters go in, and the
//! new state comes out. Nothing here touches the terminal, so these rules can be tested
//! directly.

use super::filter_summary;
use super::form::{FilterForm, FormKind};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eve_lpstore::exclusions::Excluded;
use eve_lpstore::export::ExportRequest;
use eve_lpstore::report::{Arrangement, Event, Group, LpStoreOptions, Report};
use eve_lpstore::static_data::AgentFilter;
use eve_lpstore::{OfferResult, StaticData};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

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
    /// The fetch form, opened with `f` and at startup.
    Filters,
    /// The display form, opened with `v`: the depth, the LP budget, and the thresholds.
    DisplayFilters,
    Donate,
    Export(ExportPrompt),
    /// Where the missions of the selected group's agent systems can take you.
    Destinations,
    /// Asking whether to download the static data, shown when it is not on disk.
    StaticData(StaticDataDialog),
    /// The search prompt, opened with `/`.
    Search(SearchPrompt),
}

/// The search prompt: the text typed into it, and why the last attempt found nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchPrompt {
    pub query: String,
    pub error: Option<String>,
}

/// The search last run, which `n` and `N` repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    /// Whether the last `n` or `N` found nothing. Any key clears it.
    pub missed: bool,
    /// The first corporation of the group whose corporation names the last hit was on, so the
    /// next search starts after that group's names rather than after its first offer.
    on_corporations: Option<i32>,
}

/// Which way a search steps from the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Next,
    Previous,
}

/// What the offers table is ordered by. `Rank` is the ranking order the groups are built from;
/// the others are the table's market columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OfferSortKey {
    #[default]
    Rank,
    Spread,
    Imbalance,
    Volume,
    Profit,
}

impl OfferSortKey {
    /// The next key in the cycle `o` steps through.
    pub fn next(self) -> Self {
        match self {
            OfferSortKey::Rank => OfferSortKey::Spread,
            OfferSortKey::Spread => OfferSortKey::Imbalance,
            OfferSortKey::Imbalance => OfferSortKey::Volume,
            OfferSortKey::Volume => OfferSortKey::Profit,
            OfferSortKey::Profit => OfferSortKey::Rank,
        }
    }

    /// The word the offers title names it by.
    pub fn name(self) -> &'static str {
        match self {
            OfferSortKey::Rank => "rank",
            OfferSortKey::Spread => "spread",
            OfferSortKey::Imbalance => "imbal",
            OfferSortKey::Volume => "vol",
            OfferSortKey::Profit => "profit",
        }
    }

    /// Whether the values rise down the table when it is not reversed: the rank, which counts
    /// from the best offer, and the spread, where the tightest market is the best. The others
    /// put the largest value first.
    fn ascending(self) -> bool {
        matches!(self, OfferSortKey::Rank | OfferSortKey::Spread)
    }

    /// The figure an offer is sorted by. `None` for the rank, which is the order itself, and
    /// for an offer that lacks the figure.
    fn figure(self, result: &OfferResult) -> Option<f64> {
        match self {
            OfferSortKey::Rank => None,
            OfferSortKey::Spread => result.spread,
            OfferSortKey::Imbalance => result.imbalance,
            OfferSortKey::Volume => result.within_10.map(|(volume, _)| volume as f64),
            OfferSortKey::Profit => result.within_10.map(|(_, profit)| profit),
        }
    }
}

/// How the offers table orders the offers a group lists. Display only: the groups, their
/// figures, and exports keep the ranking order. It lasts the session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OfferSort {
    pub key: OfferSortKey,
    pub reverse: bool,
}

impl OfferSort {
    /// `▲` when the sorted values rise down the table and `▼` when they fall, as the groups
    /// title uses them.
    pub fn arrow(self) -> char {
        if self.key.ascending() != self.reverse {
            '▲'
        } else {
            '▼'
        }
    }

    /// `results` in this order, each with its place in the ranking. Offers without the figure go
    /// last in both directions, and ties keep the ranking order.
    pub fn rows(self, results: &[Arc<OfferResult>]) -> Vec<(usize, &OfferResult)> {
        let ranked = results
            .iter()
            .enumerate()
            .map(|(rank, result)| (rank, &**result));
        if self.key == OfferSortKey::Rank {
            let mut rows: Vec<_> = ranked.collect();
            if self.reverse {
                rows.reverse();
            }
            return rows;
        }
        let mut with = Vec::new();
        let mut without = Vec::new();
        for (rank, result) in ranked {
            match self.key.figure(result) {
                Some(value) => with.push((value, rank, result)),
                None => without.push((rank, result)),
            }
        }
        let ascending = self.key.ascending() != self.reverse;
        // A stable sort, so equal figures stay in ranking order.
        with.sort_by(|(a, ..), (b, ..)| {
            let order = a.total_cmp(b);
            if ascending { order } else { order.reverse() }
        });
        with.into_iter()
            .map(|(_, rank, result)| (rank, result))
            .chain(without)
            .collect()
    }
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
    /// Apply the fetch form, which starts a run unless only display fields changed.
    Apply,
    /// Apply the display form to the report on hand, which never starts a run.
    Arrange,
    /// Write the report on screen to the file the export prompt names.
    Export,
    /// Download the static data archive after the user chose it in the dialog.
    Download,
    /// Run the search the prompt holds.
    Find,
    /// Repeat the last search forward, with `n`.
    FindNext,
    /// Repeat the last search backward, with `N`.
    FindPrevious,
    /// Write the excluded offers to their file, after `x` changed them.
    SaveExclusions,
}

/// What a run was resolved to, as the header and the detail pane show it. The filter summary and
/// the arrangement are handed to the view separately, because the display form changes them
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
    /// How the report is grouped and sorted. The depth, thresholds, and LP budget come from the
    /// options applied; the sort and hiding are the session's own, seeded by the first run.
    pub arrangement: Arrangement,
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
    /// How the offers table orders its rows, set with `o` and `O`.
    pub offer_sort: OfferSort,
    /// The search `n` and `N` repeat, once one has found something.
    pub search: Option<Search>,
    /// The offers excluded with `x`, with their reward types, as the file keeps them. The
    /// arrangement's `excluded` set is a copy of these IDs, made whenever this changes.
    pub excluded: Excluded,
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
            arrangement: Arrangement::default(),
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
            offer_sort: OfferSort::default(),
            search: None,
            excluded: Excluded::new(),
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
    /// is arranged for the view's arrangement, which a display change or a sort key during the run
    /// may have moved away from the run's own.
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
        report.arrange(&self.arrangement);
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

    /// Takes in what a run was resolved to, without moving the stage on or logging it. The first
    /// run seeds the whole arrangement, including the sort `--sort` asked for; later runs take
    /// only the depth, thresholds, and LP budget, so the sort and hiding chosen with `s`, `S`, and
    /// `F` last the session.
    pub(crate) fn begin_run(&mut self, info: RunInfo, filters: String, arrangement: Arrangement) {
        let first = self.run.is_none();
        self.run = Some(info);
        self.filters = filters;
        self.rearrange(|current| {
            if first {
                // The exclusions are the session's, loaded before any run, and the run's own
                // arrangement excludes nothing.
                let excluded = current.excluded.clone();
                *current = arrangement;
                current.excluded = excluded;
            } else {
                take_display_settings(current, &arrangement);
            }
        });
    }

    /// Switches the view to a new run. Earlier results stay on screen, marked stale, until
    /// this run finishes.
    pub fn start_run(&mut self, info: RunInfo, filters: String, arrangement: Arrangement) {
        self.begin_run(info, filters, arrangement);
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

    /// Changes how the report is arranged. The run itself is unchanged, so the groups are rebuilt
    /// from the offers already priced, which can join or split them; a change of sort or direction
    /// alone only sorts them again. The group holding the selected group's first corporation stays
    /// selected, and so does the selected offer while that group still lists it. The group count
    /// the finished stage reports follows the new groups.
    pub fn rearrange(&mut self, change: impl FnOnce(&mut Arrangement)) {
        let corporation = self.selected_group().map(|group| group.corp_ids[0]);
        let offer_id = self.selected_offer().map(|result| result.offer.offer_id);
        change(&mut self.arrangement);
        if let Some(report) = &mut self.report {
            let wanted = &self.arrangement;
            let current = report.arrangement();
            let resorted = Arrangement {
                sort: wanted.sort,
                reverse: wanted.reverse,
                ..current.clone()
            };
            if resorted != *wanted {
                report.arrange(wanted);
            } else if current != wanted {
                report.sort_groups(wanted.sort, wanted.reverse);
            }
            if let Some(index) = corporation.and_then(|corporation| {
                report
                    .groups
                    .iter()
                    .position(|group| group.corp_ids.contains(&corporation))
            }) {
                self.group = index;
            }
            self.group = self.group.min(report.groups.len().saturating_sub(1));
            if let Stage::Finished { groups, .. } = &mut self.stage {
                *groups = report.groups.len();
            }
        }
        self.keep_offer(offer_id);
    }

    /// Starts the session with `excluded`, as loaded from the file, without saving anything.
    pub fn set_excluded(&mut self, excluded: Excluded) {
        self.excluded = excluded;
        let ids = self.excluded_ids();
        self.rearrange(|arrangement| arrangement.excluded = ids);
    }

    /// The excluded offer IDs, as the arrangement holds them.
    fn excluded_ids(&self) -> Arc<HashSet<i32>> {
        Arc::new(self.excluded.keys().copied().collect())
    }

    /// Excludes the selected offer, or restores it when it is excluded, which can only be the case
    /// with hiding off since an excluded offer is not listed otherwise. The report is re-arranged
    /// with the corporation kept selected, and the caller is asked to save the change. Nothing
    /// happens without a report on screen, under an overlay, or with no offer selected.
    fn toggle_exclusion(&mut self) -> Action {
        if !self.arranging() {
            return Action::Continue;
        }
        let Some((offer_id, type_id)) = self
            .selected_offer()
            .map(|result| (result.offer.offer_id, result.offer.type_id))
        else {
            return Action::Continue;
        };
        if self.excluded.remove(&offer_id).is_none() {
            self.excluded.insert(offer_id, type_id);
        }
        let ids = self.excluded_ids();
        self.rearrange(|arrangement| arrangement.excluded = ids);
        Action::SaveExclusions
    }

    /// Changes the order of the offers table. The groups and the report are untouched, and the
    /// selected offer stays selected.
    pub fn sort_offers(&mut self, change: impl FnOnce(&mut OfferSort)) {
        let offer_id = self.selected_offer().map(|result| result.offer.offer_id);
        change(&mut self.offer_sort);
        self.keep_offer(offer_id);
    }

    /// Selects `offer_id` in the offers table if the selected group still lists it, and otherwise
    /// keeps the selection's place within the rows there are.
    fn keep_offer(&mut self, offer_id: Option<i32>) {
        let rows = self.offer_rows();
        let kept = offer_id.and_then(|offer_id| {
            rows.iter()
                .position(|(_, result)| result.offer.offer_id == offer_id)
        });
        let listed = rows.len();
        self.offer = kept.unwrap_or_else(|| self.offer.min(listed.saturating_sub(1)));
    }

    pub fn open_form(&mut self) {
        self.form = FilterForm::from_options(&self.applied);
        self.overlay = Overlay::Filters;
    }

    /// Opens the display form on the options applied, with the depth and LP budget in force.
    pub fn open_display_form(&mut self) {
        self.form = FilterForm::display(&self.applied);
        self.overlay = Overlay::DisplayFilters;
    }

    pub fn close_form(&mut self) {
        if matches!(self.overlay, Overlay::Filters | Overlay::DisplayFilters) {
            self.overlay = Overlay::None;
        }
    }

    /// Applies the display form: checks its values, and if they are all valid re-arranges the
    /// report on hand for them, takes them into the options applied and the header's filters, and
    /// closes the form. Nothing is fetched. Returns whether it applied, so the caller can remember
    /// the options; when a value is invalid the errors are shown beside their fields instead.
    pub fn apply_display_form(&mut self) -> bool {
        let options = self.form.to_options(&self.applied);
        let parsed = match options.parse() {
            Ok(parsed) => parsed,
            Err(errors) => {
                self.form.set_errors(errors);
                return false;
            }
        };
        let settings = Arrangement {
            filter: parsed.filter,
            lp_budget: Some(parsed.lp_budget),
            ..Arrangement::depth(parsed.depth)
        };
        self.rearrange(|current| take_display_settings(current, &settings));
        self.filters = filter_summary(&options);
        self.applied = options;
        self.close_form();
        true
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

    /// The offers the offers pane lists for the selected group, in the order it shows them, each
    /// with its place in the ranking. A group's results are already the top `depth` offers the
    /// arrangement lets through, since the report is arranged for the view's arrangement.
    pub fn offer_rows(&self) -> Vec<(usize, &OfferResult)> {
        self.selected_group()
            .map_or_else(Vec::new, |group| self.offer_sort.rows(&group.results))
    }

    /// The offer under the selection.
    pub fn selected_offer(&self) -> Option<&OfferResult> {
        self.offer_rows().get(self.offer).map(|&(_, result)| result)
    }

    /// The number of offers the offers pane lists for the selected group.
    pub fn offer_count(&self) -> usize {
        self.offer_rows().len()
    }

    /// Runs the search the prompt holds, or with the prompt closed the last one, from the
    /// selection in `direction`, matching the names the panes show.
    pub fn find(&mut self, direction: Direction, sdata: &StaticData) {
        self.find_with(
            direction,
            &|corp_id| super::ui::corp_name(sdata, corp_id),
            &|type_id| super::ui::item_name(sdata, type_id),
        );
    }

    /// [`App::find`] with the names looked up by `corp_name` and `item_name`.
    ///
    /// The places searched are, group by group in the order shown, the group's corporation names
    /// and then the item of each offer it lists, in the table's order. The search starts after the
    /// selection and wraps around, so the selection itself is tried last. Matching ignores case.
    /// A hit selects its group, and its offer when it is an item, and scrolls the details to the
    /// top. A miss leaves the selection alone: from the prompt it says `No match` there, and from
    /// `n` or `N` it marks the search as missed.
    fn find_with(
        &mut self,
        direction: Direction,
        corp_name: &dyn Fn(i32) -> String,
        item_name: &dyn Fn(i32) -> String,
    ) {
        let (query, from_prompt) = match (&self.overlay, &self.search) {
            (Overlay::Search(prompt), _) => (prompt.query.clone(), true),
            (_, Some(search)) => (search.query.clone(), false),
            _ => return,
        };
        let hit = self.next_match(&query.to_lowercase(), direction, corp_name, item_name);
        match hit {
            Some((group, offer)) => {
                self.group = group;
                self.offer = offer.unwrap_or(0);
                self.detail_scroll = 0;
                let on_corporations = match offer {
                    None => self.selected_group().map(|group| group.corp_ids[0]),
                    Some(_) => None,
                };
                self.search = Some(Search {
                    query,
                    missed: false,
                    on_corporations,
                });
                if from_prompt {
                    self.overlay = Overlay::None;
                }
            }
            None => match (&mut self.overlay, &mut self.search) {
                (Overlay::Search(prompt), _) => prompt.error = Some("No match".to_string()),
                (_, Some(search)) => search.missed = true,
                _ => {}
            },
        }
    }

    /// The next place after the selection whose name contains `needle`, as a group and, for an
    /// item, its row in the table. `None` for the group's corporation names.
    fn next_match(
        &self,
        needle: &str,
        direction: Direction,
        corp_name: &dyn Fn(i32) -> String,
        item_name: &dyn Fn(i32) -> String,
    ) -> Option<(usize, Option<usize>)> {
        let groups = self.groups();
        let mut places: Vec<(usize, Option<usize>, Option<i32>)> = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            places.push((index, None, None));
            for (row, (_, result)) in self.offer_sort.rows(&group.results).iter().enumerate() {
                places.push((index, Some(row), Some(result.offer.type_id)));
            }
        }
        if places.is_empty() || needle.is_empty() {
            return None;
        }
        let on_corporations = self
            .search
            .as_ref()
            .and_then(|search| search.on_corporations);
        let at_names = self.offer == 0
            && on_corporations.is_some()
            && self.selected_group().map(|group| group.corp_ids[0]) == on_corporations;
        let here = if at_names || self.offer_count() == 0 {
            (self.group, None)
        } else {
            (self.group, Some(self.offer))
        };
        let start = places
            .iter()
            .position(|&(group, row, _)| (group, row) == here)
            .unwrap_or(0);
        let count = places.len();
        let matches = |&(group, _, type_id): &(usize, Option<usize>, Option<i32>)| match type_id {
            Some(type_id) => item_name(type_id).to_lowercase().contains(needle),
            None => groups[group]
                .corp_ids
                .iter()
                .any(|&corp_id| corp_name(corp_id).to_lowercase().contains(needle)),
        };
        (1..=count)
            .map(|step| match direction {
                Direction::Next => (start + step) % count,
                Direction::Previous => (start + count - step % count) % count,
            })
            .map(|index| places[index])
            .find(|place| matches(place))
            .map(|(group, row, _)| (group, row))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.kind != KeyEventKind::Press {
            return Action::Continue;
        }
        // Any key acknowledges a run summary. It is not a mode and takes no key for itself: what the
        // press goes on to do — move a selection, open the log, quit — is unchanged.
        self.awaiting_run_summary = false;
        // A search that found nothing says so until the next key, like the summary.
        if let Some(search) = &mut self.search {
            search.missed = false;
        }
        // In order: the text prompts take every key, so `q` types a letter there; everywhere
        // else `q` and Ctrl+C quit before the overlay sees the key.
        match &self.overlay {
            Overlay::Filters | Overlay::DisplayFilters => self.on_form_key(key),
            Overlay::Export(_) => self.on_export_key(key),
            Overlay::Search(_) => self.on_search_key(key),
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
            KeyCode::Char('v') => self.open_display_form(),
            // The three arrangement keys act on the results on screen, so they wait for a report
            // and are left alone while an overlay covers it. Shift+s arrives as `S`, whatever the
            // terminal reports for the modifier.
            KeyCode::Char('s') if self.arranging() => {
                self.rearrange(|arrangement| arrangement.sort = arrangement.sort.next());
            }
            KeyCode::Char('S') if self.arranging() => {
                self.rearrange(|arrangement| arrangement.reverse = !arrangement.reverse);
            }
            KeyCode::Char('F') if self.arranging() => {
                self.rearrange(|arrangement| arrangement.hiding = !arrangement.hiding);
            }
            KeyCode::Char('x') => return self.toggle_exclusion(),
            // The offer sort and the search act on the same results, under the same rule.
            KeyCode::Char('o') if self.arranging() => {
                self.sort_offers(|sort| sort.key = sort.key.next());
            }
            KeyCode::Char('O') if self.arranging() => {
                self.sort_offers(|sort| sort.reverse = !sort.reverse);
            }
            KeyCode::Char('/') if self.arranging() => {
                self.overlay = Overlay::Search(SearchPrompt::default());
            }
            KeyCode::Char('n') if self.arranging() && self.search.is_some() => {
                return Action::FindNext;
            }
            KeyCode::Char('N') if self.arranging() && self.search.is_some() => {
                return Action::FindPrevious;
            }
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
            KeyCode::Enter => {
                return match self.form.kind {
                    FormKind::Fetch => Action::Apply,
                    FormKind::Display => Action::Arrange,
                };
            }
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

    /// While the search prompt is open, keys go to it, so `q` types a letter. Only Ctrl+C quits.
    fn on_search_key(&mut self, key: KeyEvent) -> Action {
        if is_ctrl_c(&key) {
            return Action::Quit;
        }
        if key.code == KeyCode::Esc {
            self.overlay = Overlay::None;
            return Action::Continue;
        }
        let Overlay::Search(prompt) = &mut self.overlay else {
            return Action::Continue;
        };
        if let Some(edit) = Edit::of(&key) {
            edit.apply(&mut prompt.query);
            prompt.error = None;
        } else if key.code == KeyCode::Enter {
            // An empty query matches everything, which is no search; Enter on it closes the prompt.
            if prompt.query.is_empty() {
                self.overlay = Overlay::None;
                return Action::Continue;
            }
            return Action::Find;
        }
        Action::Continue
    }

    /// Whether the sort and hiding keys act: a report is on screen with nothing over it.
    fn arranging(&self) -> bool {
        self.report.is_some() && self.overlay == Overlay::None
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

/// Copies the settings the display form holds, the depth, the thresholds, and the LP budget, from
/// `from` into `target`, leaving the sort, the direction, hiding, and the exclusions as they are.
fn take_display_settings(target: &mut Arrangement, from: &Arrangement) {
    target.depth = from.depth;
    target.filter = from.filter;
    target.lp_budget = from.lp_budget;
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
    use crate::tui::test_support::{
        dialog, forge_run, open_prompt, open_prompt_mut, press, search_prompt,
    };
    use crossterm::event::KeyEventState;
    use eve_lpstore::report::{Field, GroupSort, Store};
    use eve_lpstore::{OfferError, PricingError};
    use std::sync::Arc;

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn new_app(depth: usize) -> App {
        let mut app = App::new(LpStoreOptions::default());
        app.begin_run(forge_run(3), "none".to_string(), Arrangement::depth(depth));
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
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
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
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        assert!(!app.awaiting_run_summary, "the old outcome is stale news");

        let mut fresh = App::new(LpStoreOptions::default());
        fresh.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        assert!(
            !fresh.awaiting_run_summary,
            "nothing was superseded, so there is nothing to acknowledge"
        );
    }

    #[test]
    fn finishing_a_re_run_leaves_the_summary_and_a_key_clears_it() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        app.finish(report(Vec::new(), Vec::new()));
        assert!(app.awaiting_run_summary, "the run ended over a report");
        app.on_key(press(KeyCode::Esc));
        assert!(!app.awaiting_run_summary, "any key acknowledged it");
    }

    #[test]
    fn a_stopped_re_run_summarises_and_a_stopped_first_run_does_not() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        app.fail();
        assert!(
            app.awaiting_run_summary,
            "stale results are still behind it"
        );
        let mut fresh = App::new(LpStoreOptions::default());
        fresh.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        fresh.fail();
        assert!(
            !fresh.awaiting_run_summary,
            "the progress view already shows the failure in red"
        );
    }

    #[test]
    fn a_newer_run_replaces_the_summary_it_was_given() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        app.finish(report(Vec::new(), Vec::new()));
        assert!(app.awaiting_run_summary);
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
        assert!(
            !app.awaiting_run_summary,
            "the old run's outcome is not news about this one"
        );
    }

    #[test]
    fn a_new_run_keeps_the_old_results_stale_until_it_finishes() {
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.start_run(forge_run(3), "none".to_string(), Arrangement::depth(10));
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
        app.rearrange(|arrangement| arrangement.depth = 1);
        assert_eq!(app.groups().len(), 1, "both list only offer 1");
        assert_eq!(app.groups()[0].corp_ids, vec![1, 2]);
        assert_eq!(app.group, 0);
        app.rearrange(|arrangement| arrangement.depth = 2);
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
        // As the draw reports it at 80x24: 34 lines, 22 of them showing.
        app.set_help_layout(34, 22);
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.help_scroll, 1);
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.help_scroll, 11);
        app.on_key(press(KeyCode::PageDown));
        assert_eq!(app.help_scroll, 12, "no further than the last line");
        app.on_key(press(KeyCode::Char('k')));
        assert_eq!(app.help_scroll, 11);
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
        app.rearrange(|arrangement| arrangement.depth = 2);
        assert_eq!(app.offer, 1);
        assert_eq!(app.offer_count(), 2);
    }

    #[test]
    fn a_depth_change_reorders_and_keeps_the_group_selected() {
        // Depth 1 averages 10 for corp 1 and 6 for corp 2, so corp 1 sorts first. Depth 2
        // averages 5 for corp 1 and 6 for corp 2, so the order flips.
        let mut app = app_with(vec![group(1, &[10.0, 0.0]), group(2, &[6.0, 6.0])], 1);
        app.rearrange(|arrangement| arrangement.depth = 1);
        assert_eq!(app.groups()[0].corp_ids, vec![1]);
        assert_eq!(app.group, 0);

        app.rearrange(|arrangement| arrangement.depth = 2);
        assert_eq!(app.groups()[0].corp_ids, vec![2]);
        assert_eq!(app.groups()[1].corp_ids, vec![1]);
        assert_eq!(
            app.group, 1,
            "the same group stays selected after the reorder"
        );
    }

    fn char_key(c: char) -> KeyEvent {
        press(KeyCode::Char(c))
    }

    /// The first corporation of each group, in order.
    fn first_corporations(app: &App) -> Vec<i32> {
        app.groups().iter().map(|group| group.corp_ids[0]).collect()
    }

    /// The first corporation of the selected group.
    fn selected_corporation(app: &App) -> i32 {
        app.selected_group().expect("a group is selected").corp_ids[0]
    }

    #[test]
    fn s_cycles_the_group_sort_and_keeps_the_selected_corporation_and_offer() {
        // By the average, corp 2 (6) goes before corp 1 (5). No offer has buy orders or a profit
        // figure, so the budget and profit sorts tie and fall back to the corporation ID, and by
        // the best offer corp 1's 10 leads.
        let mut app = app_with(vec![group(1, &[10.0, 0.0]), group(2, &[6.0, 6.0])], 2);
        assert_eq!(first_corporations(&app), [2, 1]);
        app.group = 1;
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.selected_offer().map(|o| o.offer.offer_id), Some(101));
        for (sort, order, group) in [
            (GroupSort::BudgetIsk, [1, 2], 0),
            (GroupSort::BestOffer, [1, 2], 0),
            (GroupSort::ProfitWithin10, [1, 2], 0),
            (GroupSort::AverageIskPerLp, [2, 1], 1),
        ] {
            assert_eq!(app.on_key(char_key('s')), Action::Continue);
            assert_eq!(app.arrangement.sort, sort);
            assert_eq!(first_corporations(&app), order, "{sort:?}");
            assert_eq!(app.group, group, "{sort:?}");
            assert_eq!(selected_corporation(&app), 1, "{sort:?}");
            assert_eq!(
                app.selected_offer().map(|o| o.offer.offer_id),
                Some(101),
                "the offer stays selected under {sort:?}"
            );
        }
        assert_eq!(
            app.stage,
            Stage::Finished {
                offers: 4,
                groups: 2
            }
        );
    }

    #[test]
    fn shift_s_reverses_the_group_sort_and_back() {
        let mut app = app_with(vec![group(1, &[10.0, 0.0]), group(2, &[6.0, 6.0])], 2);
        let shifted = KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT);
        app.on_key(shifted);
        assert!(app.arrangement.reverse);
        assert_eq!(first_corporations(&app), [1, 2], "the worst average first");
        assert_eq!(
            selected_corporation(&app),
            2,
            "the selection followed corp 2"
        );
        app.on_key(char_key('S'));
        assert!(!app.arrangement.reverse);
        assert_eq!(first_corporations(&app), [2, 1]);
        assert_eq!(app.group, 0);
    }

    /// Two stores that list offer 1 first, at 9 ISK per LP, and differ below it: corp 1 at 5 and
    /// corp 2 at 8. Offer 1 rewards type 34, and the second offer of corp N type 1000 + N.
    fn sharing_stores() -> Vec<Store> {
        let store = |corp_id: i32, second: f64| {
            let mut best = offer(9.0);
            best.offer.offer_id = 1;
            best.offer.type_id = 34;
            let mut next = offer(second);
            next.offer.offer_id = corp_id * 100;
            next.offer.type_id = 1000 + corp_id;
            Store::new(
                vec![corp_id],
                vec![1, corp_id * 100],
                vec![Arc::new(best), Arc::new(next)],
            )
        };
        vec![store(1, 5.0), store(2, 8.0)]
    }

    #[test]
    fn f_turns_hiding_off_and_on_regrouping_and_keeping_the_selection() {
        let mut app = app_with(sharing_stores(), 2);
        app.rearrange(|arrangement| arrangement.filter.min_isk_per_lp = Some(9.0));
        assert_eq!(app.groups().len(), 1, "both list only offer 1");
        assert_eq!(app.groups()[0].corp_ids, [1, 2]);
        assert_eq!(
            app.stage,
            Stage::Finished {
                offers: 3,
                groups: 1
            },
            "the stage counts the groups on screen"
        );
        app.on_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
        assert!(!app.arrangement.hiding);
        assert_eq!(
            first_corporations(&app),
            [2, 1],
            "split, best average first"
        );
        assert_eq!(app.group, 1, "corporation 1 stays selected");
        assert_eq!(
            app.stage,
            Stage::Finished {
                offers: 3,
                groups: 2
            }
        );
        app.on_key(char_key('F'));
        assert!(app.arrangement.hiding);
        assert_eq!(app.groups().len(), 1);
        assert_eq!(app.group, 0);
    }

    #[test]
    fn the_arrangement_keys_do_nothing_before_a_report_or_under_an_overlay() {
        let mut app = new_app(10);
        let before = app.arrangement.clone();
        for c in ['s', 'S', 'F'] {
            assert_eq!(app.on_key(char_key(c)), Action::Continue);
        }
        assert_eq!(app.arrangement, before, "nothing to arrange yet");
        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.on_key(char_key('?'));
        app.on_key(char_key('s'));
        app.on_key(char_key('F'));
        assert_eq!(app.arrangement, before, "the help covers the results");
        assert_eq!(app.overlay, Overlay::Help);
    }

    #[test]
    fn v_opens_the_display_form_and_enter_arranges_without_a_run() {
        let mut app = app_with(sharing_stores(), 2);
        let stage = app.stage;
        app.on_key(char_key('v'));
        assert_eq!(app.overlay, Overlay::DisplayFilters);
        assert_eq!(app.form.kind, FormKind::Display);
        assert_eq!(app.form.value(Field::Depth), "10", "the default in force");
        app.form.focus = crate::tui::form::FormField::Text(Field::Depth);
        app.on_key(ctrl('u'));
        app.on_key(char_key('1'));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Arrange);
        assert_eq!(
            app.overlay,
            Overlay::DisplayFilters,
            "the loop applies it, not the key"
        );
        assert!(app.apply_display_form());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.arrangement.depth, 1);
        assert_eq!(app.groups().len(), 1, "both list only offer 1 at depth 1");
        assert_eq!(
            app.stage,
            Stage::Finished {
                offers: 3,
                groups: 1
            }
        );
        assert_ne!(stage, app.stage, "only the group count moved");
        assert!(!app.stale, "no run superseded the report");
        assert_eq!(app.applied.depth.as_deref(), Some("1"));
        // The fetch form's Enter is still a run.
        app.on_key(char_key('f'));
        assert_eq!(app.on_key(press(KeyCode::Enter)), Action::Apply);
    }

    #[test]
    fn the_first_run_seeds_the_sort_and_later_runs_keep_the_sessions() {
        let mut app = App::new(LpStoreOptions::default());
        let seeded = Arrangement {
            sort: GroupSort::BestOffer,
            ..Arrangement::depth(3)
        };
        app.start_run(forge_run(1), "none".to_string(), seeded);
        assert_eq!(app.arrangement.sort, GroupSort::BestOffer, "--sort applies");
        assert_eq!(app.arrangement.depth, 3);
        app.finish(report(vec![group(1, &[5.0]), group(2, &[6.0])], Vec::new()));
        assert_eq!(app.report.as_ref().unwrap().arrangement(), &app.arrangement);
        app.on_key(char_key('s'));
        app.on_key(char_key('S'));
        app.on_key(char_key('F'));
        let next = Arrangement {
            filter: eve_lpstore::report::OfferFilter {
                max_spread: Some(5.0),
                ..Default::default()
            },
            ..Arrangement::depth(4)
        };
        app.start_run(forge_run(1), "none".to_string(), next);
        assert_eq!(app.arrangement.sort, GroupSort::ProfitWithin10, "s lasts");
        assert!(app.arrangement.reverse, "S lasts");
        assert!(!app.arrangement.hiding, "F lasts");
        assert_eq!(app.arrangement.depth, 4, "the run's depth is taken");
        assert_eq!(app.arrangement.filter.max_spread, Some(5.0));
    }

    /// The selected offer's ID.
    fn selected_offer_id(app: &App) -> Option<i32> {
        app.selected_offer().map(|result| result.offer.offer_id)
    }

    /// [`sharing_stores`] at depth 2 with corporation 1's offer 100 selected in the offers table.
    /// By the average corp 2 (8.5) is listed before corp 1 (7).
    fn on_offer_100() -> App {
        let mut app = app_with(sharing_stores(), 2);
        assert_eq!(first_corporations(&app), [2, 1]);
        app.group = 1;
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        assert_eq!(selected_offer_id(&app), Some(100));
        app
    }

    #[test]
    fn x_excludes_the_selected_offer_and_keeps_the_corporation_selected() {
        let mut app = on_offer_100();
        assert_eq!(app.on_key(char_key('x')), Action::SaveExclusions);
        assert_eq!(app.excluded, Excluded::from([(100, 1001)]), "with its type");
        assert_eq!(*app.arrangement.excluded, HashSet::from([100]));
        assert_eq!(
            app.report.as_ref().unwrap().arrangement(),
            &app.arrangement,
            "the report was re-arranged"
        );
        // Corp 1 now lists only offer 1, an average of 9, and moves first.
        assert_eq!(first_corporations(&app), [1, 2]);
        assert_eq!(
            selected_corporation(&app),
            1,
            "the corporation stays selected"
        );
        assert_eq!(app.offer_count(), 1);
        assert_eq!(
            selected_offer_id(&app),
            Some(1),
            "the selection stays in range"
        );
        assert_eq!(app.groups()[0].hidden_counts, [1]);
        assert_eq!(app.focus, Focus::Offers);
    }

    #[test]
    fn with_hiding_off_x_restores_an_excluded_offer() {
        let mut app = on_offer_100();
        app.on_key(char_key('x'));
        app.on_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
        assert!(!app.arrangement.hiding);
        assert_eq!(selected_corporation(&app), 1);
        assert_eq!(app.offer_count(), 2, "the excluded offer is listed again");
        app.on_key(press(KeyCode::Down));
        assert_eq!(selected_offer_id(&app), Some(100));
        assert_eq!(app.on_key(char_key('x')), Action::SaveExclusions);
        assert!(app.excluded.is_empty(), "restored");
        assert!(app.arrangement.excluded.is_empty());
        assert_eq!(selected_offer_id(&app), Some(100), "and still selected");
        // With hiding off, excluding an offer leaves it listed and selected.
        assert_eq!(app.on_key(char_key('x')), Action::SaveExclusions);
        assert_eq!(app.excluded, Excluded::from([(100, 1001)]));
        assert_eq!(selected_offer_id(&app), Some(100));
        app.on_key(char_key('x'));
        app.on_key(char_key('F'));
        assert!(app.arrangement.hiding);
        assert_eq!(
            app.offer_count(),
            2,
            "nothing is hidden once it is restored"
        );
        assert_eq!(first_corporations(&app), [2, 1]);
    }

    #[test]
    fn x_does_nothing_without_a_report_or_a_selected_offer_or_under_an_overlay() {
        let mut app = new_app(10);
        assert_eq!(app.on_key(char_key('x')), Action::Continue);
        assert!(app.excluded.is_empty(), "no report yet");

        let mut app = app_with(vec![group(1, &[5.0])], 10);
        app.rearrange(|arrangement| arrangement.filter.min_isk_per_lp = Some(100.0));
        assert_eq!(app.offer_count(), 0);
        assert_eq!(app.on_key(char_key('x')), Action::Continue);
        assert!(app.excluded.is_empty(), "no offer is selected");

        let mut app = app_with(vec![group(1, &[5.0])], 10);
        let before = app.arrangement.clone();
        for overlay in ['?', 'l', 'm', 'd'] {
            app.on_key(char_key(overlay));
            assert_ne!(app.overlay, Overlay::None);
            assert_eq!(
                app.on_key(char_key('x')),
                Action::Continue,
                "under {overlay}"
            );
            app.on_key(press(KeyCode::Esc));
        }
        assert!(app.excluded.is_empty());
        assert_eq!(app.arrangement, before);
        // The forms and the prompts take it as a letter.
        app.on_key(char_key('/'));
        assert_eq!(app.on_key(char_key('x')), Action::Continue);
        assert_eq!(search_prompt(&app).query, "x");
        assert!(app.excluded.is_empty());
    }

    #[test]
    fn a_run_started_after_excluding_keeps_the_exclusion() {
        // The session loads its exclusions before the first run, whose own arrangement has none.
        let mut app = App::new(LpStoreOptions::default());
        app.set_excluded(Excluded::from([(100, 1001)]));
        app.start_run(forge_run(2), "none".to_string(), Arrangement::depth(2));
        assert_eq!(*app.arrangement.excluded, HashSet::from([100]));
        app.finish(report(sharing_stores(), Vec::new()));
        assert_eq!(
            first_corporations(&app),
            [1, 2],
            "corp 1 lists only offer 1"
        );
        assert_eq!(app.groups()[0].hidden_counts, [1]);
        // An offer excluded during the session lasts into the next run too.
        app.group = 1;
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        assert_eq!(selected_offer_id(&app), Some(200));
        app.on_key(char_key('x'));
        app.start_run(forge_run(2), "none".to_string(), Arrangement::depth(2));
        app.finish(report(sharing_stores(), Vec::new()));
        assert_eq!(app.excluded, Excluded::from([(100, 1001), (200, 1002)]));
        assert_eq!(*app.arrangement.excluded, HashSet::from([100, 200]));
        assert_eq!(app.report.as_ref().unwrap().arrangement(), &app.arrangement);
        assert_eq!(app.groups().len(), 1, "both list only offer 1");
    }

    /// An offer of `type_id` with these market figures, ranked by `isk_per_lp`.
    fn market_offer(
        offer_id: i32,
        isk_per_lp: f64,
        spread: Option<f64>,
        imbalance: Option<f64>,
        within_10: Option<(i64, f64)>,
    ) -> OfferResult {
        let mut result = offer(isk_per_lp);
        result.offer.offer_id = offer_id;
        result.spread = spread;
        result.imbalance = imbalance;
        result.within_10 = within_10;
        result
    }

    /// One store, corporation 1, whose offers 100 to 103 rank in that order. Offer 101 has no
    /// market figures, and offers 100 and 103 tie on every one.
    fn market_app() -> App {
        let results = vec![
            market_offer(100, 10.0, Some(30.0), Some(10.0), Some((5, 50.0))),
            market_offer(101, 9.0, None, None, None),
            market_offer(102, 8.0, Some(5.0), Some(-20.0), Some((50, 500.0))),
            market_offer(103, 7.0, Some(30.0), Some(10.0), Some((5, 50.0))),
        ];
        let offer_ids = results.iter().map(|result| result.offer.offer_id).collect();
        let results = results.into_iter().map(Arc::new).collect();
        app_with(vec![Store::new(vec![1], offer_ids, results)], 10)
    }

    /// The ranking place of each row of the offers table, in the order it shows them.
    fn ranks(app: &App) -> Vec<usize> {
        app.offer_rows().iter().map(|&(rank, _)| rank).collect()
    }

    #[test]
    fn o_cycles_the_offer_sort_and_keeps_the_selected_offer_and_its_rank() {
        let mut app = market_app();
        assert_eq!(ranks(&app), [0, 1, 2, 3], "the ranking to begin with");
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        app.on_key(press(KeyCode::Down));
        assert_eq!(app.selected_offer().map(|o| o.offer.offer_id), Some(102));
        let report_order = |app: &App| -> Vec<i32> {
            app.selected_group()
                .unwrap()
                .results
                .iter()
                .map(|r| r.offer.offer_id)
                .collect()
        };
        for (key, order, row) in [
            // The tightest spread first, ties in ranking order, no spread last.
            (OfferSortKey::Spread, [2, 0, 3, 1], 0),
            (OfferSortKey::Imbalance, [0, 3, 2, 1], 2),
            (OfferSortKey::Volume, [2, 0, 3, 1], 0),
            (OfferSortKey::Profit, [2, 0, 3, 1], 0),
            (OfferSortKey::Rank, [0, 1, 2, 3], 2),
        ] {
            assert_eq!(app.on_key(char_key('o')), Action::Continue);
            assert_eq!(app.offer_sort.key, key);
            assert_eq!(ranks(&app), order, "{key:?}");
            assert_eq!(app.offer, row, "{key:?}");
            assert_eq!(
                app.selected_offer().map(|o| o.offer.offer_id),
                Some(102),
                "the offer stays selected under {key:?}"
            );
            assert_eq!(app.offer_rows()[row].0, 2, "its # stays 3 under {key:?}");
            assert_eq!(
                report_order(&app),
                [100, 101, 102, 103],
                "the group is untouched"
            );
        }
    }

    #[test]
    fn shift_o_reverses_the_offer_sort_with_missing_figures_still_last() {
        let mut app = market_app();
        app.on_key(char_key('o'));
        app.on_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT));
        assert!(app.offer_sort.reverse);
        assert_eq!(
            ranks(&app),
            [0, 3, 2, 1],
            "widest spread first, ties in ranking order"
        );
        app.on_key(char_key('o'));
        assert_eq!(app.offer_sort.key, OfferSortKey::Imbalance);
        assert!(app.offer_sort.reverse, "the direction lasts, as with s");
        assert_eq!(ranks(&app), [2, 0, 3, 1]);
        app.on_key(char_key('o'));
        app.on_key(char_key('o'));
        app.on_key(char_key('o'));
        assert_eq!(app.offer_sort.key, OfferSortKey::Rank);
        assert_eq!(ranks(&app), [3, 2, 1, 0], "the ranking upside down");
        app.on_key(char_key('O'));
        assert_eq!(app.offer_sort, OfferSort::default());
        assert_eq!(ranks(&app), [0, 1, 2, 3]);
    }

    #[test]
    fn the_offer_sort_keys_do_nothing_before_a_report_or_under_an_overlay() {
        let mut app = new_app(10);
        app.on_key(char_key('o'));
        app.on_key(char_key('/'));
        assert_eq!(app.offer_sort, OfferSort::default());
        assert_eq!(app.overlay, Overlay::None);
        let mut app = market_app();
        app.on_key(char_key('?'));
        app.on_key(char_key('o'));
        app.on_key(char_key('/'));
        assert_eq!(app.offer_sort, OfferSort::default());
        assert_eq!(app.overlay, Overlay::Help);
    }

    /// Corporation 1, "Caldari Navy", lists Tritanium and Pyerite; corporation 2, "Sisters of
    /// EVE", lists Mexallon and Tritanium. Corporation 1's average is the better, so its group is
    /// first.
    fn named_app() -> App {
        let store = |corp_id: i32, offers: [(f64, i32); 2]| {
            let results: Vec<Arc<OfferResult>> = offers
                .iter()
                .enumerate()
                .map(|(index, &(isk_per_lp, type_id))| {
                    let mut result = offer(isk_per_lp);
                    result.offer.offer_id = corp_id * 100 + index as i32;
                    result.offer.type_id = type_id;
                    Arc::new(result)
                })
                .collect();
            let offer_ids = results.iter().map(|result| result.offer.offer_id).collect();
            Store::new(vec![corp_id], offer_ids, results)
        };
        let app = app_with(
            vec![
                store(1, [(10.0, 34), (9.0, 35)]),
                store(2, [(8.0, 36), (7.0, 34)]),
            ],
            10,
        );
        assert_eq!(first_corporations(&app), [1, 2]);
        app
    }

    fn corp_names(corp_id: i32) -> String {
        match corp_id {
            1 => "Caldari Navy",
            2 => "Sisters of EVE",
            _ => "Unknown",
        }
        .to_string()
    }

    fn type_names(type_id: i32) -> String {
        match type_id {
            34 => "Tritanium",
            35 => "Pyerite",
            36 => "Mexallon",
            _ => "Unknown",
        }
        .to_string()
    }

    /// Types `query` into a search prompt opened with `/`, and presses Enter.
    fn search_for(app: &mut App, query: &str) -> Action {
        assert_eq!(app.on_key(char_key('/')), Action::Continue);
        assert_eq!(search_prompt(app), &SearchPrompt::default());
        for c in query.chars() {
            assert_eq!(app.on_key(char_key(c)), Action::Continue);
        }
        app.on_key(press(KeyCode::Enter))
    }

    /// What the loop does for a search action.
    fn act(app: &mut App, action: Action) {
        let direction = match action {
            Action::Find | Action::FindNext => Direction::Next,
            Action::FindPrevious => Direction::Previous,
            other => panic!("not a search action: {other:?}"),
        };
        app.find_with(direction, &corp_names, &type_names);
    }

    /// The selected group's first corporation and the selected offer's ID.
    fn selection(app: &App) -> (i32, Option<i32>) {
        (
            selected_corporation(app),
            app.selected_offer().map(|o| o.offer.offer_id),
        )
    }

    #[test]
    fn q_is_a_letter_in_the_search_prompt_and_esc_cancels_it() {
        let mut app = named_app();
        app.on_key(char_key('/'));
        assert!(matches!(app.overlay, Overlay::Search(_)));
        assert_eq!(app.on_key(char_key('q')), Action::Continue);
        assert_eq!(search_prompt(&app).query, "q");
        app.on_key(ctrl('u'));
        assert_eq!(search_prompt(&app).query, "");
        assert_eq!(app.on_key(press(KeyCode::Esc)), Action::Continue);
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.search, None, "nothing was searched for");
        app.on_key(char_key('/'));
        assert_eq!(app.on_key(ctrl('c')), Action::Quit);
    }

    #[test]
    fn enter_asks_for_the_search_and_an_empty_one_closes_the_prompt() {
        let mut app = named_app();
        assert_eq!(search_for(&mut app, "trit"), Action::Find);
        assert!(
            matches!(app.overlay, Overlay::Search(_)),
            "the loop runs it, not the key"
        );
        app.on_key(press(KeyCode::Esc));
        assert_eq!(search_for(&mut app, ""), Action::Continue);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn n_does_nothing_without_a_search() {
        let mut app = named_app();
        app.on_key(press(KeyCode::Tab));
        app.on_key(press(KeyCode::Down));
        let before = (app.group, app.offer, app.overlay.clone());
        assert_eq!(app.on_key(char_key('n')), Action::Continue);
        assert_eq!(app.on_key(char_key('N')), Action::Continue);
        assert_eq!((app.group, app.offer, app.overlay.clone()), before);
    }

    #[test]
    fn find_jumps_across_groups_to_an_item_and_wraps_around() {
        let mut app = named_app();
        app.detail_scroll = 3;
        let action = search_for(&mut app, "TRIT");
        act(&mut app, action);
        assert_eq!(app.overlay, Overlay::None, "a hit closes the prompt");
        assert_eq!(
            selection(&app),
            (2, Some(201)),
            "past the selected Tritanium, to the next group's"
        );
        assert_eq!(app.detail_scroll, 0, "the details start at the top");
        assert_eq!(app.on_key(char_key('n')), Action::FindNext);
        act(&mut app, Action::FindNext);
        assert_eq!(selection(&app), (1, Some(100)), "round to the first group");
        assert_eq!(app.on_key(char_key('N')), Action::FindPrevious);
        act(&mut app, Action::FindPrevious);
        assert_eq!(selection(&app), (2, Some(201)), "back the other way");
    }

    #[test]
    fn find_matches_corporation_names_and_follows_the_table_order() {
        let mut app = named_app();
        let action = search_for(&mut app, "sisters");
        act(&mut app, action);
        assert_eq!(app.group, 1);
        assert_eq!(
            app.offer, 0,
            "a corporation hit selects the group's first row"
        );
        // Sorted by ISK per LP reversed, corporation 2's table shows Tritanium first, so going
        // on from the corporation names reaches it before Mexallon.
        app.offer_sort.reverse = true;
        // "n" is in both item names, so starting after the first row would find Mexallon.
        let action = search_for(&mut app, "n");
        act(&mut app, action);
        assert_eq!(selection(&app), (2, Some(201)));
        act(&mut app, Action::FindNext);
        assert_eq!(selection(&app), (2, Some(200)));
    }

    #[test]
    fn a_missed_search_keeps_the_prompt_open_with_no_match() {
        let mut app = named_app();
        let action = search_for(&mut app, "veldspar");
        act(&mut app, action);
        assert_eq!(search_prompt(&app).error.as_deref(), Some("No match"));
        assert_eq!(selection(&app), (1, Some(100)), "the selection stays");
        app.on_key(press(KeyCode::Backspace));
        assert_eq!(search_prompt(&app).error, None, "editing clears it");
        assert_eq!(app.search, None, "only a hit becomes the search n repeats");
    }

    #[test]
    fn n_that_finds_nothing_marks_the_search_until_the_next_key() {
        let mut app = named_app();
        let action = search_for(&mut app, "pyerite");
        act(&mut app, action);
        assert_eq!(selection(&app), (1, Some(101)));
        // At depth 1 corporation 1 lists only its Tritanium, so nothing lists Pyerite.
        app.rearrange(|arrangement| arrangement.depth = 1);
        let action = app.on_key(char_key('n'));
        act(&mut app, action);
        let search = app.search.as_ref().expect("the search is kept");
        assert!(search.missed);
        assert_eq!(selection(&app), (1, Some(100)), "the selection stays");
        app.on_key(press(KeyCode::Down));
        assert!(!app.search.as_ref().unwrap().missed, "any key clears it");
    }

    #[test]
    fn find_reads_the_names_the_panes_show() {
        // Without static data the panes show `Type <id>` and the corporation ID.
        let mut app = named_app();
        let action = search_for(&mut app, "type 36");
        assert_eq!(action, Action::Find);
        app.find(Direction::Next, &StaticData::default());
        assert_eq!(selection(&app), (2, Some(200)));
        app.on_key(char_key('/'));
        app.on_key(char_key('1'));
        app.find(Direction::Next, &StaticData::default());
        assert_eq!(selection(&app), (1, Some(100)), "corporation 1 by its ID");
    }
}

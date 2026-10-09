//! The state of the `--tui` view. Run events and key presses go in, and the new state comes
//! out. Nothing here touches the terminal, so these rules can be tested directly.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use eve_lpstore::OfferResult;
use eve_lpstore::report::{Event, Group, Report};
use eve_lpstore::static_data::AgentFilter;
use std::collections::VecDeque;

const LOG_LIMIT: usize = 500;
const PAGE: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Starting,
    Fetching { done: usize, total: usize },
    Pricing { done: usize, total: usize },
    Finished { groups: usize },
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

pub struct App {
    pub region: String,
    pub corporations: usize,
    pub filters: String,
    pub depth: usize,
    pub agent_filter: AgentFilter,
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
}

impl App {
    pub fn new(
        region: String,
        corporations: usize,
        filters: String,
        depth: usize,
        agent_filter: AgentFilter,
    ) -> Self {
        Self {
            region,
            corporations,
            filters,
            depth,
            agent_filter,
            stage: Stage::Starting,
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
        self.report = Some(report);
        self.focus = Focus::Groups;
        self.group = 0;
        self.offer = 0;
        self.detail_scroll = 0;
    }

    pub fn fail(&mut self) {
        self.stage = Stage::Failed;
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
            Stage::Starting | Stage::Failed => ((0, 0), (0, 0)),
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
        if is_quit(&key) {
            return Action::Quit;
        }
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('?') => self.toggle(Overlay::Help),
            KeyCode::Char('l') => self.toggle(Overlay::Log),
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

/// Whether a key press asks to quit: `q`, or Ctrl+C, which raw mode delivers as a key event.
fn is_quit(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('q') => key.modifiers.is_empty(),
        KeyCode::Char('c') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
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
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            depth,
            AgentFilter::default(),
        );
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
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            10,
            AgentFilter::default(),
        );
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
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            10,
            AgentFilter::default(),
        );
        app.fail();
        assert_eq!(app.stage, Stage::Failed);
        assert_eq!(app.stage_text(), "The run stopped unexpectedly");
    }

    #[test]
    fn log_keeps_only_the_newest_lines() {
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            10,
            AgentFilter::default(),
        );
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
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            10,
            AgentFilter::default(),
        );
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
        let mut app = App::new(
            "The Forge".to_string(),
            3,
            "none".to_string(),
            10,
            AgentFilter::default(),
        );
        app.on_key(press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Groups);
        app.finish(Report { groups: Vec::new() });
        app.on_key(press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Offers);
    }
}

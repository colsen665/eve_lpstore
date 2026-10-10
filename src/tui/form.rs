//! The filter form of the `--tui` view. It holds the text the user types and turns it into
//! `LpStoreOptions`. Whether the values name real things is decided later, by `resolve`.

use eve_lpstore::report::{Field, LpStoreOptions, OptionError};

/// The fields of the form, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Corporations,
    Region,
    Security,
    AgentRegion,
    AgentSystem,
    AgentLevel,
    Division,
    Type,
    Faction,
    System,
    Depth,
    CacheMinutes,
    Expire,
}

impl FormField {
    /// The fields in the order the dialog shows them and Tab moves through them. Grouped: which
    /// corporations and agents, then where the prices are read from, then how the results are
    /// shown. The two agent place filters sit directly below `Security`, which is also a filter on
    /// where an agent stands. Cache timing follows depth; `Expire` stays last so the space toggle is
    /// always the final stop.
    pub const ALL: [FormField; 13] = [
        FormField::Corporations,
        FormField::Security,
        FormField::AgentRegion,
        FormField::AgentSystem,
        FormField::AgentLevel,
        FormField::Division,
        FormField::Type,
        FormField::Faction,
        FormField::Region,
        FormField::System,
        FormField::Depth,
        FormField::CacheMinutes,
        FormField::Expire,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FormField::Corporations => "Corporations",
            FormField::Security => "Security",
            // Named for the agents they filter, standing beside the market pair further down, which
            // is named for the prices it moves.
            FormField::AgentRegion => "Agent region",
            FormField::AgentSystem => "Agent system",
            FormField::AgentLevel => "Agent level",
            FormField::Division => "Division",
            FormField::Type => "Agent type",
            FormField::Faction => "Faction",
            // Named for what they filter, since a bare "Region" next to agent filters reads like
            // another corporation attribute. Only the first word is capitalised, as in "Agent level"
            // and "Expire cache". The command line keeps --region and --system.
            FormField::Region => "Market region",
            FormField::System => "Market system",
            FormField::Depth => "Depth",
            FormField::CacheMinutes => "Cache minutes",
            FormField::Expire => "Expire cache",
        }
    }

    /// The form field an option error about `field` belongs to, if one does.
    fn of(field: Field) -> Option<FormField> {
        match field {
            Field::Lpstore => Some(FormField::Corporations),
            Field::Region => Some(FormField::Region),
            Field::Security => Some(FormField::Security),
            Field::AgentLevel => Some(FormField::AgentLevel),
            Field::AgentDivision => Some(FormField::Division),
            Field::AgentType => Some(FormField::Type),
            Field::AgentRegion => Some(FormField::AgentRegion),
            Field::AgentSystem => Some(FormField::AgentSystem),
            Field::Faction => Some(FormField::Faction),
            Field::System => Some(FormField::System),
            Field::Depth => Some(FormField::Depth),
            Field::CacheMinutes => Some(FormField::CacheMinutes),
            Field::Concurrency | Field::MaxInflight => None,
        }
    }
}

/// The filters and depth being edited, and the problems found when they were last applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterForm {
    pub focus: FormField,
    pub corporations: String,
    pub region: String,
    pub security: String,
    pub agent_region: String,
    pub agent_system: String,
    pub agent_level: String,
    pub division: String,
    pub agent_type: String,
    pub faction: String,
    pub system: String,
    pub depth: String,
    pub cache_minutes: String,
    pub expire: bool,
    errors: Vec<(FormField, String)>,
    general_error: Option<String>,
}

impl FilterForm {
    pub fn from_options(options: &LpStoreOptions) -> Self {
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        Self {
            focus: FormField::Corporations,
            corporations: text(&options.lpstore),
            region: text(&options.region),
            security: text(&options.security),
            agent_region: text(&options.agent_region),
            agent_system: text(&options.agent_system),
            agent_level: text(&options.agent_level),
            division: text(&options.agent_division),
            agent_type: text(&options.agent_type),
            faction: text(&options.faction),
            system: text(&options.system),
            depth: text(&options.depth),
            cache_minutes: text(&options.cache_minutes),
            expire: options.expire,
            errors: Vec::new(),
            general_error: None,
        }
    }

    /// The form as the startup dialog shows it: region and depth carry the values the run would
    /// use when they are not set, so the user sees what is about to happen instead of two blank
    /// fields standing for invisible defaults. Every other field stays empty, because for those
    /// an empty field is the default, meaning all corporations or any security status, and
    /// writing one out would suggest a filter is in force when none is.
    pub fn from_options_with_defaults(
        options: &LpStoreOptions,
        region: &str,
        depth: usize,
    ) -> Self {
        Self {
            region: options.region.clone().unwrap_or_else(|| region.to_string()),
            depth: options.depth.clone().unwrap_or_else(|| depth.to_string()),
            ..Self::from_options(options)
        }
    }

    /// The options the form describes. Blank fields are unset. The options the form does not
    /// show come from `base`.
    pub fn to_options(&self, base: &LpStoreOptions) -> LpStoreOptions {
        LpStoreOptions {
            lpstore: blank_to_none(&self.corporations),
            region: blank_to_none(&self.region),
            security: blank_to_none(&self.security),
            agent_region: blank_to_none(&self.agent_region),
            agent_system: blank_to_none(&self.agent_system),
            agent_level: blank_to_none(&self.agent_level),
            agent_division: blank_to_none(&self.division),
            agent_type: blank_to_none(&self.agent_type),
            faction: blank_to_none(&self.faction),
            system: blank_to_none(&self.system),
            depth: blank_to_none(&self.depth),
            cache_minutes: blank_to_none(&self.cache_minutes),
            expire: self.expire,
            ..base.clone()
        }
    }

    pub fn text(&self, field: FormField) -> &str {
        match field {
            FormField::Corporations => self.corporations.as_str(),
            FormField::Region => self.region.as_str(),
            FormField::Security => self.security.as_str(),
            FormField::AgentRegion => self.agent_region.as_str(),
            FormField::AgentSystem => self.agent_system.as_str(),
            FormField::AgentLevel => self.agent_level.as_str(),
            FormField::Division => self.division.as_str(),
            FormField::Type => self.agent_type.as_str(),
            FormField::Faction => self.faction.as_str(),
            FormField::System => self.system.as_str(),
            FormField::Depth => self.depth.as_str(),
            FormField::CacheMinutes => self.cache_minutes.as_str(),
            FormField::Expire => "",
        }
    }

    fn text_mut(&mut self, field: FormField) -> Option<&mut String> {
        match field {
            FormField::Corporations => Some(&mut self.corporations),
            FormField::Region => Some(&mut self.region),
            FormField::Security => Some(&mut self.security),
            FormField::AgentRegion => Some(&mut self.agent_region),
            FormField::AgentSystem => Some(&mut self.agent_system),
            FormField::AgentLevel => Some(&mut self.agent_level),
            FormField::Division => Some(&mut self.division),
            FormField::Type => Some(&mut self.agent_type),
            FormField::Faction => Some(&mut self.faction),
            FormField::System => Some(&mut self.system),
            FormField::Depth => Some(&mut self.depth),
            FormField::CacheMinutes => Some(&mut self.cache_minutes),
            FormField::Expire => None,
        }
    }

    pub fn next_field(&mut self) {
        self.focus = step(self.focus, 1);
    }

    pub fn previous_field(&mut self) {
        self.focus = step(self.focus, -1);
    }

    /// Types `c` into the focused field. On the expire option, a space toggles it instead.
    pub fn type_char(&mut self, c: char) {
        let focus = self.focus;
        if focus == FormField::Expire {
            if c == ' ' {
                self.expire = !self.expire;
            }
            return;
        }
        if let Some(text) = self.text_mut(focus) {
            text.push(c);
        }
        self.clear_error(focus);
    }

    pub fn backspace(&mut self) {
        let focus = self.focus;
        if let Some(text) = self.text_mut(focus) {
            text.pop();
        }
        self.clear_error(focus);
    }

    pub fn clear_focused(&mut self) {
        let focus = self.focus;
        if let Some(text) = self.text_mut(focus) {
            text.clear();
        }
        self.clear_error(focus);
    }

    fn clear_error(&mut self, field: FormField) {
        self.errors.retain(|(error_field, _)| *error_field != field);
    }

    /// Keeps the problems `resolve` found, each beside its field. Problems that belong to no
    /// field are kept apart.
    pub fn set_errors(&mut self, errors: Vec<OptionError>) {
        self.errors.clear();
        self.general_error = None;
        for error in errors {
            match FormField::of(error.field) {
                Some(field) => self.errors.push((field, error.message)),
                None => self.general_error = Some(error.message),
            }
        }
    }

    pub fn error_for(&self, field: FormField) -> Option<String> {
        let messages: Vec<&str> = self
            .errors
            .iter()
            .filter(|(error_field, _)| *error_field == field)
            .map(|(_, message)| message.as_str())
            .collect();
        (!messages.is_empty()).then(|| messages.join("; "))
    }

    /// A note about the values as they stand, shown where a field error would be. Used for a saved
    /// filters file that could not be read: worth saying, but not a validation failure, so it does
    /// not stop Enter from applying. A later `set_errors` replaces it.
    pub fn set_note(&mut self, note: Option<String>) {
        self.general_error = note;
    }

    pub fn general_error(&self) -> Option<&str> {
        self.general_error.as_deref()
    }

    /// The name being typed into a name field, to match against the known names. In the
    /// corporations field, only the entry after the last comma counts.
    pub fn name_query(&self) -> Option<&str> {
        let query = match self.focus {
            FormField::Corporations => self.corporations.rsplit(',').next().unwrap_or(""),
            FormField::Region => self.region.as_str(),
            FormField::System => self.system.as_str(),
            FormField::AgentRegion => self.agent_region.as_str(),
            FormField::AgentSystem => self.agent_system.as_str(),
            _ => return None,
        };
        let query = query.trim();
        (!query.is_empty()).then_some(query)
    }
}

fn step(field: FormField, delta: isize) -> FormField {
    let index = FormField::ALL
        .iter()
        .position(|&candidate| candidate == field)
        .unwrap_or(0) as isize;
    let count = FormField::ALL.len() as isize;
    FormField::ALL[(index + delta).rem_euclid(count) as usize]
}

fn blank_to_none(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> FilterForm {
        FilterForm::from_options(&LpStoreOptions::default())
    }

    #[test]
    fn options_survive_a_round_trip() {
        let options = LpStoreOptions {
            lpstore: Some("Caldari Navy,Amarr Navy".to_string()),
            security: Some("0.5..0.7".to_string()),
            depth: Some("5".to_string()),
            cache_minutes: Some("30".to_string()),
            expire: true,
            ..Default::default()
        };
        let form = FilterForm::from_options(&options);
        assert_eq!(form.to_options(&options), options);
    }

    #[test]
    fn blank_fields_are_unset_and_the_rest_comes_from_the_base() {
        let base = LpStoreOptions {
            verbose: true,
            ..Default::default()
        };
        let mut form = form();
        form.region = "   ".to_string();
        let options = form.to_options(&base);
        assert_eq!(options.region, None);
        assert_eq!(options.depth, None);
        assert!(options.verbose);
    }

    #[test]
    fn typing_and_backspace_edit_the_focused_field() {
        let mut form = form();
        // Focused by name rather than by stepping, so this does not move when the fields are regrouped.
        form.focus = FormField::Region;
        form.type_char('T');
        form.type_char('h');
        form.backspace();
        assert_eq!(form.region, "T");
    }

    #[test]
    fn space_toggles_the_expire_option_and_other_keys_do_not() {
        let mut form = form();
        form.previous_field();
        assert_eq!(form.focus, FormField::Expire);
        form.type_char(' ');
        assert!(form.expire);
        form.type_char('x');
        assert!(form.expire);
    }

    #[test]
    fn focus_wraps_around_the_fields() {
        let mut form = form();
        form.previous_field();
        assert_eq!(form.focus, FormField::Expire);
        form.next_field();
        assert_eq!(form.focus, FormField::Corporations);
    }

    #[test]
    fn errors_sit_beside_their_fields() {
        let mut form = form();
        form.set_errors(vec![
            OptionError {
                field: Field::AgentDivision,
                message: "bad division".to_string(),
            },
            OptionError {
                field: Field::CacheMinutes,
                message: "bad minutes".to_string(),
            },
        ]);
        assert_eq!(
            form.error_for(FormField::Division).as_deref(),
            Some("bad division")
        );
        assert_eq!(
            form.error_for(FormField::CacheMinutes).as_deref(),
            Some("bad minutes")
        );
        assert_eq!(form.general_error(), None);
    }

    #[test]
    fn editing_a_field_clears_its_error() {
        let mut form = form();
        form.set_errors(vec![OptionError {
            field: Field::Lpstore,
            message: "not found".to_string(),
        }]);
        assert!(form.error_for(FormField::Corporations).is_some());
        form.type_char('x');
        assert_eq!(form.error_for(FormField::Corporations), None);
    }

    #[test]
    fn defaults_fill_region_and_depth_only() {
        let options = LpStoreOptions {
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        let form = FilterForm::from_options_with_defaults(&options, "The Forge", 10);
        assert_eq!(form.region, "The Forge");
        assert_eq!(form.depth, "10");
        // The filters keep their meaning of "no filter", so an empty field is not a value.
        assert_eq!(form.corporations, "");
        assert_eq!(form.security, "");
        assert_eq!(form.agent_level, "");
        assert_eq!(form.division, "");
        assert_eq!(form.agent_type, "");
        assert_eq!(form.system, "");
        assert_eq!(
            form.faction, "caldari",
            "a filter the command line set is still shown"
        );
    }

    #[test]
    fn a_value_from_the_command_line_beats_the_default() {
        let options = LpStoreOptions {
            region: Some("Domain".to_string()),
            depth: Some("25".to_string()),
            ..Default::default()
        };
        let form = FilterForm::from_options_with_defaults(&options, "The Forge", 10);
        assert_eq!(form.region, "Domain");
        assert_eq!(form.depth, "25");
    }

    #[test]
    fn the_prefilled_defaults_round_trip_back_to_options() {
        // Pre-filling must not invent a filter: the form has to give back options whose only
        // additions over the command line are the two defaults it was handed.
        let blank = LpStoreOptions::default();
        let form = FilterForm::from_options_with_defaults(&blank, "The Forge", 10);
        let applied = form.to_options(&blank);
        assert_eq!(applied.lpstore, None);
        assert_eq!(applied.security, None);
        assert_eq!(applied.agent_level, None);
        assert_eq!(applied.agent_division, None);
        assert_eq!(applied.agent_type, None);
        assert_eq!(applied.faction, None);
        assert_eq!(applied.system, None);
        assert_eq!(applied.region.as_deref(), Some("The Forge"));
        assert_eq!(applied.depth.as_deref(), Some("10"));
    }

    #[test]
    fn a_note_shows_where_a_field_error_would() {
        let mut form = form();
        assert_eq!(form.general_error(), None);
        form.set_note(Some("filters.json is not valid JSON".to_string()));
        assert_eq!(form.general_error(), Some("filters.json is not valid JSON"));
        // A note is not a validation failure, so it must not stop the form from being applied.
        assert_eq!(form.error_for(FormField::Corporations), None);
    }

    #[test]
    fn a_validation_failure_replaces_the_note() {
        let mut form = form();
        form.set_note(Some("filters.json is not valid JSON".to_string()));
        form.set_errors(vec![OptionError {
            field: Field::Region,
            message: "no such region".to_string(),
        }]);
        assert_eq!(form.general_error(), None, "the note is gone");
        assert_eq!(
            form.error_for(FormField::Region),
            Some("no such region".to_string())
        );
    }

    #[test]
    fn clearing_the_note_leaves_nothing_shown() {
        let mut form = form();
        form.set_note(Some("anything".to_string()));
        form.set_note(None);
        assert_eq!(form.general_error(), None);
    }

    #[test]
    fn name_queries_come_from_the_text_being_typed() {
        let mut form = form();
        form.corporations = "Caldari Navy, Am".to_string();
        assert_eq!(form.name_query(), Some("Am"));
        form.corporations = "Caldari Navy,".to_string();
        assert_eq!(form.name_query(), None);
        form.focus = FormField::Region;
        form.region = "The Fo".to_string();
        assert_eq!(form.name_query(), Some("The Fo"));
        form.focus = FormField::System;
        form.system = "Jita".to_string();
        assert_eq!(form.name_query(), Some("Jita"));
        form.focus = FormField::Security;
        assert_eq!(form.name_query(), None);
    }

    #[test]
    fn the_market_fields_are_labelled_and_grouped_together() {
        assert_eq!(FormField::Region.label(), "Market region");
        assert_eq!(FormField::System.label(), "Market system");
        // Both names fit the dialog's label column, so grouping costs no layout change.
        let column = crate::tui::ui::LABEL_WIDTH;
        for field in FormField::ALL {
            assert!(
                field.label().chars().count() <= column,
                "{:?} does not fit {} columns",
                field,
                column
            );
        }
        let position = |wanted| {
            FormField::ALL
                .iter()
                .position(|field| *field == wanted)
                .unwrap()
        };
        // The two that choose where prices come from sit side by side, after the corporation and
        // agent filters and before how the results are shown.
        assert_eq!(position(FormField::Region) + 1, position(FormField::System));
        assert!(position(FormField::Faction) < position(FormField::Region));
        assert!(position(FormField::Region) < position(FormField::Depth));
    }

    #[test]
    fn the_two_place_filters_sit_directly_below_security() {
        let names: Vec<&str> = FormField::ALL.iter().map(|field| field.label()).collect();
        assert_eq!(
            &names[1..5],
            ["Security", "Agent region", "Agent system", "Agent level"],
            "the dialog order is what the request asked for: {names:?}"
        );
        let position = |field| {
            FormField::ALL
                .iter()
                .position(|candidate| *candidate == field)
                .unwrap()
        };
        assert_eq!(
            position(FormField::Depth) + 1,
            position(FormField::CacheMinutes)
        );
        assert_eq!(
            position(FormField::CacheMinutes) + 1,
            position(FormField::Expire)
        );
        assert_eq!(FormField::ALL.len(), 13);
    }

    #[test]
    fn the_new_fields_round_trip_through_the_options() {
        let options = LpStoreOptions {
            agent_region: Some("Black Rise".to_string()),
            agent_system: Some("4-3L53".to_string()),
            cache_minutes: Some("90".to_string()),
            ..Default::default()
        };
        let form = FilterForm::from_options(&options);
        assert_eq!(form.agent_region, "Black Rise");
        assert_eq!(form.agent_system, "4-3L53");
        assert_eq!(form.cache_minutes, "90");
        let back = form.to_options(&LpStoreOptions::default());
        assert_eq!(back.agent_region, Some("Black Rise".to_string()));
        assert_eq!(back.agent_system, Some("4-3L53".to_string()));
        assert_eq!(back.cache_minutes, Some("90".to_string()));
    }

    #[test]
    fn blank_place_fields_are_no_filter() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.agent_region = "   ".to_string();
        let options = form.to_options(&LpStoreOptions::default());
        assert_eq!(options.agent_region, None, "spaces are not a region");
    }

    #[test]
    fn cache_minutes_stays_blank_when_unset() {
        let form = FilterForm::from_options(&LpStoreOptions::default());
        assert_eq!(form.cache_minutes, "");
        assert_eq!(
            form.to_options(&LpStoreOptions::default()).cache_minutes,
            None,
            "blank leaves the existing default to resolve"
        );
    }

    #[test]
    fn both_place_fields_offer_name_matches() {
        let mut form = FilterForm::from_options(&LpStoreOptions {
            agent_region: Some("bla".to_string()),
            ..Default::default()
        });
        form.focus = FormField::AgentRegion;
        assert_eq!(form.name_query(), Some("bla"));
        form.focus = FormField::AgentSystem;
        form.agent_system = "Jita".to_string();
        assert_eq!(form.name_query(), Some("Jita"));
    }

    #[test]
    fn tab_walks_the_places_without_skipping_or_repeating() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.focus = FormField::Security;
        form.next_field();
        assert_eq!(form.focus, FormField::AgentRegion);
        form.next_field();
        assert_eq!(form.focus, FormField::AgentSystem);
        form.next_field();
        assert_eq!(
            form.focus,
            FormField::AgentLevel,
            "the market fields stay where they were"
        );
        form.previous_field();
        assert_eq!(form.focus, FormField::AgentSystem);
    }

    #[test]
    fn typing_and_backspacing_land_in_the_right_box() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.focus = FormField::AgentSystem;
        for c in "Jita".chars() {
            form.type_char(c);
        }
        assert_eq!(form.agent_system, "Jita");
        form.backspace();
        assert_eq!(form.agent_system, "Jit");
        form.clear_focused();
        assert_eq!(form.agent_system, "");
        assert_eq!(form.agent_region, "", "the other box is untouched");
    }

    #[test]
    fn a_problem_about_an_agent_place_lands_on_its_field() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.set_errors(vec![
            OptionError {
                field: Field::AgentRegion,
                message: "Region 'Nope' not found".to_string(),
            },
            OptionError {
                field: Field::AgentSystem,
                message: "Unknown --agent-system 'Nope'".to_string(),
            },
        ]);
        assert_eq!(
            form.error_for(FormField::AgentRegion),
            Some("Region 'Nope' not found".to_string())
        );
        assert_eq!(
            form.error_for(FormField::AgentSystem),
            Some("Unknown --agent-system 'Nope'".to_string())
        );
        assert_eq!(
            form.error_for(FormField::Region),
            None,
            "the market fields stay clean"
        );
    }
}

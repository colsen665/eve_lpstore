//! The filter form of the `--tui` view. It holds the text the user types and turns it into
//! `LpStoreOptions`. Whether the values name real things is decided later, by `resolve`.

use eve_lpstore::report::{Field, LpStoreOptions, OptionError};

/// A field of the form: one of the options [`Field::in_form`] names, or the expire toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Text(Field),
    Expire,
}

impl FormField {
    /// The fields in the order the dialog shows them and Tab moves through them, which is the
    /// order of [`Field::ALL`]: which corporations and agents, then where the prices are read from,
    /// then how the results are shown. `Expire` stays last so the space toggle is always the final
    /// stop.
    pub fn all() -> Vec<FormField> {
        Field::ALL
            .into_iter()
            .filter(|field| field.in_form())
            .map(FormField::Text)
            .chain([FormField::Expire])
            .collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            FormField::Text(field) => field.spec().form_label.unwrap_or_else(|| field.flag()),
            FormField::Expire => "Expire cache",
        }
    }

    /// The form field an option error about `field` belongs to, if one does.
    fn of(field: Field) -> Option<FormField> {
        field.in_form().then_some(FormField::Text(field))
    }
}

/// The filters and depth being edited, and the problems found when they were last applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterForm {
    pub focus: FormField,
    /// The text of every field, held as the options it stands for. A field typed empty is
    /// `Some("")` here and becomes unset in [`to_options`](Self::to_options).
    typed: LpStoreOptions,
    errors: Vec<(FormField, String)>,
    general_error: Option<String>,
}

impl FilterForm {
    pub fn from_options(options: &LpStoreOptions) -> Self {
        let mut typed = LpStoreOptions {
            expire: options.expire,
            ..Default::default()
        };
        for field in Field::ALL.into_iter().filter(|field| field.in_form()) {
            *typed.value_mut(field) = options.value(field).clone();
        }
        Self {
            focus: FormField::all()[0],
            typed,
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
        let mut form = Self::from_options(options);
        form.typed.region.get_or_insert_with(|| region.to_string());
        form.typed.depth.get_or_insert_with(|| depth.to_string());
        form
    }

    /// The options the form describes. Blank fields are unset. The options the form does not
    /// show come from `base`.
    pub fn to_options(&self, base: &LpStoreOptions) -> LpStoreOptions {
        let mut options = base.clone();
        for field in Field::ALL.into_iter().filter(|field| field.in_form()) {
            *options.value_mut(field) = blank_to_none(self.value(field));
        }
        options.expire = self.typed.expire;
        options
    }

    /// The text typed into `field`. Empty for the expire toggle, which has no text.
    pub fn text(&self, field: FormField) -> &str {
        match field {
            FormField::Text(field) => self.value(field),
            FormField::Expire => "",
        }
    }

    /// The text typed into the box for `field`.
    pub fn value(&self, field: Field) -> &str {
        self.typed.value(field).as_deref().unwrap_or("")
    }

    /// Replaces the text of the box for `field`. Its error, if it has one, stays until the user
    /// edits the box or the form is applied again.
    pub fn set_value(&mut self, field: Field, text: &str) {
        *self.typed.value_mut(field) = Some(text.to_string());
    }

    pub fn expire(&self) -> bool {
        self.typed.expire
    }

    fn text_mut(&mut self, field: FormField) -> Option<&mut String> {
        match field {
            FormField::Text(field) => Some(self.typed.value_mut(field).get_or_insert_default()),
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
                self.typed.expire = !self.typed.expire;
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
        let FormField::Text(field) = self.focus else {
            return None;
        };
        let typed = self.value(field);
        let query = match field {
            Field::Lpstore => typed.rsplit(',').next().unwrap_or(""),
            Field::Region | Field::System | Field::AgentRegion | Field::AgentSystem => typed,
            _ => return None,
        };
        let query = query.trim();
        (!query.is_empty()).then_some(query)
    }
}

fn step(field: FormField, delta: isize) -> FormField {
    let all = FormField::all();
    let index = all
        .iter()
        .position(|&candidate| candidate == field)
        .unwrap_or(0) as isize;
    let count = all.len() as isize;
    all[(index + delta).rem_euclid(count) as usize]
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
    fn the_form_carries_only_its_own_fields() {
        // An option the form has no box for comes from the base on apply, whatever the form was
        // built from.
        let built_from = LpStoreOptions {
            concurrency: Some("8".to_string()),
            cache_dir: Some("/tmp/a".to_string()),
            ..Default::default()
        };
        let base = LpStoreOptions {
            concurrency: Some("2".to_string()),
            ..Default::default()
        };
        let options = FilterForm::from_options(&built_from).to_options(&base);
        assert_eq!(options.concurrency.as_deref(), Some("2"));
        assert_eq!(options.cache_dir, None);
        for field in FormField::all() {
            assert_ne!(field.label(), "", "{field:?} has a label");
            if let FormField::Text(field) = field {
                assert!(field.in_form());
            }
        }
    }

    #[test]
    fn blank_fields_are_unset_and_the_rest_comes_from_the_base() {
        let base = LpStoreOptions {
            verbose: true,
            ..Default::default()
        };
        let mut form = form();
        form.set_value(Field::Region, "   ");
        let options = form.to_options(&base);
        assert_eq!(options.region, None);
        assert_eq!(options.depth, None);
        assert!(options.verbose);
    }

    #[test]
    fn typing_and_backspace_edit_the_focused_field() {
        let mut form = form();
        // Focused by name rather than by stepping, so this does not move when the fields are regrouped.
        form.focus = FormField::Text(Field::Region);
        form.type_char('T');
        form.type_char('h');
        form.backspace();
        assert_eq!(form.value(Field::Region), "T");
    }

    #[test]
    fn space_toggles_the_expire_option_and_other_keys_do_not() {
        let mut form = form();
        form.previous_field();
        assert_eq!(form.focus, FormField::Expire);
        form.type_char(' ');
        assert!(form.expire());
        form.type_char('x');
        assert!(form.expire());
    }

    #[test]
    fn focus_wraps_around_the_fields() {
        let mut form = form();
        form.previous_field();
        assert_eq!(form.focus, FormField::Expire);
        form.next_field();
        assert_eq!(form.focus, FormField::Text(Field::Lpstore));
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
            form.error_for(FormField::Text(Field::AgentDivision))
                .as_deref(),
            Some("bad division")
        );
        assert_eq!(
            form.error_for(FormField::Text(Field::CacheMinutes))
                .as_deref(),
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
        assert!(form.error_for(FormField::Text(Field::Lpstore)).is_some());
        form.type_char('x');
        assert_eq!(form.error_for(FormField::Text(Field::Lpstore)), None);
    }

    #[test]
    fn defaults_fill_region_and_depth_only() {
        let options = LpStoreOptions {
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        let form = FilterForm::from_options_with_defaults(&options, "The Forge", 10);
        assert_eq!(form.value(Field::Region), "The Forge");
        assert_eq!(form.value(Field::Depth), "10");
        // The filters keep their meaning of "no filter", so an empty field is not a value.
        assert_eq!(form.value(Field::Lpstore), "");
        assert_eq!(form.value(Field::Security), "");
        assert_eq!(form.value(Field::AgentLevel), "");
        assert_eq!(form.value(Field::AgentDivision), "");
        assert_eq!(form.value(Field::AgentType), "");
        assert_eq!(form.value(Field::System), "");
        assert_eq!(
            form.value(Field::Faction),
            "caldari",
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
        assert_eq!(form.value(Field::Region), "Domain");
        assert_eq!(form.value(Field::Depth), "25");
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
        assert_eq!(form.error_for(FormField::Text(Field::Lpstore)), None);
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
            form.error_for(FormField::Text(Field::Region)),
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
        form.set_value(Field::Lpstore, "Caldari Navy, Am");
        assert_eq!(form.name_query(), Some("Am"));
        form.set_value(Field::Lpstore, "Caldari Navy,");
        assert_eq!(form.name_query(), None);
        form.focus = FormField::Text(Field::Region);
        form.set_value(Field::Region, "The Fo");
        assert_eq!(form.name_query(), Some("The Fo"));
        form.focus = FormField::Text(Field::System);
        form.set_value(Field::System, "Jita");
        assert_eq!(form.name_query(), Some("Jita"));
        form.focus = FormField::Text(Field::Security);
        assert_eq!(form.name_query(), None);
    }

    #[test]
    fn the_market_fields_are_labelled_and_grouped_together() {
        assert_eq!(FormField::Text(Field::Region).label(), "Market region");
        assert_eq!(FormField::Text(Field::System).label(), "Market system");
        // Both names fit the dialog's label column, so grouping costs no layout change.
        let column = crate::tui::ui::LABEL_WIDTH;
        for field in FormField::all() {
            assert!(
                field.label().chars().count() <= column,
                "{:?} does not fit {} columns",
                field,
                column
            );
        }
        let position = |wanted| {
            FormField::all()
                .iter()
                .position(|field| *field == wanted)
                .unwrap()
        };
        // The two that choose where prices come from sit side by side, after the corporation and
        // agent filters and before how the results are shown.
        assert_eq!(
            position(FormField::Text(Field::Region)) + 1,
            position(FormField::Text(Field::System))
        );
        assert!(
            position(FormField::Text(Field::Faction)) < position(FormField::Text(Field::Region))
        );
        assert!(position(FormField::Text(Field::Region)) < position(FormField::Text(Field::Depth)));
    }

    #[test]
    fn the_two_place_filters_sit_directly_below_security() {
        let names: Vec<&str> = FormField::all().iter().map(|field| field.label()).collect();
        assert_eq!(
            &names[1..5],
            ["Security", "Agent region", "Agent system", "Agent level"],
            "the dialog order is what the request asked for: {names:?}"
        );
        let position = |field| {
            FormField::all()
                .iter()
                .position(|candidate| *candidate == field)
                .unwrap()
        };
        assert_eq!(
            position(FormField::Text(Field::Depth)) + 1,
            position(FormField::Text(Field::CacheMinutes))
        );
        assert_eq!(
            position(FormField::Text(Field::CacheMinutes)) + 1,
            position(FormField::Expire)
        );
        assert_eq!(FormField::all().len(), 13);
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
        assert_eq!(form.value(Field::AgentRegion), "Black Rise");
        assert_eq!(form.value(Field::AgentSystem), "4-3L53");
        assert_eq!(form.value(Field::CacheMinutes), "90");
        let back = form.to_options(&LpStoreOptions::default());
        assert_eq!(back.agent_region, Some("Black Rise".to_string()));
        assert_eq!(back.agent_system, Some("4-3L53".to_string()));
        assert_eq!(back.cache_minutes, Some("90".to_string()));
    }

    #[test]
    fn blank_place_fields_are_no_filter() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.set_value(Field::AgentRegion, "   ");
        let options = form.to_options(&LpStoreOptions::default());
        assert_eq!(options.agent_region, None, "spaces are not a region");
    }

    #[test]
    fn cache_minutes_stays_blank_when_unset() {
        let form = FilterForm::from_options(&LpStoreOptions::default());
        assert_eq!(form.value(Field::CacheMinutes), "");
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
        form.focus = FormField::Text(Field::AgentRegion);
        assert_eq!(form.name_query(), Some("bla"));
        form.focus = FormField::Text(Field::AgentSystem);
        form.set_value(Field::AgentSystem, "Jita");
        assert_eq!(form.name_query(), Some("Jita"));
    }

    #[test]
    fn tab_walks_the_places_without_skipping_or_repeating() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.focus = FormField::Text(Field::Security);
        form.next_field();
        assert_eq!(form.focus, FormField::Text(Field::AgentRegion));
        form.next_field();
        assert_eq!(form.focus, FormField::Text(Field::AgentSystem));
        form.next_field();
        assert_eq!(
            form.focus,
            FormField::Text(Field::AgentLevel),
            "the market fields stay where they were"
        );
        form.previous_field();
        assert_eq!(form.focus, FormField::Text(Field::AgentSystem));
    }

    #[test]
    fn typing_and_backspacing_land_in_the_right_box() {
        let mut form = FilterForm::from_options(&LpStoreOptions::default());
        form.focus = FormField::Text(Field::AgentSystem);
        for c in "Jita".chars() {
            form.type_char(c);
        }
        assert_eq!(form.value(Field::AgentSystem), "Jita");
        form.backspace();
        assert_eq!(form.value(Field::AgentSystem), "Jit");
        form.clear_focused();
        assert_eq!(form.value(Field::AgentSystem), "");
        assert_eq!(
            form.value(Field::AgentRegion),
            "",
            "the other box is untouched"
        );
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
            form.error_for(FormField::Text(Field::AgentRegion)),
            Some("Region 'Nope' not found".to_string())
        );
        assert_eq!(
            form.error_for(FormField::Text(Field::AgentSystem)),
            Some("Unknown --agent-system 'Nope'".to_string())
        );
        assert_eq!(
            form.error_for(FormField::Text(Field::Region)),
            None,
            "the market fields stay clean"
        );
    }
}

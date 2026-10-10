//! The filter form of the `--tui` view. It holds the text the user types and turns it into
//! `LpStoreOptions`. Whether the values name real things is decided later, by `resolve`.

use eve_lpstore::report::{Field, LpStoreOptions, OptionError};

/// The fields of the form, in the order they are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Corporations,
    Region,
    Security,
    AgentLevel,
    Division,
    Type,
    Faction,
    System,
    Depth,
    Expire,
}

impl FormField {
    pub const ALL: [FormField; 10] = [
        FormField::Corporations,
        FormField::Region,
        FormField::Security,
        FormField::AgentLevel,
        FormField::Division,
        FormField::Type,
        FormField::Faction,
        FormField::System,
        FormField::Depth,
        FormField::Expire,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FormField::Corporations => "Corporations",
            FormField::Region => "Region",
            FormField::Security => "Security",
            FormField::AgentLevel => "Agent level",
            FormField::Division => "Division",
            FormField::Type => "Agent type",
            FormField::Faction => "Faction",
            FormField::System => "System",
            FormField::Depth => "Depth",
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
            Field::Faction => Some(FormField::Faction),
            Field::System => Some(FormField::System),
            Field::Depth => Some(FormField::Depth),
            Field::CacheMinutes | Field::Concurrency | Field::MaxInflight => None,
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
    pub agent_level: String,
    pub division: String,
    pub agent_type: String,
    pub faction: String,
    pub system: String,
    pub depth: String,
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
            agent_level: text(&options.agent_level),
            division: text(&options.agent_division),
            agent_type: text(&options.agent_type),
            faction: text(&options.faction),
            system: text(&options.system),
            depth: text(&options.depth),
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
            agent_level: blank_to_none(&self.agent_level),
            agent_division: blank_to_none(&self.division),
            agent_type: blank_to_none(&self.agent_type),
            faction: blank_to_none(&self.faction),
            system: blank_to_none(&self.system),
            depth: blank_to_none(&self.depth),
            expire: self.expire,
            ..base.clone()
        }
    }

    pub fn text(&self, field: FormField) -> &str {
        match field {
            FormField::Corporations => self.corporations.as_str(),
            FormField::Region => self.region.as_str(),
            FormField::Security => self.security.as_str(),
            FormField::AgentLevel => self.agent_level.as_str(),
            FormField::Division => self.division.as_str(),
            FormField::Type => self.agent_type.as_str(),
            FormField::Faction => self.faction.as_str(),
            FormField::System => self.system.as_str(),
            FormField::Depth => self.depth.as_str(),
            FormField::Expire => "",
        }
    }

    fn text_mut(&mut self, field: FormField) -> Option<&mut String> {
        match field {
            FormField::Corporations => Some(&mut self.corporations),
            FormField::Region => Some(&mut self.region),
            FormField::Security => Some(&mut self.security),
            FormField::AgentLevel => Some(&mut self.agent_level),
            FormField::Division => Some(&mut self.division),
            FormField::Type => Some(&mut self.agent_type),
            FormField::Faction => Some(&mut self.faction),
            FormField::System => Some(&mut self.system),
            FormField::Depth => Some(&mut self.depth),
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
            security: Some("+0.5".to_string()),
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
        form.next_field();
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
        assert_eq!(form.general_error(), Some("bad minutes"));
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
        form.next_field();
        form.region = "The Fo".to_string();
        assert_eq!(form.name_query(), Some("The Fo"));
        form.next_field();
        assert_eq!(form.name_query(), None);
    }
}

//! The filters the last `--tui` run was applied with, kept between runs.
//!
//! The view starts from what the command line asked for and the defaults behind it. That is the
//! right answer for a script and the wrong one for an interactive session, where the same handful
//! of corporations or the same region is usually wanted again. So a successful apply in the view
//! writes the filters it used to `filters.json`, and the next session reads them back before it
//! shows the filters dialog.
//!
//! Only the nine fields the form carries are kept. `--expire` is a one-shot cache bypass, and
//! remembering it would refetch every offer on every later run, and the command-line-only fields
//! are not shown by the form, so there is nothing to remember about them.

use crate::report::LpStoreOptions;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "filters.json";

/// Where the remembered filters are kept. Relative to the working directory, which Phase 8 replaces
/// with the platform's config location.
pub fn path() -> PathBuf {
    PathBuf::from(FILE_NAME)
}

/// The filters a run was applied with, as they are written to disk.
///
/// This is its own struct rather than `LpStoreOptions` with serde on it, so a field the form does
/// not carry cannot be persisted by accident. `#[serde(default)]` lets a file written by a future
/// version, or edited by hand and missing a field, still load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedFilters {
    pub lpstore: Option<String>,
    pub region: Option<String>,
    pub security: Option<String>,
    pub agent_level: Option<String>,
    pub agent_division: Option<String>,
    pub agent_type: Option<String>,
    pub faction: Option<String>,
    pub system: Option<String>,
    pub depth: Option<String>,
}

impl SavedFilters {
    /// The options a run starts from: `cli` for every field the command line set, these values for
    /// the rest. Fields that are `None` in both stay `None`, which is what makes clearing a field in
    /// the form and applying it the way to forget a filter.
    ///
    /// A saved value that is only whitespace is treated as absent, so a hand-edited empty field
    /// reads the way an emptied form field does rather than as a filter that matches nothing. An
    /// empty value the command line gave keeps its current meaning, because this must not change
    /// how the flags behave.
    pub fn overlay(&self, cli: &LpStoreOptions) -> LpStoreOptions {
        let pick = |from_cli: &Option<String>, from_file: &Option<String>| match from_cli {
            Some(value) => Some(value.clone()),
            None => from_file.clone().filter(|value| !value.trim().is_empty()),
        };
        LpStoreOptions {
            lpstore: pick(&cli.lpstore, &self.lpstore),
            region: pick(&cli.region, &self.region),
            security: pick(&cli.security, &self.security),
            agent_level: pick(&cli.agent_level, &self.agent_level),
            agent_division: pick(&cli.agent_division, &self.agent_division),
            agent_type: pick(&cli.agent_type, &self.agent_type),
            faction: pick(&cli.faction, &self.faction),
            system: pick(&cli.system, &self.system),
            depth: pick(&cli.depth, &self.depth),
            // cache_minutes, concurrency, max_inflight, expire, and verbose are command line only,
            // so they come from `cli` and there is no saved value that could override them.
            ..cli.clone()
        }
    }
}

impl From<&LpStoreOptions> for SavedFilters {
    fn from(options: &LpStoreOptions) -> Self {
        Self {
            lpstore: options.lpstore.clone(),
            region: options.region.clone(),
            security: options.security.clone(),
            agent_level: options.agent_level.clone(),
            agent_division: options.agent_division.clone(),
            agent_type: options.agent_type.clone(),
            faction: options.faction.clone(),
            system: options.system.clone(),
            depth: options.depth.clone(),
        }
    }
}

/// What was found where the remembered filters live.
///
/// Three cases rather than an `Option`, because "no file yet" is the normal state of a first run and
/// must not be reported, while "there is a file and it is unusable" has to be. Collapsing them into
/// one, as a boolean did in Phase 7's start-up check, made the missing case read as the good one.
pub enum Loaded {
    /// No file is there, which is the state until a run is applied for the first time.
    Absent,
    /// The filters from the last applied run.
    Filters(SavedFilters),
    /// A file is there and cannot be used. The reason is for a log line.
    Unusable(String),
}

impl Loaded {
    /// The reason to report, for the one case that has something worth saying.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unusable(reason) => Some(reason),
            _ => None,
        }
    }

    /// `cli` with the remembered filters filling the fields it left alone. `Absent` and `Unusable`
    /// leave it exactly as it was, so a first run, and a run after a damaged file, behave the way
    /// they did before the file existed.
    pub fn merge(self, cli: LpStoreOptions) -> LpStoreOptions {
        match self {
            Self::Filters(filters) => filters.overlay(&cli),
            Self::Absent | Self::Unusable(_) => cli,
        }
    }
}

/// Read the remembered filters from [`path`](path).
pub fn load() -> Loaded {
    load_from(&path())
}

/// Read the remembered filters from `file`. Never fails: a missing file is [`Loaded::Absent`], and
/// a file that cannot be read or parsed is [`Loaded::Unusable`] with the reason.
pub fn load_from(file: &Path) -> Loaded {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Absent,
        Err(error) => {
            return Loaded::Unusable(format!("could not read {}: {}", file.display(), error));
        }
    };
    match serde_json::from_str::<SavedFilters>(&text) {
        Ok(filters) => Loaded::Filters(filters),
        Err(error) => Loaded::Unusable(format!("{} is not valid JSON: {}", file.display(), error)),
    }
}

/// Write the nine persisted fields of `options` to [`path`](path).
pub fn save(options: &LpStoreOptions) -> io::Result<()> {
    save_to(&path(), options)
}

/// Write the nine persisted fields of `options` to `file`, as JSON that is readable and editable by
/// hand. Takes an explicit path so a test can write somewhere else than the project directory.
pub fn save_to(file: &Path, options: &LpStoreOptions) -> io::Result<()> {
    let text = serde_json::to_string_pretty(&SavedFilters::from(options))
        .expect("the saved filters always serialize");
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_fields() -> SavedFilters {
        SavedFilters {
            lpstore: Some("1000002".to_string()),
            region: Some("Domain".to_string()),
            security: Some("0.5..0.7".to_string()),
            agent_level: Some("4".to_string()),
            agent_division: Some("security".to_string()),
            agent_type: Some("research".to_string()),
            faction: Some("caldari".to_string()),
            system: Some("Jita".to_string()),
            depth: Some("25".to_string()),
        }
    }

    #[test]
    fn the_saved_fields_round_trip_through_json() {
        let options = LpStoreOptions {
            lpstore: Some("Caldari Navy".to_string()),
            region: Some("The Forge".to_string()),
            depth: Some("5".to_string()),
            ..Default::default()
        };
        let dir =
            std::env::temp_dir().join(format!("eve_lpstore_roundtrip_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("filters.json");
        save_to(&file, &options).unwrap();
        let loaded = load_from(&file);
        let Loaded::Filters(filters) = &loaded else {
            panic!(
                "expected filters, got {:?}",
                loaded.reason().unwrap_or("other")
            );
        };
        assert_eq!(filters.lpstore.as_deref(), Some("Caldari Navy"));
        assert_eq!(filters.region.as_deref(), Some("The Forge"));
        assert_eq!(filters.depth.as_deref(), Some("5"));
        assert_eq!(filters.faction, None, "an unset field stays unset");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_absent_not_broken() {
        let file = std::env::temp_dir().join("eve_lpstore_definitely_not_here.json");
        assert!(matches!(load_from(&file), Loaded::Absent));
        assert_eq!(Loaded::Absent.reason(), None);
    }

    #[test]
    fn a_damaged_file_is_reported_with_a_reason() {
        let dir = std::env::temp_dir().join(format!("eve_lpstore_damaged_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for text in ["", "{ not json", "{\"depth\": 10}", "\"a string\"", "42"] {
            let file = dir.join("filters.json");
            std::fs::write(&file, text).unwrap();
            let loaded = load_from(&file);
            assert!(matches!(loaded, Loaded::Unusable(_)), "accepted {:?}", text);
            assert!(loaded.reason().is_some(), "no reason for {:?}", text);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_json_array_loads_as_no_remembered_filters() {
        // The derived deserializer accepts the sequence form, so an empty array is every field
        // absent rather than a damaged file. That is the same run the file being missing would
        // give, so there is nothing worth reporting.
        let filters: SavedFilters = serde_json::from_str("[]").unwrap();
        assert_eq!(filters, SavedFilters::default());
    }

    #[test]
    fn expire_and_the_command_line_only_fields_are_never_written() {
        let options = LpStoreOptions {
            expire: true,
            verbose: true,
            cache_minutes: Some("90".to_string()),
            concurrency: Some("8".to_string()),
            max_inflight: Some("6".to_string()),
            region: Some("Domain".to_string()),
            ..Default::default()
        };
        let text = serde_json::to_string_pretty(&SavedFilters::from(&options)).unwrap();
        for name in [
            "expire",
            "verbose",
            "cache_minutes",
            "concurrency",
            "max_inflight",
        ] {
            assert!(!text.contains(name), "{} reached the file: {}", name, text);
        }
        assert!(text.contains("Domain"));
    }

    #[test]
    fn the_command_line_wins_field_by_field() {
        let cli = LpStoreOptions {
            region: Some("Sinq Laison".to_string()),
            ..Default::default()
        };
        let merged = Loaded::Filters(all_fields()).merge(cli);
        assert_eq!(
            merged.region.as_deref(),
            Some("Sinq Laison"),
            "the flag wins"
        );
        assert_eq!(
            merged.lpstore.as_deref(),
            Some("1000002"),
            "the file fills the rest"
        );
        assert_eq!(merged.depth.as_deref(), Some("25"));
        assert_eq!(merged.faction.as_deref(), Some("caldari"));
    }

    #[test]
    fn an_absent_or_damaged_file_leaves_the_command_line_options_alone() {
        let cli = LpStoreOptions {
            faction: Some("amarr".to_string()),
            expire: true,
            ..Default::default()
        };
        let from_absent = Loaded::Absent.merge(cli.clone());
        assert_eq!(from_absent, cli);
        let from_damaged = Loaded::Unusable("bad json".to_string()).merge(cli.clone());
        assert_eq!(
            from_damaged, cli,
            "a damaged file must not change what runs"
        );
    }

    #[test]
    fn the_command_line_only_settings_survive_the_merge() {
        let cli = LpStoreOptions {
            expire: true,
            verbose: true,
            cache_minutes: Some("90".to_string()),
            concurrency: Some("8".to_string()),
            max_inflight: Some("6".to_string()),
            ..Default::default()
        };
        let merged = Loaded::Filters(all_fields()).merge(cli);
        assert!(
            merged.expire,
            "--expire came from the command line and stays"
        );
        assert!(merged.verbose);
        assert_eq!(merged.cache_minutes.as_deref(), Some("90"));
        assert_eq!(merged.concurrency.as_deref(), Some("8"));
        assert_eq!(merged.max_inflight.as_deref(), Some("6"));
    }

    #[test]
    fn a_saved_blank_does_not_become_a_filter_that_matches_nothing() {
        let filters = SavedFilters {
            lpstore: Some("   ".to_string()),
            region: Some("".to_string()),
            faction: Some("caldari".to_string()),
            ..Default::default()
        };
        let merged = filters.overlay(&LpStoreOptions::default());
        assert_eq!(merged.lpstore, None, "whitespace is absent");
        assert_eq!(merged.region, None);
        assert_eq!(merged.faction.as_deref(), Some("caldari"));
    }

    #[test]
    fn a_blank_from_the_command_line_keeps_its_own_meaning() {
        // This change must not alter how the flags behave, so an empty value typed on the command
        // line is passed through as it was, rather than quietly treated as absent.
        let cli = LpStoreOptions {
            region: Some(String::new()),
            ..Default::default()
        };
        let filters = SavedFilters {
            region: Some("Domain".to_string()),
            ..Default::default()
        };
        let merged = filters.overlay(&cli);
        assert_eq!(merged.region.as_deref(), Some(""));
    }

    #[test]
    fn clearing_every_field_in_the_form_forgets_the_saved_filters() {
        // Applying an all-empty form is how a user stops the view from remembering anything, so the
        // file has to hold the empties rather than keep the previous run's values.
        let saved = SavedFilters::from(&LpStoreOptions::default());
        assert_eq!(saved, SavedFilters::default());
        let merged = Loaded::Filters(saved).merge(LpStoreOptions::default());
        assert_eq!(merged, LpStoreOptions::default());
    }

    #[test]
    fn a_field_missing_from_the_file_loads_as_absent() {
        // A file from an older version, or one edited by hand, must still load.
        let filters: SavedFilters = serde_json::from_str("{\"region\": \"Domain\"}").unwrap();
        assert_eq!(filters.region.as_deref(), Some("Domain"));
        assert_eq!(filters.lpstore, None);
        assert_eq!(filters.depth, None);
    }

    #[test]
    fn an_unknown_field_in_the_file_does_not_stop_it_loading() {
        let filters: SavedFilters =
            serde_json::from_str("{\"region\": \"Domain\", \"warp_speed\": \"fast\"}").unwrap();
        assert_eq!(filters.region.as_deref(), Some("Domain"));
    }

    #[test]
    fn the_path_is_the_top_level_file_phase_8_relocates() {
        assert_eq!(path(), PathBuf::from("filters.json"));
    }
}

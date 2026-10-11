//! The filters the last `--tui` run was applied with, kept between runs.
//!
//! The view starts from what the command line asked for and the defaults behind it. That is the
//! right answer for a script and the wrong one for an interactive session, where the same handful
//! of corporations or the same region is usually wanted again. So a successful apply in the view
//! writes the filters it used to a file in the platform config directory, and the next session
//! reads them back before it shows the filters dialog.
//!
//! Only the text fields the form carries are kept. `--expire` is a one-shot cache bypass, and
//! remembering it would refetch every offer on every later run, and the command-line-only fields
//! are not shown by the form, so there is nothing to remember about them.

use crate::report::{Field, LpStoreOptions};
use serde::de::Error as _;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "filters.json";

/// Where the remembered filters live, inside the platform config directory.
pub fn path(config: &Path) -> PathBuf {
    config.join(FILE_NAME)
}

/// The filters a run was applied with, as they are written to disk.
///
/// Holds only the options [`Field::in_form`] names, so an option the form does not carry cannot be
/// persisted by accident. The file is an object keyed by each option's [`key`](crate::report::FieldSpec::key),
/// with `null` for an option that was not set. A key the file lacks reads as not set, so a file
/// written by an older version, or edited by hand, still loads, and a key it does not know is
/// ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedFilters(LpStoreOptions);

/// The options the file keeps, in the order it writes them.
fn kept() -> impl Iterator<Item = Field> {
    Field::ALL.into_iter().filter(|field| field.in_form())
}

impl SavedFilters {
    /// The remembered value of `field`, or `None` when it was not set or is not one the file keeps.
    pub fn value(&self, field: Field) -> &Option<String> {
        self.0.value(field)
    }

    /// The options a run starts from: `cli` for every field the command line set, these values for
    /// the rest. Fields that are `None` in both stay `None`, which is what makes clearing a field in
    /// the form and applying it the way to forget a filter.
    ///
    /// A saved value that is only whitespace is treated as absent, so a hand-edited empty field
    /// reads the way an emptied form field does rather than as a filter that matches nothing. An
    /// empty value the command line gave keeps its current meaning, because this must not change
    /// how the flags behave.
    pub fn overlay(&self, cli: &LpStoreOptions) -> LpStoreOptions {
        // The options the file does not keep, and expire and verbose, come from `cli` untouched.
        let mut options = cli.clone();
        for field in kept() {
            let value = options.value_mut(field);
            if value.is_none() {
                *value = self
                    .value(field)
                    .clone()
                    .filter(|saved| !saved.trim().is_empty());
            }
        }
        options
    }
}

impl From<&LpStoreOptions> for SavedFilters {
    fn from(options: &LpStoreOptions) -> Self {
        let mut saved = LpStoreOptions::default();
        for field in kept() {
            *saved.value_mut(field) = options.value(field).clone();
        }
        Self(saved)
    }
}

impl Serialize for SavedFilters {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut file = serializer.serialize_map(None)?;
        for field in kept() {
            file.serialize_entry(field.spec().key, self.value(field))?;
        }
        file.end()
    }
}

impl<'de> Deserialize<'de> for SavedFilters {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut file = HashMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let mut saved = LpStoreOptions::default();
        for field in kept() {
            let key = field.spec().key;
            *saved.value_mut(field) = match file.remove(key) {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::String(value)) => Some(value),
                Some(other) => {
                    return Err(D::Error::custom(format!(
                        "{key} is {other}, where a string or null was expected"
                    )));
                }
            };
        }
        Ok(Self(saved))
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
    /// The filters from the last applied run. Boxed because an unboxed variant the size of the
    /// struct next to a bare `String` is what `large_enum_variant` asks about:
    /// the whole enum would be that large for the one case that reads it once at start-up.
    Filters(Box<SavedFilters>),
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

/// Read the remembered filters from [`path`](path) inside `config`.
pub fn load(config: &Path) -> Loaded {
    load_from(&path(config))
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
        Ok(filters) => Loaded::Filters(Box::new(filters)),
        Err(error) => Loaded::Unusable(format!("{} is not valid JSON: {}", file.display(), error)),
    }
}

/// Write the persisted fields of `options` to `file`, as JSON that is readable and editable by
/// hand. Takes an explicit path so a test can write somewhere other than the config directory.
pub fn save_to(file: &Path, options: &LpStoreOptions) -> io::Result<()> {
    let text = serde_json::to_string_pretty(&SavedFilters::from(options))
        .expect("the saved filters always serialize");
    // The config directory is not created by anything else, and the first apply happens before
    // any other file has been written there.
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_fields() -> SavedFilters {
        SavedFilters::from(&LpStoreOptions {
            lpstore: Some("1000002".to_string()),
            region: Some("Domain".to_string()),
            security: Some("0.5..0.7".to_string()),
            agent_level: Some("4".to_string()),
            agent_division: Some("security".to_string()),
            agent_type: Some("research".to_string()),
            agent_region: Some("Black Rise".to_string()),
            agent_system: Some("4-3L53".to_string()),
            faction: Some("caldari".to_string()),
            system: Some("Jita".to_string()),
            depth: Some("25".to_string()),
            cache_minutes: Some("90".to_string()),
            ..Default::default()
        })
    }

    /// Saved filters holding only the given values.
    fn saved(values: &[(Field, &str)]) -> SavedFilters {
        let mut options = LpStoreOptions::default();
        for (field, value) in values {
            *options.value_mut(*field) = Some(value.to_string());
        }
        SavedFilters::from(&options)
    }

    #[test]
    fn the_saved_fields_round_trip_through_json() {
        let options = LpStoreOptions {
            lpstore: Some("Caldari Navy".to_string()),
            region: Some("The Forge".to_string()),
            depth: Some("5".to_string()),
            cache_minutes: Some("17".to_string()),
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
        assert_eq!(
            filters.value(Field::Lpstore).as_deref(),
            Some("Caldari Navy")
        );
        assert_eq!(filters.value(Field::Region).as_deref(), Some("The Forge"));
        assert_eq!(filters.value(Field::Depth).as_deref(), Some("5"));
        assert_eq!(filters.value(Field::CacheMinutes).as_deref(), Some("17"));
        assert_eq!(
            *filters.value(Field::Faction),
            None,
            "an unset field stays unset"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_written_before_the_option_table_still_loads_every_field() {
        // The shape the derived serializer wrote, field for field, in its order. The keys are the
        // table's `key`s, so nothing a user saved is lost to the change.
        let text = r#"{
  "lpstore": "1000002",
  "region": "Domain",
  "security": "0.5..0.7",
  "agent_level": "4",
  "agent_division": "security",
  "agent_type": "research",
  "agent_region": "Black Rise",
  "agent_system": "4-3L53",
  "faction": "caldari",
  "system": "Jita",
  "depth": "25",
  "cache_minutes": "90"
}"#;
        let filters: SavedFilters = serde_json::from_str(text).unwrap();
        assert_eq!(filters, all_fields());
    }

    #[test]
    fn every_kept_option_is_written_with_null_for_unset() {
        let text = serde_json::to_string(&saved(&[(Field::Region, "Domain")])).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let file = file.as_object().unwrap();
        assert_eq!(file.len(), kept().count());
        assert_eq!(file["region"], "Domain");
        assert!(file["lpstore"].is_null());
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
        for text in [
            "",
            "{ not json",
            "{\"depth\": 10}",
            "\"a string\"",
            "42",
            "[]",
        ] {
            let file = dir.join("filters.json");
            std::fs::write(&file, text).unwrap();
            let loaded = load_from(&file);
            assert!(matches!(loaded, Loaded::Unusable(_)), "accepted {:?}", text);
            assert!(loaded.reason().is_some(), "no reason for {:?}", text);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_json_array_is_not_read_by_position() {
        // The derived deserializer accepted the sequence form, so `["a", "b"]` set the first two
        // fields in declaration order. The file is an object; anything else is damaged.
        let error = serde_json::from_str::<SavedFilters>(r#"["Caldari Navy", "Domain"]"#);
        assert!(error.is_err());
    }

    #[test]
    fn a_wrongly_typed_value_names_its_key() {
        let error = serde_json::from_str::<SavedFilters>(r#"{"depth": 10}"#).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("depth is 10, where a string or null"),
            "{error}"
        );
    }

    #[test]
    fn expire_and_the_remaining_command_line_only_fields_are_never_written() {
        let options = LpStoreOptions {
            expire: true,
            verbose: true,
            cache_minutes: Some("90".to_string()),
            concurrency: Some("8".to_string()),
            max_inflight: Some("6".to_string()),
            cache_dir: Some("/tmp/elsewhere".to_string()),
            region: Some("Domain".to_string()),
            ..Default::default()
        };
        let text = serde_json::to_string_pretty(&SavedFilters::from(&options)).unwrap();
        for name in [
            "expire",
            "verbose",
            "concurrency",
            "max_inflight",
            "cache_dir",
        ] {
            assert!(!text.contains(name), "{} reached the file: {}", name, text);
        }
        assert!(
            text.contains("\"cache_minutes\": \"90\""),
            "the TUI setting is remembered: {text}"
        );
        assert!(text.contains("Domain"));
    }

    #[test]
    fn the_command_line_wins_field_by_field() {
        let cli = LpStoreOptions {
            region: Some("Sinq Laison".to_string()),
            cache_minutes: Some("120".to_string()),
            ..Default::default()
        };
        let merged = Loaded::Filters(Box::new(all_fields())).merge(cli);
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
        assert_eq!(
            merged.cache_minutes.as_deref(),
            Some("120"),
            "the CLI still wins"
        );
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
            cache_dir: Some("/tmp/elsewhere".to_string()),
            ..Default::default()
        };
        let merged = Loaded::Filters(Box::new(all_fields())).merge(cli);
        assert!(
            merged.expire,
            "--expire came from the command line and stays"
        );
        assert!(merged.verbose);
        assert_eq!(merged.cache_minutes.as_deref(), Some("90"));
        assert_eq!(merged.concurrency.as_deref(), Some("8"));
        assert_eq!(merged.max_inflight.as_deref(), Some("6"));
        assert_eq!(merged.cache_dir.as_deref(), Some("/tmp/elsewhere"));
    }

    #[test]
    fn a_saved_blank_does_not_become_a_filter_that_matches_nothing() {
        let filters = saved(&[
            (Field::Lpstore, "   "),
            (Field::Region, ""),
            (Field::Faction, "caldari"),
        ]);
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
        let merged = saved(&[(Field::Region, "Domain")]).overlay(&cli);
        assert_eq!(merged.region.as_deref(), Some(""));
    }

    #[test]
    fn clearing_every_field_in_the_form_forgets_the_saved_filters() {
        // Applying an all-empty form is how a user stops the view from remembering anything, so the
        // file has to hold the empties rather than keep the previous run's values.
        let saved = SavedFilters::from(&LpStoreOptions::default());
        assert_eq!(saved, SavedFilters::default());
        let merged = Loaded::Filters(Box::new(saved)).merge(LpStoreOptions::default());
        assert_eq!(merged, LpStoreOptions::default());
    }

    #[test]
    fn a_field_missing_from_the_file_loads_as_absent() {
        // A file from an older version, or one edited by hand, must still load.
        let filters: SavedFilters = serde_json::from_str("{\"region\": \"Domain\"}").unwrap();
        assert_eq!(filters.value(Field::Region).as_deref(), Some("Domain"));
        assert_eq!(*filters.value(Field::Lpstore), None);
        assert_eq!(*filters.value(Field::Depth), None);
    }

    #[test]
    fn an_unknown_field_in_the_file_does_not_stop_it_loading() {
        let filters: SavedFilters =
            serde_json::from_str("{\"region\": \"Domain\", \"warp_speed\": 5}").unwrap();
        assert_eq!(filters.value(Field::Region).as_deref(), Some("Domain"));
    }

    #[test]
    fn a_command_line_only_key_in_the_file_is_ignored() {
        // Hand-editing `concurrency` into the file must not make it a remembered setting.
        let filters: SavedFilters = serde_json::from_str("{\"concurrency\": \"8\"}").unwrap();
        assert_eq!(*filters.value(Field::Concurrency), None);
        assert_eq!(filters, SavedFilters::default());
    }

    #[test]
    fn the_file_sits_in_the_directory_it_is_given() {
        // It used to be top-level `filters.json`, which a future move of the roots would strand.
        let config = Path::new("/home/user/.config/eve_lpstore");
        assert_eq!(
            path(config),
            PathBuf::from("/home/user/.config/eve_lpstore/filters.json")
        );
    }
}

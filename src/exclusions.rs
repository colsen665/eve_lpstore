//! The offers excluded with `x` in the `--tui` view, kept between runs.
//!
//! An offer that is never wanted, such as one that needs a blueprint the user does not have, would
//! otherwise have to be excluded again in every session. So each change writes the excluded offers
//! to a file beside `filters.json`, and the next session reads them back before its first run.
//!
//! An exclusion is per offer ID, wherever the offer is held. Only the view reads the file:
//! `--lpstores` ignores it, so a script's report does not depend on what was set aside by hand.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "excluded_offers.json";

/// Where the excluded offers live, inside the platform config directory.
pub fn path(config: &Path) -> PathBuf {
    config.join(FILE_NAME)
}

/// The excluded offers: each offer ID with the type ID of its reward. The type is kept so the file
/// can name the item for a reader, also for an offer the report on screen does not hold. Ordered by
/// offer ID, which is the order the file is written in.
pub type Excluded = BTreeMap<i32, i32>;

/// One excluded offer as the file holds it. `item` is the reward's name, written for a person
/// reading or editing the file; it is not read back, so a stale or edited name changes nothing.
/// A key the file has and this does not is ignored.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    offer_id: i32,
    type_id: i32,
    #[serde(default, skip_deserializing)]
    item: String,
}

/// What was found where the excluded offers live. Three cases, as for the saved filters: no file
/// is the normal state before the first exclusion and is not reported, while a file that cannot
/// be used is.
#[derive(Debug)]
pub enum Loaded {
    /// No file is there, which is the state until an offer is excluded for the first time.
    Absent,
    /// The offers the file names.
    Offers(Excluded),
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

    /// The offers to start the session with. `Absent` and `Unusable` start with none.
    pub fn into_offers(self) -> Excluded {
        match self {
            Self::Offers(offers) => offers,
            Self::Absent | Self::Unusable(_) => Excluded::new(),
        }
    }
}

/// Read the excluded offers from [`path`](path) inside `config`.
pub fn load(config: &Path) -> Loaded {
    load_from(&path(config))
}

/// Read the excluded offers from `file`. Never fails: a missing file is [`Loaded::Absent`], and a
/// file that cannot be read or parsed is [`Loaded::Unusable`] with the reason. An offer listed
/// twice keeps the type of its last entry.
pub fn load_from(file: &Path) -> Loaded {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Absent,
        Err(error) => {
            return Loaded::Unusable(format!("could not read {}: {}", file.display(), error));
        }
    };
    match serde_json::from_str::<Vec<Entry>>(&text) {
        Ok(entries) => Loaded::Offers(
            entries
                .into_iter()
                .map(|entry| (entry.offer_id, entry.type_id))
                .collect(),
        ),
        Err(error) => Loaded::Unusable(format!("{} is not valid JSON: {}", file.display(), error)),
    }
}

/// Write `offers` to `file`, sorted by offer ID, each with the name `item_name` gives its reward
/// type. Takes an explicit path so a test can write somewhere other than the config directory.
pub fn save_to(
    file: &Path,
    offers: &Excluded,
    item_name: impl Fn(i32) -> String,
) -> io::Result<()> {
    let entries: Vec<Entry> = offers
        .iter()
        .map(|(&offer_id, &type_id)| Entry {
            offer_id,
            type_id,
            item: item_name(type_id),
        })
        .collect();
    let text =
        serde_json::to_string_pretty(&entries).expect("the excluded offers always serialize");
    // The config directory may not exist yet, when no filters have been applied either.
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of its own in the system temp directory, distinct per test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eve_lpstore_exclusions_{}_{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn name(type_id: i32) -> String {
        format!("Item {type_id}")
    }

    #[test]
    fn the_excluded_offers_round_trip_through_the_file() {
        let dir = temp_dir("roundtrip");
        let file = path(&dir);
        let offers = Excluded::from([(3, 30486), (1, 34), (2, 35)]);
        save_to(&file, &offers, name).unwrap();
        let loaded = load_from(&file);
        let Loaded::Offers(back) = loaded else {
            panic!("expected offers, got {loaded:?}");
        };
        assert_eq!(back, offers);
        assert_eq!(load(&dir).into_offers(), offers, "load reads the same file");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_is_sorted_by_offer_id_and_names_each_item() {
        let dir = temp_dir("sorted");
        let file = path(&dir);
        let offers = Excluded::from([(30, 7), (10, 9), (20, 8)]);
        save_to(&file, &offers, name).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let entries: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        let ids: Vec<i64> = entries
            .iter()
            .map(|entry| entry["offer_id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [10, 20, 30]);
        assert_eq!(entries[0]["type_id"], 9);
        assert_eq!(entries[0]["item"], "Item 9");
        assert_eq!(entries[0].as_object().unwrap().len(), 3, "{text}");
        // The same offers in another order write the same bytes.
        let again = dir.join("again.json");
        save_to(&again, &Excluded::from([(20, 8), (30, 7), (10, 9)]), name).unwrap();
        assert_eq!(std::fs::read_to_string(&again).unwrap(), text);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_absent_not_broken() {
        let dir = temp_dir("absent");
        let loaded = load(&dir);
        assert!(matches!(loaded, Loaded::Absent), "{loaded:?}");
        assert_eq!(loaded.reason(), None);
        assert!(loaded.into_offers().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_file_is_reported_with_a_reason_and_excludes_nothing() {
        let dir = temp_dir("damaged");
        let file = path(&dir);
        for text in [
            "",
            "[ not json",
            "{}",
            "42",
            r#"[{"offer_id": 1}]"#,
            r#"[{"offer_id": "1", "type_id": 34}]"#,
            r#"[{"type_id": 34}]"#,
        ] {
            std::fs::write(&file, text).unwrap();
            let loaded = load_from(&file);
            assert!(matches!(loaded, Loaded::Unusable(_)), "accepted {text:?}");
            assert!(loaded.reason().is_some(), "no reason for {text:?}");
            assert!(loaded.into_offers().is_empty());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_keys_and_the_item_name_are_ignored_on_load() {
        let loaded: Vec<Entry> = serde_json::from_str(
            r#"[{"offer_id": 5, "type_id": 34, "item": "Renamed by hand", "note": "why"}]"#,
        )
        .unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!((loaded[0].offer_id, loaded[0].type_id), (5, 34));
        assert_eq!(loaded[0].item, "", "the name is not read back");
        // Without the name at all, the entry still loads.
        let dir = temp_dir("unknown");
        let file = path(&dir);
        std::fs::write(
            &file,
            r#"[{"type_id": 34, "offer_id": 5, "extra": [1, 2]}]"#,
        )
        .unwrap();
        assert_eq!(load_from(&file).into_offers(), Excluded::from([(5, 34)]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_list_is_no_exclusions_rather_than_damage() {
        let dir = temp_dir("empty");
        let file = path(&dir);
        save_to(&file, &Excluded::new(), name).unwrap();
        let loaded = load_from(&file);
        assert!(
            matches!(&loaded, Loaded::Offers(offers) if offers.is_empty()),
            "{loaded:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_sits_beside_the_filters_in_the_directory_it_is_given() {
        let config = Path::new("/home/user/.config/eve_lpstore");
        assert_eq!(
            path(config),
            PathBuf::from("/home/user/.config/eve_lpstore/excluded_offers.json")
        );
        assert_eq!(
            path(config).parent(),
            crate::saved_filters::path(config).parent()
        );
    }
}

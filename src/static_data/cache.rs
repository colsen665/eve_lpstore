//! The four files kept in the data directory, and reading them back into a `StaticData`.

use super::agents::AgentRow;
use super::{StaticData, StaticDataError, SystemInfo};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const REGIONS_CACHE_FILE: &str = "regions.json";

const CORPORATIONS_CACHE_FILE: &str = "corporations.json";

const TYPES_CACHE_FILE: &str = "types.json";

const SECURITY_CACHE_FILE: &str = "corporation_agent_stations.json";

/// A single region entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RegionEntry {
    pub(super) region_id: i32,
    pub(super) name: String,
}

/// A single corporation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CorporationEntry {
    pub(super) corporation_id: i32,
    pub(super) name: String,
}

/// A single type entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TypeEntry {
    pub(super) type_id: i32,
    pub(super) name: String,
}

/// The agent data cached from the SDE.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct AgentCache {
    /// Bumped whenever the meaning of a stored field changes, so an old cache is refetched rather
    /// than read with the new shape. Version 2 added the owning region to each system. A cache
    /// written before this field existed has no key, which `serde(default)` reads as zero.
    #[serde(default)]
    pub(super) version: u32,
    pub(super) corporations: Vec<(i32, Vec<AgentRow>)>,
    pub(super) divisions: Vec<(i32, String)>,
    pub(super) agent_types: Vec<(i32, String)>,
    /// Id, name, true security, region and constellation.
    pub(super) systems: Vec<(i32, String, f64, i32, i32)>,
    pub(super) corporation_factions: Vec<(i32, i32)>,
    pub(super) factions: Vec<(i32, String)>,
    pub(super) stations: Vec<(i64, i32)>,
    /// Stargate connections, one entry per direction the archive lists, so both ways round for a
    /// gate between two systems.
    pub(super) gates: Vec<(i32, i32)>,
}

/// The shape `load` expects in the cached agent data. A file from an older build, or one written
/// before the version field existed, is treated as missing: the caller is then asked to download,
/// which is cheaper than silently reading a mapping that is not there.
///
/// Version 2 added each system's region, version 3 its constellation and the stargate connections.
pub(super) const AGENT_CACHE_VERSION: u32 = 3;

impl StaticData {
    /// The static data on disk under `data`, or `None` when it has to be downloaded: a file is
    /// missing, or the agent file is from an older build or cannot be read. An error means the
    /// files are there and current and still do not load, which a download is also the way past.
    ///
    /// Callers ask this before downloading, because a download is the whole static data archive,
    /// about 100 MB, and is worth saying so before it starts. The agent file is read once, for its
    /// version and for its contents: a cache from an older build exists and is the wrong shape, and
    /// the version field is how that is told apart.
    pub fn load_cached(data: &Path) -> Result<Option<Self>, StaticDataError> {
        let files = Self::files(data);
        if !files.iter().all(|path| path.exists()) {
            return Ok(None);
        }
        let Ok(agent_text) = std::fs::read_to_string(&files[3]) else {
            return Ok(None);
        };
        if agent_cache_version(&agent_text) != Some(AGENT_CACHE_VERSION) {
            return Ok(None);
        }
        Self::load_from_disk(&files, &agent_text).map(Some)
    }

    /// Download the archive, replace the four files under `data` with what it holds, and read
    /// them back.
    pub async fn download(data: &Path) -> Result<Self, StaticDataError> {
        Self::download_and_cache(data).await?;
        let files = Self::files(data);
        let agent_text =
            std::fs::read_to_string(&files[3]).map_err(|source| StaticDataError::Io {
                path: files[3].clone(),
                source,
            })?;
        Self::load_from_disk(&files, &agent_text)
    }

    /// The file holding the region names.
    fn regions_path(data: &Path) -> PathBuf {
        data.join(REGIONS_CACHE_FILE)
    }

    /// The file holding the corporation names.
    fn corporations_path(data: &Path) -> PathBuf {
        data.join(CORPORATIONS_CACHE_FILE)
    }

    /// The file holding the type names.
    fn types_path(data: &Path) -> PathBuf {
        data.join(TYPES_CACHE_FILE)
    }

    /// The file holding the agent, faction, system, and station data.
    fn security_path(data: &Path) -> PathBuf {
        data.join(SECURITY_CACHE_FILE)
    }

    /// The four files [`load_cached`](Self::load_cached) reads, in the order it reads them.
    pub(super) fn files(data: &Path) -> [PathBuf; 4] {
        [
            Self::regions_path(data),
            Self::corporations_path(data),
            Self::types_path(data),
            Self::security_path(data),
        ]
    }

    /// Load regions, corporations, and types from the first three of `files`, and the agent cache
    /// from `agent_text`, which the caller has already read from the fourth.
    fn load_from_disk(files: &[PathBuf; 4], agent_text: &str) -> Result<Self, StaticDataError> {
        let [regions_path, corporations_path, types_path, security_path] = files;
        let region_entries: Vec<RegionEntry> = read_json(regions_path)?;
        let mut regions = HashMap::with_capacity(region_entries.len());
        let mut region_names_by_id = HashMap::with_capacity(region_entries.len());
        for entry in region_entries {
            regions.insert(entry.name.to_lowercase(), entry.region_id);
            region_names_by_id.insert(entry.region_id, entry.name);
        }

        let corporation_entries: Vec<CorporationEntry> = read_json(corporations_path)?;
        let mut corporations = HashMap::with_capacity(corporation_entries.len());
        let mut corporation_names_by_id = HashMap::with_capacity(corporation_entries.len());
        for entry in corporation_entries {
            corporations.insert(entry.name.to_lowercase(), entry.corporation_id);
            corporation_names_by_id.insert(entry.corporation_id, entry.name);
        }

        let type_entries: Vec<TypeEntry> = read_json(types_path)?;
        let types = type_entries
            .into_iter()
            .map(|entry| (entry.type_id, entry.name))
            .collect();

        let agents: AgentCache =
            serde_json::from_str(agent_text).map_err(|source| StaticDataError::Json {
                path: security_path.clone(),
                source,
            })?;
        // Derived from `agents.systems` and `agents.gates`, which the struct below then moves.
        let constellation_members = Self::constellation_index(
            &agents
                .systems
                .iter()
                .map(|(id, _, _, _, constellation)| (*id, *constellation))
                .collect::<Vec<_>>(),
        );
        let neighbours = Self::neighbour_map(&agents.gates);

        Ok(Self {
            regions,
            region_names_by_id,
            corporations,
            corporation_names_by_id,
            types,
            corporation_agents: agents.corporations.into_iter().collect(),
            divisions: agents.divisions,
            agent_types: agents.agent_types,
            corporation_factions: agents.corporation_factions.into_iter().collect(),
            factions: agents.factions,
            stations: agents.stations,
            systems: agents
                .systems
                .into_iter()
                .map(|(id, name, true_sec, region_id, constellation_id)| {
                    let system = SystemInfo {
                        name,
                        true_sec,
                        region_id,
                        constellation_id,
                    };
                    (id, system)
                })
                .collect(),
            neighbours,
            constellation_members,
            burner_spreads: Mutex::new(HashMap::new()),
        })
    }
}

/// Writes `value` to `path` as indented JSON.
pub(super) async fn write_json<T: Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), StaticDataError> {
    let text = serde_json::to_string_pretty(value).map_err(|source| StaticDataError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    tokio::fs::write(path, text)
        .await
        .map_err(|source| StaticDataError::Io {
            path: path.to_path_buf(),
            source,
        })
}

/// The version recorded in the agent file's text, or `None` when it is not JSON. A file from
/// before the version field existed reports zero, not `None`.
fn agent_cache_version(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct VersionOnly {
        #[serde(default)]
        version: u32,
    }
    serde_json::from_str::<VersionOnly>(text)
        .ok()
        .map(|parsed| parsed.version)
}

/// Reads one of the cache files back.
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, StaticDataError> {
    let text = std::fs::read_to_string(path).map_err(|source| StaticDataError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| StaticDataError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data directory under the system temp directory, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eve_lpstore_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The smallest agent file of the current version that loads.
    fn current_agent_file() -> String {
        format!(
            r#"{{"version":{AGENT_CACHE_VERSION},"corporations":[],"divisions":[],"agent_types":[],"systems":[],"corporation_factions":[],"factions":[],"stations":[],"gates":[]}}"#
        )
    }

    #[test]
    fn a_damaged_cache_file_is_named_in_the_error() {
        // `load_cached` never downloads, so this cannot turn into a 100 MB fetch.
        let dir = scratch("damaged_cache");
        let [regions, corporations, types, security] = StaticData::files(&dir);
        for path in [&regions, &corporations] {
            std::fs::write(path, "[]").unwrap();
        }
        std::fs::write(&types, "not json").unwrap();
        std::fs::write(&security, current_agent_file()).unwrap();
        let error = StaticData::load_cached(&dir)
            .err()
            .expect("the types file does not parse");
        let named = types.display().to_string();
        assert!(
            error.to_string().starts_with(&format!("{named}: ")),
            "{error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_cached_agent_version_is_read_from_the_text() {
        // A file from before the version field existed reads as zero, not unreadable.
        assert_eq!(agent_cache_version(r#"{"corporations":[]}"#), Some(0));
        // The previous shape, which has each system's region but no constellation or gates.
        assert_eq!(agent_cache_version(r#"{"version":2}"#), Some(2));
        assert_eq!(
            agent_cache_version(&current_agent_file()),
            Some(AGENT_CACHE_VERSION)
        );
        assert_eq!(agent_cache_version("not json"), None);
    }

    #[test]
    fn a_stale_or_incomplete_cache_has_to_be_downloaded() {
        let dir = scratch("stale");
        for path in StaticData::files(&dir) {
            std::fs::write(path, "[]").unwrap();
        }
        // A version 2 file exists but is stale, which is the case the check is for. All four
        // files exist, so a check on existence alone would say yes and then read a mapping that
        // is not there.
        std::fs::write(StaticData::security_path(&dir), r#"{"version":2}"#).unwrap();
        assert!(matches!(StaticData::load_cached(&dir), Ok(None)), "stale");
        std::fs::write(StaticData::security_path(&dir), "not json").unwrap();
        assert!(
            matches!(StaticData::load_cached(&dir), Ok(None)),
            "unreadable"
        );
        std::fs::write(StaticData::security_path(&dir), current_agent_file()).unwrap();
        assert!(
            matches!(StaticData::load_cached(&dir), Ok(Some(_))),
            "current"
        );
        std::fs::remove_file(StaticData::types_path(&dir)).unwrap();
        assert!(
            matches!(StaticData::load_cached(&dir), Ok(None)),
            "a file missing"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

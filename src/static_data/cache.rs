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
    /// Whether every file [`load`](Self::load) reads from disk is there and current. It says nothing
    /// about whether they parse, so a `true` answer can still fail to load.
    ///
    /// `load` downloads on its own when this is `false`, which is a fetch of the whole static data
    /// archive, about 100 MB. Callers that want to ask before spending that, rather than
    /// finding out silently, check this first.
    ///
    /// The agent file is read rather than merely stat-ed, because a cache written by an older build
    /// exists and is the wrong shape: a version field is how that is told apart, and it cannot be
    /// seen without opening the file.
    pub fn is_cached(data: &Path) -> bool {
        if !Self::files(data).iter().all(|path| path.exists()) {
            return false;
        }
        Self::agent_cache_version(data) == Some(AGENT_CACHE_VERSION)
    }

    /// The version recorded in the cached agent data, or `None` when it cannot be read. A file from
    /// before the version field existed reports zero, not `None`, so only an unreadable file is
    /// `None`.
    fn agent_cache_version(data: &Path) -> Option<u32> {
        let text = std::fs::read_to_string(Self::security_path(data)).ok()?;
        #[derive(Deserialize)]
        struct VersionOnly {
            #[serde(default)]
            version: u32,
        }
        let parsed: VersionOnly = serde_json::from_str(&text).ok()?;
        Some(parsed.version)
    }

    /// Load static data from the local cache under `data`.
    ///
    /// If `force_download` is `true`, the cache is re-downloaded from the SDE even if a local copy
    /// exists. If `false`, the local cache is used when present; otherwise a download is
    /// triggered.
    pub async fn load(force_download: bool, data: &Path) -> Result<Self, StaticDataError> {
        if force_download || !Self::is_cached(data) {
            Self::download_and_cache(data).await?;
        }
        let [regions, corporations, types, security] = Self::files(data);
        Self::load_from_disk(&regions, &corporations, &types, &security)
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

    /// The four files [`load`](Self::load) reads, in the order it reads them.
    pub(super) fn files(data: &Path) -> [PathBuf; 4] {
        [
            Self::regions_path(data),
            Self::corporations_path(data),
            Self::types_path(data),
            Self::security_path(data),
        ]
    }

    /// Load regions, corporations, types, and the agent cache from the four files under the data
    /// directory.
    fn load_from_disk(
        regions_path: &Path,
        corporations_path: &Path,
        types_path: &Path,
        security_path: &Path,
    ) -> Result<Self, StaticDataError> {
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

        let agents: AgentCache = read_json(security_path)?;
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

    #[test]
    fn a_damaged_cache_file_is_named_in_the_error() {
        // Read through `load_from_disk` rather than `load`, so a regression in `is_cached` cannot
        // turn this test into a download.
        let dir =
            std::env::temp_dir().join(format!("eve_lpstore_damaged_cache_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let [regions, corporations, types, security] = StaticData::files(&dir);
        for path in [&regions, &corporations, &security] {
            std::fs::write(path, "[]").unwrap();
        }
        std::fs::write(&types, "not json").unwrap();
        let error = StaticData::load_from_disk(&regions, &corporations, &types, &security)
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
    fn the_cached_agent_version_is_read_from_the_file() {
        let dir = std::env::temp_dir().join(format!("eve_lpstore_ver_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = StaticData::security_path(&dir);

        // A file from before the version field existed reads as zero, not unreadable.
        std::fs::write(&file, r#"{"corporations":[]}"#).unwrap();
        assert_eq!(StaticData::agent_cache_version(&dir), Some(0));
        // The previous shape, which has each system's region but no constellation or gates.
        std::fs::write(&file, r#"{"version":2}"#).unwrap();
        assert_eq!(StaticData::agent_cache_version(&dir), Some(2));
        std::fs::write(&file, format!(r#"{{"version":{AGENT_CACHE_VERSION}}}"#)).unwrap();
        assert_eq!(
            StaticData::agent_cache_version(&dir),
            Some(AGENT_CACHE_VERSION)
        );
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(StaticData::agent_cache_version(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_agent_cache_is_not_reported_as_cached() {
        let dir = std::env::temp_dir().join(format!("eve_lpstore_stale_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for path in StaticData::files(&dir) {
            std::fs::write(path, "[]").unwrap();
        }
        // A version 2 file exists but is stale, which is the case the gate needs to catch. All four
        // files exist, so a purely existence-based check would say yes and then read a mapping
        // that is not there.
        std::fs::write(StaticData::security_path(&dir), r#"{"version":2}"#).unwrap();
        assert!(!StaticData::is_cached(&dir), "a stale shape is not cached");
        std::fs::write(
            StaticData::security_path(&dir),
            format!(r#"{{"version":{AGENT_CACHE_VERSION}}}"#),
        )
        .unwrap();
        assert!(StaticData::is_cached(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

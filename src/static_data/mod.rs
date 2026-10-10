use crate::esi::error::EsiError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

const SDE_ZIP_URL: &str =
    "https://developers.eveonline.com/static-data/eve-online-static-data-latest-jsonl.zip";

const CACHE_DIR: &str = "data";
const REGIONS_CACHE_FILE: &str = "regions.json";
const CORPORATIONS_CACHE_FILE: &str = "corporations.json";
const TYPES_CACHE_FILE: &str = "types.json";
const SECURITY_CACHE_FILE: &str = "corporation_agent_stations.json";

/// Displayed security is rounded as described at https://wiki.eveuniversity.org/System_security
///
/// Returns the displayed security in tenths, so 0.5 is `5` and -0.5 is `-5`.
pub fn displayed_security(true_sec: f64) -> i32 {
    if true_sec > 0.0 && true_sec < 0.05 {
        return 1;
    }
    (true_sec * 10.0).round() as i32
}

/// A security range, compared on displayed security. Both bounds are inclusive, and either side
/// being `None` leaves that end open, so `0.5..0.7` is `low: 5, high: 7`, `0.5..` is
/// `low: 5, high: None`, and `0.5` alone is both bounds equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityFilter {
    low: Option<i32>,
    high: Option<i32>,
}

impl SecurityFilter {
    /// Parse a level or a range: `0.5` exactly, `0.5..0.7` inclusive on both bounds, `0.5..` at
    /// least, and `..0.7` at most.
    ///
    /// The old `<op><level>` forms are refused rather than reinterpreted. A bare value starting
    /// with `-` is the reason: `-0.5` used to mean *at most 0.5* and now reads as the level
    /// *minus a half*, so accepting it would quietly filter different corporations. Write
    /// `..0.5` for *at most*, or `-0.5..-0.5` for that level exactly.
    pub fn parse(value: &str) -> Result<Self, String> {
        // Surrounding space is noise on a value that came from a quoted argument or a form field,
        // and an all-blank side counts as an open one so ".. " reads as the bare ".." it means.
        let value = value.trim();
        if let Some((low, high)) = value.split_once("..") {
            let (low, high) = (low.trim(), high.trim());
            let low = if low.is_empty() {
                None
            } else {
                Some(level(low)?)
            };
            let high = if high.is_empty() {
                None
            } else {
                Some(level(high)?)
            };
            match (low, high) {
                (None, None) => {
                    return Err(format!(
                        "'{value}' bounds nothing. Use 0.5..0.7, 0.5.., or ..0.7"
                    ));
                }
                (Some(low), Some(high)) if low > high => {
                    return Err(format!(
                        "the lower bound {} is above the upper bound {}",
                        show(low),
                        show(high)
                    ));
                }
                _ => return Ok(Self { low, high }),
            }
        }

        // A level on its own, unless it carries an old operator.
        if let Some(rest) = value.strip_prefix('+') {
            return Err(format!(
                "'{value}' no longer uses +. Write '{rest}..' for at least {}",
                show_or_raw(rest)
            ));
        }
        if let Some(rest) = value.strip_prefix('=') {
            return Err(format!(
                "'{value}' no longer uses =. Write '{rest}' on its own for exactly {}",
                show_or_raw(rest)
            ));
        }
        if value.starts_with('-') {
            return Err(format!(
                "'{value}' is ambiguous: '-x' used to mean at most x. Write '..{}' for at most \
                 {}, or '{value}..{value}' for that level exactly",
                value.strip_prefix('-').unwrap_or(value),
                show_or_raw(value.strip_prefix('-').unwrap_or(value)),
            ));
        }
        let only = level(value)?;
        Ok(Self {
            low: Some(only),
            high: Some(only),
        })
    }

    fn matches(self, tenths: i32) -> bool {
        self.low.map_or(true, |low| tenths >= low) && self.high.map_or(true, |high| tenths <= high)
    }
}

/// One bound, in the displayed tenths the filter compares on, so `0.45` lands on `0.5` the same
/// way a single level always has.
fn level(text: &str) -> Result<i32, String> {
    let parsed: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number"))?;
    if !(-1.0..=1.0).contains(&parsed) {
        return Err("the level must be between -1.0 and 1.0".to_string());
    }
    Ok(displayed_security(parsed))
}

/// A bound in tenths back to the spelling the user typed.
fn show(tenths: i32) -> String {
    format!("{:.1}", tenths as f64 / 10.0)
}

/// Echo a level as written when it does not convert, so an error about `+abc` does not claim
/// `at least 0`.
fn show_or_raw(text: &str) -> String {
    match text.parse::<f64>() {
        Ok(parsed) if (-1.0..=1.0).contains(&parsed) => show(displayed_security(parsed)),
        _ => text.to_string(),
    }
}

/// A single region entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionEntry {
    pub region_id: i32,
    pub name: String,
}

/// Raw JSONL record from the SDE ZIP.
#[derive(Debug, Deserialize)]
struct SdeRegionRecord {
    #[serde(rename = "_key")]
    region_id: i32,
    name: SdeRegionName,
}

#[derive(Debug, Deserialize)]
struct SdeRegionName {
    #[serde(rename = "en")]
    en: String,
}

/// A single corporation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorporationEntry {
    pub corporation_id: i32,
    pub name: String,
}

/// Raw JSONL record from the SDE ZIP for corporations.
#[derive(Debug, Deserialize)]
struct SdeCorporationRecord {
    #[serde(rename = "_key")]
    corporation_id: i32,
    name: SdeCorporationName,
}

#[derive(Debug, Deserialize)]
struct SdeCorporationName {
    #[serde(rename = "en")]
    en: String,
}

/// A single type entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeEntry {
    pub type_id: i32,
    pub name: String,
}

/// Raw JSONL record from the SDE ZIP for types.
#[derive(Debug, Deserialize)]
struct SdeTypeRecord {
    #[serde(rename = "_key")]
    type_id: i32,
    name: SdeTypeName,
}

/// NPC station record, used to find each station's solar system.
#[derive(Debug, Deserialize)]
struct SdeNpcStation {
    #[serde(rename = "_key")]
    station_id: i32,
    #[serde(rename = "solarSystemID")]
    solar_system_id: i32,
}

/// Solar system record, used for its name and security status.
#[derive(Debug, Deserialize)]
struct SdeSolarSystem {
    #[serde(rename = "_key")]
    system_id: i32,
    #[serde(rename = "securityStatus")]
    security_status: f64,
    name: SdeSystemName,
}

#[derive(Debug, Deserialize)]
struct SdeSystemName {
    en: String,
}

/// Agent details on an NPC character record.
#[derive(Debug, Deserialize)]
struct SdeAgent {
    level: i32,
    #[serde(rename = "agentTypeID")]
    agent_type_id: i32,
    #[serde(rename = "divisionID")]
    division_id: i32,
}

/// Agent division record, named by its internal name.
#[derive(Debug, Deserialize)]
struct SdeDivision {
    #[serde(rename = "_key")]
    division_id: i32,
    #[serde(rename = "internalName")]
    internal_name: String,
}

/// Agent type record.
#[derive(Debug, Deserialize)]
struct SdeAgentType {
    #[serde(rename = "_key")]
    agent_type_id: i32,
    name: String,
}

/// NPC character record. Characters with an `agent` field are agents.
#[derive(Debug, Deserialize)]
struct SdeNpcCharacter {
    #[serde(rename = "corporationID")]
    corporation_id: i32,
    #[serde(rename = "locationID", default)]
    location_id: Option<i32>,
    #[serde(default)]
    agent: Option<SdeAgent>,
}

#[derive(Debug, Deserialize)]
struct SdeTypeName {
    #[serde(rename = "en")]
    en: String,
}

/// One agent's filterable details: level, displayed security in tenths, division, and type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentRow {
    pub level: i32,
    pub tenths: i32,
    pub division_id: i32,
    pub agent_type_id: i32,
    pub system_id: i32,
}

/// The agent data cached from the SDE.
#[derive(Debug, Serialize, Deserialize)]
struct AgentCache {
    corporations: Vec<(i32, Vec<AgentRow>)>,
    divisions: Vec<(i32, String)>,
    agent_types: Vec<(i32, String)>,
    systems: Vec<(i32, String, f64)>,
    corporation_factions: Vec<(i32, i32)>,
    factions: Vec<(i32, String)>,
    stations: Vec<(i64, i32)>,
}

/// NPC corporation record, used for its faction.
#[derive(Debug, Deserialize)]
struct SdeFactionCorporation {
    #[serde(rename = "_key")]
    corporation_id: i32,
    #[serde(rename = "factionID", default)]
    faction_id: Option<i32>,
}

/// Faction record, named by its English name.
#[derive(Debug, Deserialize)]
struct SdeFaction {
    #[serde(rename = "_key")]
    faction_id: i32,
    name: SdeSystemName,
}

/// Conditions that one agent must all satisfy. A missing condition matches any agent.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentFilter {
    pub security: Option<SecurityFilter>,
    pub level: Option<i32>,
    pub division_id: Option<i32>,
    pub agent_type_id: Option<i32>,
}

impl AgentFilter {
    fn matches(&self, row: &AgentRow) -> bool {
        self.security.map_or(true, |filter| filter.matches(row.tenths))
            && self.level.map_or(true, |level| row.level == level)
            && self.division_id.map_or(true, |division| row.division_id == division)
            && self.agent_type_id.map_or(true, |agent_type| row.agent_type_id == agent_type)
    }
}

/// Static data cache for region, corporation, and type name → ID lookups.
///
/// Data is cached locally at `data/regions.json`, `data/corporations.json`, and `data/types.json`.
/// The cache is only downloaded when explicitly forced or when no cache file exists.
///
/// The default is an empty cache: every lookup misses and callers fall back to IDs. That is a
/// usable value, and it lets the `--tui` view be render-tested without the `data/` files, which
/// are gitignored and downloaded on demand.
#[derive(Default)]
pub struct StaticData {
    regions: HashMap<String, i32>,
    region_names_by_id: HashMap<i32, String>,
    corporations: HashMap<String, i32>,
    corporation_names_by_id: HashMap<i32, String>,
    types: HashMap<i32, String>,
    corporation_agents: HashMap<i32, Vec<AgentRow>>,
    divisions: Vec<(i32, String)>,
    agent_types: Vec<(i32, String)>,
    systems: HashMap<i32, (String, f64)>,
    corporation_factions: HashMap<i32, i32>,
    factions: Vec<(i32, String)>,
    stations: Vec<(i64, i32)>,
}

impl StaticData {
    /// Whether every file [`load`](Self::load) reads from disk is there. It says nothing about
    /// whether they parse, so a `true` answer can still fail to load.
    ///
    /// `load` downloads on its own when this is `false`, which is a several hundred megabyte fetch
    /// of the whole static data archive. Callers that want to ask before spending that, rather than
    /// finding out silently, check this first.
    pub fn is_cached() -> bool {
        Self::regions_cache_path().exists()
            && Self::corporations_cache_path().exists()
            && Self::types_cache_path().exists()
            && Self::security_cache_path().exists()
    }

    /// Load static data from local cache.
    ///
    /// If `force_download` is `true`, the cache is re-downloaded from the SDE
    /// even if a local copy exists. If `false`, the local cache is used when
    /// present; otherwise a download is triggered.
    pub async fn load(force_download: bool) -> Result<Self, EsiError> {
        let regions_path = Self::regions_cache_path();
        let corporations_path = Self::corporations_cache_path();
        let types_path = Self::types_cache_path();
        let security_path = Self::security_cache_path();

        if force_download || !Self::is_cached() {
            Self::download_and_cache(&regions_path, &corporations_path, &types_path, &security_path)
                .await?;
        }

        Self::load_from_disk(&regions_path, &corporations_path, &types_path, &security_path)
    }

    /// Look up a region ID by name (case-insensitive).
    pub fn region_id(&self, name: &str) -> Option<i32> {
        self.regions.get(&name.to_lowercase()).copied()
    }

    /// Get all known region names, with their proper capitalization.
    pub fn region_names(&self) -> impl Iterator<Item = &String> {
        self.region_names_by_id.values()
    }

    /// Look up a region name by region ID.
    pub fn region_name(&self, region_id: i32) -> Option<&String> {
        self.region_names_by_id.get(&region_id)
    }

    /// Look up a corporation ID by name (case-insensitive).
    pub fn corporation_id(&self, name: &str) -> Option<i32> {
        self.corporations.get(&name.to_lowercase()).copied()
    }

    /// Get all known corporation names.
    pub fn corporation_names(&self) -> impl Iterator<Item = &String> {
        self.corporations.keys()
    }

    /// Get all known corporation IDs.
    pub fn corporation_name(&self, corporation_id: i32) -> Option<&String> {
        self.corporation_names_by_id.get(&corporation_id)
    }

    pub fn corporation_ids(&self) -> impl Iterator<Item = i32> {
        self.corporations.values().copied()
    }

    /// Get all known solar system names.
    pub fn system_names(&self) -> impl Iterator<Item = &String> {
        self.systems.values().map(|(name, _)| name)
    }

    /// Look up a type name by type ID.
    pub fn type_name(&self, type_id: i32) -> Option<&String> {
        self.types.get(&type_id)
    }

    /// Returns the path to the local regions cache file.
    pub fn regions_cache_path() -> PathBuf {
        PathBuf::from(CACHE_DIR).join(REGIONS_CACHE_FILE)
    }

    /// Returns the path to the local corporations cache file.
    pub fn corporations_cache_path() -> PathBuf {
        PathBuf::from(CACHE_DIR).join(CORPORATIONS_CACHE_FILE)
    }

    /// Returns the path to the local types cache file.
    pub fn types_cache_path() -> PathBuf {
        PathBuf::from(CACHE_DIR).join(TYPES_CACHE_FILE)
    }

    /// Returns the path to the local hi-sec corporations cache file.
    pub fn security_cache_path() -> PathBuf {
        PathBuf::from(CACHE_DIR).join(SECURITY_CACHE_FILE)
    }

    /// Whether one agent of the corporation satisfies every condition in the filter.
    pub fn corporation_passes(&self, corporation_id: i32, filter: &AgentFilter) -> bool {
        self.corporation_agents
            .get(&corporation_id)
            .is_some_and(|rows| rows.iter().any(|row| filter.matches(row)))
    }

    /// Names of the systems where the corporation's agents matching the filter are, in data order.
    pub fn corporation_systems(&self, corporation_id: i32, filter: &AgentFilter) -> Vec<(String, f64)> {
        let mut names: Vec<(String, f64)> = Vec::new();
        for row in self.corporation_agents.get(&corporation_id).into_iter().flatten() {
            if !filter.matches(row) {
                continue;
            }
            if let Some((name, true_sec)) = self.systems.get(&row.system_id) {
                if !names.iter().any(|(existing, _)| existing == name) {
                    names.push((name.clone(), *true_sec));
                }
            }
        }
        names
    }

    /// The solar system ID for a system name or ID, with names matched case-insensitively.
    pub fn system_id(&self, value: &str) -> Option<i32> {
        if let Ok(id) = value.parse::<i32>() {
            if self.systems.contains_key(&id) {
                return Some(id);
            }
        }
        self.systems
            .iter()
            .find(|(_, (name, _))| name.eq_ignore_ascii_case(value))
            .map(|(id, _)| *id)
    }

    /// The NPC station IDs located in the solar system.
    pub fn station_ids_in_system(&self, system_id: i32) -> HashSet<i64> {
        self.stations
            .iter()
            .filter(|(_, system)| *system == system_id)
            .map(|(station_id, _)| *station_id)
            .collect()
    }

    /// Whether the corporation belongs to the faction.
    pub fn corporation_in_faction(&self, corporation_id: i32, faction_id: i32) -> bool {
        self.corporation_factions.get(&corporation_id) == Some(&faction_id)
    }

    /// The faction ID for a name such as `caldari` or `Caldari State`, matched case-insensitively.
    pub fn faction_id(&self, name: &str) -> Option<i32> {
        self.factions.iter().find_map(|(id, full)| {
            let full = full.to_lowercase();
            let short = Self::faction_short_name(&full);
            (name.eq_ignore_ascii_case(&full) || name.eq_ignore_ascii_case(&short)).then_some(*id)
        })
    }

    /// Short faction names accepted by `faction_id`, such as `caldari`.
    pub fn faction_names(&self) -> Vec<String> {
        self.factions
            .iter()
            .map(|(_, full)| Self::faction_short_name(&full.to_lowercase()))
            .collect()
    }

    fn faction_short_name(full: &str) -> String {
        let stripped = full.strip_prefix("the ").unwrap_or(full);
        stripped.split_whitespace().next().unwrap_or(stripped).to_string()
    }

    /// The division ID for an internal name, matched case-insensitively.
    pub fn division_id(&self, name: &str) -> Option<i32> {
        self.divisions
            .iter()
            .find(|(_, internal)| internal.eq_ignore_ascii_case(name))
            .map(|(id, _)| *id)
    }

    /// Division names accepted by `division_id`.
    pub fn division_names(&self) -> Vec<String> {
        self.divisions.iter().map(|(_, internal)| internal.to_lowercase()).collect()
    }

    /// The agent type ID for a name such as `basic` or `BasicAgent`, matched case-insensitively.
    pub fn agent_type_id(&self, name: &str) -> Option<i32> {
        self.agent_types.iter().find_map(|(id, full)| {
            let full = full.to_lowercase();
            let short = full.strip_suffix("agent").unwrap_or(&full);
            (name.eq_ignore_ascii_case(&full) || name.eq_ignore_ascii_case(short)).then_some(*id)
        })
    }

    /// Short agent type names accepted by `agent_type_id`, without the `Agent` suffix.
    pub fn agent_type_names(&self) -> Vec<String> {
        self.agent_types
            .iter()
            .map(|(_, full)| {
                let full = full.to_lowercase();
                full.strip_suffix("agent").unwrap_or(&full).to_string()
            })
            .collect()
    }

    /// Download the SDE ZIP, extract regions, corporations, and types, and write to cache files.
    async fn download_and_cache(
        regions_path: &std::path::Path,
        corporations_path: &std::path::Path,
        types_path: &std::path::Path,
        security_path: &std::path::Path,
    ) -> Result<(), EsiError> {
        if let Some(parent) = regions_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(EsiError::Io)?;
        }

        let response = reqwest::get(SDE_ZIP_URL)
            .await
            .map_err(EsiError::Http)?
            .error_for_status()
            .map_err(EsiError::Http)?;

        let bytes = response.bytes().await.map_err(EsiError::Http)?;

        // Extract regions, corporations, and types from the ZIP in memory
        let regions = Self::extract_regions(&bytes)?;
        let corporations = Self::extract_corporations(&bytes)?;
        let types = Self::extract_types(&bytes)?;
        let agents = Self::extract_agent_cache(&bytes)?;

        // Serialize to JSON and write to cache files
        let regions_json =
            serde_json::to_string_pretty(&regions).map_err(EsiError::Deserialize)?;
        tokio::fs::write(regions_path, regions_json)
            .await
            .map_err(EsiError::Io)?;

        let corporations_json =
            serde_json::to_string_pretty(&corporations).map_err(EsiError::Deserialize)?;
        tokio::fs::write(corporations_path, corporations_json)
            .await
            .map_err(EsiError::Io)?;

        let types_json =
            serde_json::to_string_pretty(&types).map_err(EsiError::Deserialize)?;
        tokio::fs::write(types_path, types_json)
            .await
            .map_err(EsiError::Io)?;

        let security_json =
            serde_json::to_string_pretty(&agents).map_err(EsiError::Deserialize)?;
        tokio::fs::write(security_path, security_json)
            .await
            .map_err(EsiError::Io)?;

        Ok(())
    }

    /// Extract region data from the SDE ZIP bytes.
    fn extract_regions(bytes: &[u8]) -> Result<Vec<RegionEntry>, EsiError> {
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Find the regions file in the archive
        let regions_filename = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .find(|name| name.to_lowercase().contains("region"))
            .ok_or_else(|| {
                EsiError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no regions file found in SDE ZIP",
                ))
            })?;

        let mut file = zip.by_name(&regions_filename).map_err(|e| {
            EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content).map_err(EsiError::Io)?;

        // Parse JSONL: each line is a record
        let mut regions = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: SdeRegionRecord =
                serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            regions.push(RegionEntry {
                region_id: record.region_id,
                name: record.name.en,
            });
        }

        Ok(regions)
    }

    /// Extract corporation data from the SDE ZIP bytes.
    fn extract_corporations(bytes: &[u8]) -> Result<Vec<CorporationEntry>, EsiError> {
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Find the corporations file in the archive
        let corporations_filename = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .find(|name| name.to_lowercase().contains("corporation"))
            .ok_or_else(|| {
                EsiError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no corporations file found in SDE ZIP",
                ))
            })?;

        let mut file = zip.by_name(&corporations_filename).map_err(|e| {
            EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content).map_err(EsiError::Io)?;

        // Parse JSONL: each line is a record
        let mut corporations = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: SdeCorporationRecord =
                serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            corporations.push(CorporationEntry {
                corporation_id: record.corporation_id,
                name: record.name.en,
            });
        }

        Ok(corporations)
    }

    /// Extract type data from the SDE ZIP bytes.
    fn extract_types(bytes: &[u8]) -> Result<Vec<TypeEntry>, EsiError> {
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Find the types file in the archive (exact match to avoid typeDogma, typeBonus, etc.)
        let types_filename = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .find(|name| name.eq_ignore_ascii_case("types.jsonl"))
            .ok_or_else(|| {
                EsiError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no types file found in SDE ZIP",
                ))
            })?;

        let mut file = zip.by_name(&types_filename).map_err(|e| {
            EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content).map_err(EsiError::Io)?;

        // Parse JSONL: each line is a record
        let mut types = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: SdeTypeRecord =
                serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            types.push(TypeEntry {
                type_id: record.type_id,
                name: record.name.en,
            });
        }

        Ok(types)
    }

    /// Read one file from the SDE ZIP as text.
    fn read_sde_file(bytes: &[u8], name: &str) -> Result<String, EsiError> {
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        let mut file = zip
            .by_name(name)
            .map_err(|e| EsiError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
        let mut content = String::new();
        file.read_to_string(&mut content).map_err(EsiError::Io)?;
        Ok(content)
    }

    /// Extract, for each corporation with agents, the displayed security of the systems its agents are in.
    ///
    /// Agents are NPC characters with an `agent` field. Each agent's location is an NPC
    /// station, whose solar system's security status gives the displayed value.
    fn extract_agent_cache(bytes: &[u8]) -> Result<AgentCache, EsiError> {
        let mut station_systems = HashMap::new();
        for line in Self::read_sde_file(bytes, "npcStations.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeNpcStation = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            station_systems.insert(record.station_id, record.solar_system_id);
        }

        let mut system_names = Vec::new();
        let mut system_security = HashMap::new();
        for line in Self::read_sde_file(bytes, "mapSolarSystems.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeSolarSystem = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            system_names.push((record.system_id, record.name.en, record.security_status));
            system_security.insert(record.system_id, record.security_status);
        }

        let mut corporation_factions = Vec::new();
        for line in Self::read_sde_file(bytes, "npcCorporations.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeFactionCorporation =
                serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            if let Some(faction_id) = record.faction_id {
                corporation_factions.push((record.corporation_id, faction_id));
            }
        }

        let mut factions = Vec::new();
        for line in Self::read_sde_file(bytes, "factions.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeFaction = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            factions.push((record.faction_id, record.name.en));
        }

        let mut divisions = Vec::new();
        for line in Self::read_sde_file(bytes, "npcCorporationDivisions.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeDivision = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            divisions.push((record.division_id, record.internal_name));
        }

        let mut agent_types = Vec::new();
        for line in Self::read_sde_file(bytes, "agentTypes.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeAgentType = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            agent_types.push((record.agent_type_id, record.name));
        }

        let mut corporations: HashMap<i32, Vec<AgentRow>> = HashMap::new();
        for line in Self::read_sde_file(bytes, "npcCharacters.jsonl")?.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SdeNpcCharacter = serde_json::from_str(line).map_err(EsiError::Deserialize)?;
            let Some(agent) = record.agent else {
                continue;
            };
            let Some(location_id) = record.location_id else {
                continue;
            };
            let Some(system_id) = station_systems.get(&location_id) else {
                continue;
            };
            if let Some(true_sec) = system_security.get(system_id) {
                let row = AgentRow {
                    level: agent.level,
                    tenths: displayed_security(*true_sec),
                    division_id: agent.division_id,
                    agent_type_id: agent.agent_type_id,
                    system_id: *system_id,
                };
                let rows = corporations.entry(record.corporation_id).or_default();
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }

        let stations: Vec<(i64, i32)> = station_systems
            .iter()
            .map(|(station_id, system_id)| (*station_id as i64, *system_id))
            .collect();

        Ok(AgentCache {
            corporations: corporations
                .into_iter()
                .map(|(corporation_id, rows)| (corporation_id, rows.into_iter().collect()))
                .collect(),
            divisions,
            agent_types,
            systems: system_names,
            corporation_factions,
            factions,
            stations,
        })
    }

    /// Load regions, corporations, types, and corporation security from local cache files.
    fn load_from_disk(
        regions_path: &std::path::Path,
        corporations_path: &std::path::Path,
        types_path: &std::path::Path,
        security_path: &std::path::Path,
    ) -> Result<Self, EsiError> {
        let regions_text = std::fs::read_to_string(regions_path).map_err(EsiError::Io)?;
        let region_entries: Vec<RegionEntry> =
            serde_json::from_str(&regions_text).map_err(EsiError::Deserialize)?;

        let mut regions = HashMap::with_capacity(region_entries.len());
        let mut region_names_by_id = HashMap::with_capacity(region_entries.len());
        for entry in region_entries {
            regions.insert(entry.name.to_lowercase(), entry.region_id);
            region_names_by_id.insert(entry.region_id, entry.name);
        }

        let corporations_text =
            std::fs::read_to_string(corporations_path).map_err(EsiError::Io)?;
        let corporation_entries: Vec<CorporationEntry> =
            serde_json::from_str(&corporations_text).map_err(EsiError::Deserialize)?;

        let mut corporations = HashMap::with_capacity(corporation_entries.len());
        let mut corporation_names_by_id = HashMap::with_capacity(corporation_entries.len());
        for entry in corporation_entries {
            corporations.insert(entry.name.to_lowercase(), entry.corporation_id);
            corporation_names_by_id.insert(entry.corporation_id, entry.name);
        }

        let types_text = std::fs::read_to_string(types_path).map_err(EsiError::Io)?;
        let type_entries: Vec<TypeEntry> =
            serde_json::from_str(&types_text).map_err(EsiError::Deserialize)?;

        let mut types = HashMap::with_capacity(type_entries.len());
        for entry in type_entries {
            types.insert(entry.type_id, entry.name);
        }

        let security_text = std::fs::read_to_string(security_path).map_err(EsiError::Io)?;
        let agents: AgentCache =
            serde_json::from_str(&security_text).map_err(EsiError::Deserialize)?;

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
                .map(|(id, name, true_sec)| (id, (name, true_sec)))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(low: Option<i32>, high: Option<i32>) -> SecurityFilter {
        SecurityFilter { low, high }
    }

    #[test]
    fn a_single_level_means_exactly_that_level() {
        assert_eq!(SecurityFilter::parse("0.5"), Ok(bounds(Some(5), Some(5))));
        assert_eq!(SecurityFilter::parse("1.0"), Ok(bounds(Some(10), Some(10))));
    }

    #[test]
    fn a_closed_range_includes_both_bounds() {
        assert_eq!(
            SecurityFilter::parse("0.5..0.7"),
            Ok(bounds(Some(5), Some(7)))
        );
        let filter = SecurityFilter::parse("0.5..0.7").unwrap();
        assert!(filter.matches(5), "the lower bound is included");
        assert!(filter.matches(6));
        assert!(filter.matches(7), "the upper bound is included");
        assert!(!filter.matches(4));
        assert!(!filter.matches(8));
    }

    #[test]
    fn an_open_end_leaves_that_side_unbounded() {
        assert_eq!(SecurityFilter::parse("0.5.."), Ok(bounds(Some(5), None)));
        assert_eq!(SecurityFilter::parse("..0.5"), Ok(bounds(None, Some(5))));
        let at_least = SecurityFilter::parse("0.5..").unwrap();
        assert!(at_least.matches(5));
        assert!(at_least.matches(10), "the old +0.5 also matched highsec");
        assert!(!at_least.matches(4));
        let at_most = SecurityFilter::parse("..0.5").unwrap();
        assert!(at_most.matches(0), "the old -0.5 also matched nullsec");
        assert!(at_most.matches(-5));
        assert!(!at_most.matches(6));
    }

    #[test]
    fn the_double_dot_keeps_a_leading_dash_a_sign() {
        assert_eq!(
            SecurityFilter::parse("-0.9..-0.5"),
            Ok(bounds(Some(-9), Some(-5)))
        );
        assert_eq!(
            SecurityFilter::parse("-0.5..-0.5"),
            Ok(bounds(Some(-5), Some(-5)))
        );
        assert_eq!(SecurityFilter::parse("..-0.5"), Ok(bounds(None, Some(-5))));
    }

    #[test]
    fn bounds_are_rounded_to_displayed_tenths() {
        // A single level has always gone through displayed_security, and each bound of a range
        // does the same, so 0.45 sits on the 0.5 band rather than being rejected.
        assert_eq!(
            SecurityFilter::parse("0.45..0.65"),
            Ok(bounds(Some(5), Some(7)))
        );
        assert_eq!(SecurityFilter::parse("0.45"), Ok(bounds(Some(5), Some(5))));
    }

    #[test]
    fn a_reversed_range_names_both_bounds() {
        let error = SecurityFilter::parse("0.7..0.5").unwrap_err();
        assert!(error.contains("0.7"), "{}", error);
        assert!(error.contains("0.5"), "{}", error);
        assert!(error.contains("above"), "{}", error);
    }

    #[test]
    fn a_range_that_bounds_nothing_is_refused() {
        for value in ["..", ".. ", " .."] {
            let error = SecurityFilter::parse(value).unwrap_err();
            assert!(error.contains("0.5..0.7"), "{value:?} gave {error:?}");
        }
    }

    #[test]
    fn the_old_forms_are_refused_and_say_what_to_write() {
        let plus = SecurityFilter::parse("+0.5").unwrap_err();
        assert!(plus.contains("0.5.."), "{plus}");
        let equals = SecurityFilter::parse("=0.5").unwrap_err();
        assert!(equals.contains("'0.5'"), "{equals}");
        // The bare dash is the dangerous one: it used to mean at most, and a plain reading of
        // "-0.5" as a level would silently change which corporations match.
        let minus = SecurityFilter::parse("-0.5").unwrap_err();
        assert!(minus.contains("..0.5"), "{minus}");
        assert!(minus.contains("-0.5..-0.5"), "{minus}");
    }

    #[test]
    fn levels_outside_the_security_range_are_refused() {
        for value in ["1.5", "-1.5..-1.5", "0.5..1.5", "..-1.5", "11..12"] {
            let error = SecurityFilter::parse(value).unwrap_err();
            assert!(
                error.contains("between -1.0 and 1.0"),
                "{value:?} gave {error:?}"
            );
        }
        // A bare negative never reaches that check, because the ambiguity guard answers it first.
        assert!(
            SecurityFilter::parse("-1.5")
                .unwrap_err()
                .contains("ambiguous")
        );
    }

    #[test]
    fn a_bound_that_is_not_a_number_is_refused() {
        for value in ["abc", "0.5..abc", "abc..0.5", "0.5...0.7", "0.5.0.7"] {
            assert!(SecurityFilter::parse(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn the_displayed_tenths_of_a_level_are_unchanged() {
        assert_eq!(displayed_security(0.0), 0);
        assert_eq!(displayed_security(0.5), 5);
        assert_eq!(displayed_security(-0.5), -5);
        assert_eq!(displayed_security(1.0), 10);
    }
}

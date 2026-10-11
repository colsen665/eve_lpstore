//! The EVE Online static data archive: its record shapes, and extracting the cache from it.

use super::agents::AgentRow;
use super::cache::{
    AGENT_CACHE_VERSION, AgentCache, CorporationEntry, RegionEntry, TypeEntry, write_json,
};
use super::security::displayed_security;
use super::{StaticData, StaticDataError};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

const SDE_ZIP_URL: &str =
    "https://developers.eveonline.com/static-data/eve-online-static-data-latest-jsonl.zip";

/// The archive files the cache is built from, by their exact names in the archive.
const SDE_REGIONS: &str = "mapRegions.jsonl";

const SDE_CORPORATIONS: &str = "npcCorporations.jsonl";

const SDE_TYPES: &str = "types.jsonl";

const SDE_STATIONS: &str = "npcStations.jsonl";

const SDE_SYSTEMS: &str = "mapSolarSystems.jsonl";

const SDE_STARGATES: &str = "mapStargates.jsonl";

const SDE_FACTIONS: &str = "factions.jsonl";

const SDE_DIVISIONS: &str = "npcCorporationDivisions.jsonl";

const SDE_AGENT_TYPES: &str = "agentTypes.jsonl";

const SDE_CHARACTERS: &str = "npcCharacters.jsonl";

/// A region, corporation, or type record from the archive, read for its id and English name.
#[derive(Debug, Deserialize)]
struct SdeNamed {
    #[serde(rename = "_key")]
    id: i32,
    name: SdeName,
}

/// The localized name on an archive record, of which only English is read.
#[derive(Debug, Deserialize)]
struct SdeName {
    en: String,
}

/// NPC station record, used to find each station's solar system.
#[derive(Debug, Deserialize)]
struct SdeNpcStation {
    #[serde(rename = "_key")]
    station_id: i32,
    #[serde(rename = "solarSystemID")]
    solar_system_id: i32,
}

/// Solar system record, used for its name, security status, and the region and constellation it
/// belongs to.
#[derive(Debug, Deserialize)]
struct SdeSolarSystem {
    #[serde(rename = "_key")]
    system_id: i32,
    #[serde(rename = "securityStatus")]
    security_status: f64,
    #[serde(rename = "regionID")]
    region_id: i32,
    #[serde(rename = "constellationID")]
    constellation_id: i32,
    name: SdeName,
}

/// Stargate record, used only for the pair of systems it joins. The archive lists one record per
/// direction, so every edge appears twice. The cache keeps both, and `neighbour_map` collapses
/// any repeats.
#[derive(Debug, Deserialize)]
struct SdeStargate {
    #[serde(rename = "solarSystemID")]
    system_id: i32,
    destination: SdeStargateDestination,
}

#[derive(Debug, Deserialize)]
struct SdeStargateDestination {
    #[serde(rename = "solarSystemID")]
    system_id: i32,
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
    name: SdeName,
}

impl StaticData {
    /// Download the archive, extract the four cache files from it, and write them under `data`.
    pub(super) async fn download_and_cache(data: &Path) -> Result<(), StaticDataError> {
        let [regions_path, corporations_path, types_path, security_path] = Self::files(data);
        tokio::fs::create_dir_all(data)
            .await
            .map_err(|source| StaticDataError::Io {
                path: data.to_path_buf(),
                source,
            })?;

        let response = reqwest::get(SDE_ZIP_URL).await?.error_for_status()?;
        let bytes = response.bytes().await?;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))?;

        let regions: Vec<RegionEntry> = records::<SdeNamed>(&mut archive, SDE_REGIONS)?
            .into_iter()
            .map(|record| RegionEntry {
                region_id: record.id,
                name: record.name.en,
            })
            .collect();
        let corporations: Vec<CorporationEntry> =
            records::<SdeNamed>(&mut archive, SDE_CORPORATIONS)?
                .into_iter()
                .map(|record| CorporationEntry {
                    corporation_id: record.id,
                    name: record.name.en,
                })
                .collect();
        let types: Vec<TypeEntry> = records::<SdeNamed>(&mut archive, SDE_TYPES)?
            .into_iter()
            .map(|record| TypeEntry {
                type_id: record.id,
                name: record.name.en,
            })
            .collect();
        let agents = Self::extract_agent_cache(&mut archive)?;

        write_json(&regions_path, &regions).await?;
        write_json(&corporations_path, &corporations).await?;
        write_json(&types_path, &types).await?;
        write_json(&security_path, &agents).await?;
        Ok(())
    }

    /// The agent cache: each corporation's agents with the displayed security of the system they
    /// stand in, and the system, stargate, faction, division, agent type, and station tables the
    /// queries need.
    ///
    /// Agents are NPC characters with an `agent` field. Each agent's location is an NPC
    /// station, whose solar system's security status gives the displayed value.
    fn extract_agent_cache(archive: &mut Archive) -> Result<AgentCache, StaticDataError> {
        let station_systems: HashMap<i32, i32> = records::<SdeNpcStation>(archive, SDE_STATIONS)?
            .into_iter()
            .map(|record| (record.station_id, record.solar_system_id))
            .collect();

        let mut system_names = Vec::new();
        let mut system_security = HashMap::new();
        for record in records::<SdeSolarSystem>(archive, SDE_SYSTEMS)? {
            system_names.push((
                record.system_id,
                record.name.en,
                record.security_status,
                record.region_id,
                record.constellation_id,
            ));
            system_security.insert(record.system_id, record.security_status);
        }

        let gates = records::<SdeStargate>(archive, SDE_STARGATES)?
            .into_iter()
            .map(|record| (record.system_id, record.destination.system_id))
            .collect();

        let corporation_factions = records::<SdeFactionCorporation>(archive, SDE_CORPORATIONS)?
            .into_iter()
            .filter_map(|record| Some((record.corporation_id, record.faction_id?)))
            .collect();

        let factions = records::<SdeFaction>(archive, SDE_FACTIONS)?
            .into_iter()
            .map(|record| (record.faction_id, record.name.en))
            .collect();

        let divisions = records::<SdeDivision>(archive, SDE_DIVISIONS)?
            .into_iter()
            .map(|record| (record.division_id, record.internal_name))
            .collect();

        let agent_types = records::<SdeAgentType>(archive, SDE_AGENT_TYPES)?
            .into_iter()
            .map(|record| (record.agent_type_id, record.name))
            .collect();

        let mut corporations: HashMap<i32, Vec<AgentRow>> = HashMap::new();
        for record in records::<SdeNpcCharacter>(archive, SDE_CHARACTERS)? {
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
            version: AGENT_CACHE_VERSION,
            corporations: corporations.into_iter().collect(),
            divisions,
            agent_types,
            systems: system_names,
            corporation_factions,
            factions,
            stations,
            gates,
        })
    }
}

/// The downloaded archive, read in memory.
type Archive<'a> = zip::ZipArchive<std::io::Cursor<&'a [u8]>>;

/// Every record of one JSONL file in the archive, one per line that is not blank.
fn records<T: DeserializeOwned>(
    archive: &mut Archive,
    file: &'static str,
) -> Result<Vec<T>, StaticDataError> {
    let mut entry = archive
        .by_name(file)
        .map_err(|source| StaticDataError::ArchiveFile { file, source })?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|source| StaticDataError::ArchiveFile {
            file,
            source: source.into(),
        })?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|source| StaticDataError::Record {
                file,
                line: index + 1,
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An archive in memory holding the given files.
    fn archive_with(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, text) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn records_skip_blank_lines_and_name_the_line_that_fails() {
        let bytes = archive_with(&[(
            SDE_REGIONS,
            "{\"_key\": 1, \"name\": {\"en\": \"A\"}}\n\n{\"_key\": \"two\"}\n",
        )]);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        let error = records::<SdeNamed>(&mut archive, SDE_REGIONS).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("mapRegions.jsonl in the static data archive, line 3: "),
            "{error}"
        );
    }

    #[test]
    fn a_file_the_archive_lacks_is_named() {
        let bytes = archive_with(&[(SDE_TYPES, "")]);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        assert!(
            records::<SdeNamed>(&mut archive, SDE_TYPES)
                .unwrap()
                .is_empty()
        );
        let error = records::<SdeNamed>(&mut archive, SDE_REGIONS).unwrap_err();
        assert!(
            matches!(
                error,
                StaticDataError::ArchiveFile {
                    file: SDE_REGIONS,
                    source: zip::result::ZipError::FileNotFound,
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn names_are_read_from_the_english_entry() {
        let bytes = archive_with(&[(
            SDE_REGIONS,
            "{\"_key\": 10000002, \"name\": {\"de\": \"Die Schmiede\", \"en\": \"The Forge\"}}\n",
        )]);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        let regions = records::<SdeNamed>(&mut archive, SDE_REGIONS).unwrap();
        assert_eq!(
            (regions[0].id, regions[0].name.en.as_str()),
            (10000002, "The Forge")
        );
    }
}

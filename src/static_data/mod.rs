//! The EVE Online static data: names, agents, systems, and stargates, downloaded once and kept
//! in the platform data directory.

mod agents;
mod cache;
mod error;
#[cfg(test)]
mod fixtures;
mod routes;
mod sde;
mod security;

pub use agents::AgentFilter;
pub use error::StaticDataError;
pub use routes::DestinationSpread;
pub use security::SecurityFilter;

use agents::AgentRow;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Name of the archive the four files below are extracted from. Nothing writes it to disk today,
/// since `download_and_cache` unzips it in memory, but `--clear-cache` looks for it in case that
/// changes.
pub const SDE_ARCHIVE_NAME: &str = "static-data.zip";

/// What the static data knows about one solar system.
#[derive(Debug, Clone, PartialEq)]
struct SystemInfo {
    name: String,
    /// The unrounded security status, which a route's floor is measured in.
    true_sec: f64,
    region_id: i32,
    constellation_id: i32,
}

/// Names, agents, systems, and stargates from the EVE Online static data, looked up by name or by
/// ID.
///
/// The four files that [`files`](Self::files) names are kept in the platform data directory. They
/// are downloaded when forced, when one is missing, or when the agent file is from an older
/// build.
///
/// The default is an empty cache: every lookup misses and callers fall back to IDs. That is a
/// usable value, and it lets the `--tui` view be render-tested without the data files, which are
/// downloaded on demand.
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
    systems: HashMap<i32, SystemInfo>,
    /// Member systems of each constellation, sorted so iteration is stable across runs.
    constellation_members: HashMap<i32, Vec<i32>>,
    /// Solar systems reachable through one stargate, keyed by the system you start in. Built once
    /// at load so a hop count is a map lookup rather than a scan of the gate list.
    neighbours: HashMap<i32, Vec<i32>>,
    /// Burner route spreads are computed on the full gate graph, which has thousands of systems.
    /// Cache by agent origin so redraws of the Details pane do not repeat that graph search.
    burner_spreads: Mutex<HashMap<i32, Option<DestinationSpread>>>,
    corporation_factions: HashMap<i32, i32>,
    factions: Vec<(i32, String)>,
    stations: Vec<(i64, i32)>,
}

impl StaticData {
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

    /// How a region is written when it has to be named to a user: `The Forge (10000002)`, or the
    /// bare id when the archive does not name it, which is the state before the data is fetched.
    pub fn region_label(&self, region_id: i32) -> String {
        match self.region_name(region_id) {
            Some(name) => format!("{} ({})", name, region_id),
            None => region_id.to_string(),
        }
    }

    /// Look up a corporation ID by name (case-insensitive).
    pub fn corporation_id(&self, name: &str) -> Option<i32> {
        self.corporations.get(&name.to_lowercase()).copied()
    }

    /// The name of a corporation, or `None` when the id is unknown.
    pub fn corporation_name(&self, corporation_id: i32) -> Option<&String> {
        self.corporation_names_by_id.get(&corporation_id)
    }

    /// Get all known corporation IDs.
    pub fn corporation_ids(&self) -> impl Iterator<Item = i32> {
        self.corporations.values().copied()
    }

    /// The name of a solar system, or `None` when the id is unknown.
    pub fn system_name(&self, system_id: i32) -> Option<&String> {
        self.systems.get(&system_id).map(|system| &system.name)
    }

    /// Get all known solar system names.
    pub fn system_names(&self) -> impl Iterator<Item = &String> {
        self.systems.values().map(|system| &system.name)
    }

    /// The region a solar system is in, or `None` when the system is not known.
    pub fn region_of_system(&self, system_id: i32) -> Option<i32> {
        self.systems.get(&system_id).map(|system| system.region_id)
    }

    /// Look up a type name by type ID.
    pub fn type_name(&self, type_id: i32) -> Option<&String> {
        self.types.get(&type_id)
    }

    /// The solar system ID for a system name or ID, with names matched case-insensitively.
    pub fn system_id(&self, value: &str) -> Option<i32> {
        if let Ok(id) = value.parse::<i32>()
            && self.systems.contains_key(&id)
        {
            return Some(id);
        }
        self.systems
            .iter()
            .find(|(_, system)| system.name.eq_ignore_ascii_case(value))
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

    /// The internal name of a division, or `None` when the id is unknown.
    pub fn division_name(&self, division_id: i32) -> Option<&String> {
        self.divisions
            .iter()
            .find(|(id, _)| *id == division_id)
            .map(|(_, name)| name)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_cache_knows_no_regions() {
        assert_eq!(StaticData::default().region_of_system(30000142), None);
    }
}

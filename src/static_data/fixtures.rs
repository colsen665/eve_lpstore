//! Hand-built `StaticData` values shared by the tests of the submodules.

use super::agents::AgentRow;
use super::security::displayed_security;
use super::{StaticData, SystemInfo};
use std::collections::HashMap;

/// One row of a test fixture: id, name, true security, region and constellation.
pub(super) type FixtureRow = (i32, (&'static str, f64, i32, i32));

/// A `StaticData` for tests: the given systems and the given gate pairs, each recorded both
/// ways as the archive does.
pub(super) fn fixture(rows: &[FixtureRow], gates: &[(i32, i32)]) -> StaticData {
    let directed: Vec<(i32, i32)> = gates
        .iter()
        .flat_map(|(a, b)| [(*a, *b), (*b, *a)])
        .collect();
    let systems: HashMap<i32, SystemInfo> = rows
        .iter()
        .map(|(id, (name, true_sec, region_id, constellation_id))| {
            let system = SystemInfo {
                name: name.to_string(),
                true_sec: *true_sec,
                region_id: *region_id,
                constellation_id: *constellation_id,
            };
            (*id, system)
        })
        .collect();
    let pairs: Vec<(i32, i32)> = systems
        .iter()
        .map(|(id, system)| (*id, system.constellation_id))
        .collect();
    StaticData {
        constellation_members: StaticData::constellation_index(&pairs),
        neighbours: StaticData::neighbour_map(&directed),
        systems,
        ..Default::default()
    }
}

/// The worked example: four systems in one constellation whose true security rounds to three at
/// 0.5 and one at 0.6.
pub(super) const FOUR_SYSTEMS: [FixtureRow; 4] = [
    (1, ("A", 0.49, 7, 100)),
    (2, ("B", 0.50, 7, 100)),
    (3, ("C", 0.51, 7, 100)),
    (4, ("D", 0.60, 7, 100)),
];

/// A `StaticData` with agent rows, for the listing questions the destination pane asks. Each
/// agent is (corporation, system, level, division), and the division ids are the archive's.
pub(super) fn fixture_with_agents(
    rows: &[FixtureRow],
    agents: &[(i32, i32, i32, i32)],
) -> StaticData {
    let agents: Vec<(i32, i32, i32, i32, i32)> = agents
        .iter()
        .map(|(corp, system, level, division)| (*corp, *system, *level, *division, 2))
        .collect();
    fixture_with_agents_and_gates(rows, &[], &agents)
}

/// As above, but with a gate fixture too, for route filters and burner destination pools.
pub(super) fn fixture_with_agents_and_gates(
    rows: &[FixtureRow],
    gates: &[(i32, i32)],
    agents: &[(i32, i32, i32, i32, i32)],
) -> StaticData {
    let mut data = fixture(rows, gates);
    let mut by_corporation: HashMap<i32, Vec<AgentRow>> = HashMap::new();
    for (corporation, system, level, division, agent_type) in agents {
        let true_sec = data
            .systems
            .get(system)
            .map(|system| system.true_sec)
            .unwrap_or(0.4);
        by_corporation
            .entry(*corporation)
            .or_default()
            .push(AgentRow {
                level: *level,
                tenths: displayed_security(true_sec),
                division_id: *division,
                agent_type_id: *agent_type,
                system_id: *system,
            });
    }
    data.corporation_agents = by_corporation;
    data.divisions = vec![
        (22, "Distribution".to_string()),
        (23, "Mining".to_string()),
        (24, "Security".to_string()),
    ];
    data.agent_types = vec![
        (1, "ResearchAgent".to_string()),
        (2, "BasicAgent".to_string()),
    ];
    data
}

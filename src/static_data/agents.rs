//! Agents: the rows the archive lists for each corporation, and the filters over them.

use super::{SecurityFilter, StaticData};
use serde::{Deserialize, Serialize};

/// One agent's filterable details: level, displayed security in tenths, division, and type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct AgentRow {
    pub(super) level: i32,
    pub(super) tenths: i32,
    pub(super) division_id: i32,
    pub(super) agent_type_id: i32,
    pub(super) system_id: i32,
}

/// Conditions that one agent must all satisfy. A missing condition matches any agent.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentFilter {
    pub security: Option<SecurityFilter>,
    pub level: Option<i32>,
    pub division_id: Option<i32>,
    pub agent_type_id: Option<i32>,
    /// Keeps agents standing in a system inside this region. The row carries only its system, so
    /// the region is asked of the static data, which is why `matches` takes it.
    pub region: Option<i32>,
    /// Keeps agents standing in this exact system.
    pub system: Option<i32>,
}

impl AgentFilter {
    /// Whether one agent row satisfies every condition set. They all have to hold for the **same**
    /// agent, which is what the help text promises: a corporation with a level 1 agent in the region
    /// and a level 5 agent elsewhere does not pass a region filter combined with a level.
    ///
    fn matches(&self, row: &AgentRow, sdata: &StaticData) -> bool {
        self.security
            .is_none_or(|filter| filter.matches(row.tenths))
            && self.level.is_none_or(|level| row.level == level)
            && self
                .division_id
                .is_none_or(|division| row.division_id == division)
            && self
                .agent_type_id
                .is_none_or(|agent_type| row.agent_type_id == agent_type)
            && self.system.is_none_or(|system| row.system_id == system)
            && self
                .region
                .is_none_or(|region| sdata.region_of_system(row.system_id) == Some(region))
    }
}

impl StaticData {
    /// Agent systems for which the same corporation has a level 4 Basic Security agent that passes
    /// the active filters. Burner capability is a property of that matching agent row, not merely
    /// of any level-4 agent in the system: when filters select a different agent, its row does not
    /// display a burner distribution.
    pub fn burner_agent_systems(
        &self,
        corporation_id: i32,
        filter: &AgentFilter,
    ) -> Vec<(i32, String, f64)> {
        let Some(security) = self.security_division_id() else {
            return Vec::new();
        };
        let Some(basic) = self.agent_type_id("Basic") else {
            return Vec::new();
        };
        if filter.level.is_some_and(|level| level != 4)
            || filter
                .division_id
                .is_some_and(|division| division != security)
            || filter
                .agent_type_id
                .is_some_and(|agent_type| agent_type != basic)
        {
            return Vec::new();
        }
        let scoped = AgentFilter {
            level: Some(4),
            division_id: Some(security),
            agent_type_id: Some(basic),
            ..*filter
        };
        self.agent_systems(corporation_id, &scoped)
    }

    /// Whether one agent of the corporation satisfies every condition in the filter.
    pub fn corporation_passes(&self, corporation_id: i32, filter: &AgentFilter) -> bool {
        self.corporation_agents
            .get(&corporation_id)
            .is_some_and(|rows| rows.iter().any(|row| filter.matches(row, self)))
    }

    /// Names of the systems where the corporation's agents matching the filter are, in data order.
    pub fn corporation_systems(
        &self,
        corporation_id: i32,
        filter: &AgentFilter,
    ) -> Vec<(String, f64)> {
        self.agent_systems(corporation_id, filter)
            .into_iter()
            .map(|(_, name, true_sec)| (name, true_sec))
            .collect()
    }

    /// The id of the agent division whose missions this project models, named by its internal name
    /// rather than a number: the archive calls it `Security` and numbers it 24 today, but the id is
    /// an SDE row and the name is what the filter form already shows the user.
    pub fn security_division_id(&self) -> Option<i32> {
        self.division_id("Security")
    }

    /// The agent systems whose missions the destination model describes: the Security division only,
    /// since courier and mining missions are outside it. The rest of `filter` still applies, and a
    /// filter that names some other division answers empty rather than quietly widening the scope to
    /// match what it lists.
    ///
    /// Note what this cannot do: burner and anomic missions come from the same level 4 Security
    /// agents as regular ones, so narrowing by division does not exclude them.
    pub fn security_agent_systems(
        &self,
        corporation_id: i32,
        filter: &AgentFilter,
    ) -> Vec<(i32, String, f64)> {
        let Some(security) = self.security_division_id() else {
            return Vec::new();
        };
        if filter
            .division_id
            .is_some_and(|division| division != security)
        {
            return Vec::new();
        }
        let mut scoped = *filter;
        scoped.division_id = Some(security);
        self.agent_systems(corporation_id, &scoped)
    }

    /// The systems where the corporation's agents matching the filter are, with their ids, which
    /// is what a destination table needs: the name and security alone cannot be looked up in the
    /// constellation or gate maps. Deduplicated by name, so a corporation with several agents in
    /// one system lists it once.
    fn agent_systems(&self, corporation_id: i32, filter: &AgentFilter) -> Vec<(i32, String, f64)> {
        let mut found: Vec<(i32, String, f64)> = Vec::new();
        for row in self
            .corporation_agents
            .get(&corporation_id)
            .into_iter()
            .flatten()
        {
            if !filter.matches(row, self) {
                continue;
            }
            if let Some(system) = self.systems.get(&row.system_id)
                && !found
                    .iter()
                    .any(|(_, existing, _)| *existing == system.name)
            {
                found.push((row.system_id, system.name.clone(), system.true_sec));
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::static_data::fixtures::*;
    use std::collections::HashMap;

    fn burner_agent_fixture() -> StaticData {
        let rows: [FixtureRow; 6] = [
            (1, ("Origin", 0.9, 7, 100)),
            (2, ("Home", 0.8, 7, 100)),
            (3, ("Neighbour", 0.7, 8, 200)),
            (4, ("HubLocked", 0.6, 8, 200)),
            (5, ("DetourTarget", 0.5, 8, 200)),
            (6, ("TransitOnly", 0.95, 9, 300)),
        ];
        let agents = [
            (77, 1, 4, 24, 2), // Level 4, Basic, Security: can offer burners.
            (77, 1, 1, 24, 2), // Same system, but not a burner-capable agent.
            (77, 2, 1, 24, 2),
            (77, 3, 4, 22, 2), // Distribution is out of scope.
            (77, 5, 4, 24, 1), // Research is out of scope.
            (77, 6, 4, 24, 2), // Another qualifying system, used for active-filter tests.
        ];
        fixture_with_agents_and_gates(&rows, &[(1, 2), (1, 3), (3, 6), (6, 5)], &agents)
    }

    #[test]
    fn only_matching_level_four_basic_security_agents_show_burner_support() {
        let data = burner_agent_fixture();
        assert_eq!(data.security_division_id(), Some(24));
        assert_eq!(data.agent_type_id("Basic"), Some(2));
        assert_eq!(data.agent_systems(77, &AgentFilter::default()).len(), 5);
        let systems = data.burner_agent_systems(77, &AgentFilter::default());
        assert_eq!(
            systems.iter().map(|row| row.0).collect::<Vec<_>>(),
            vec![1, 6],
            "only Level 4 Basic Security agents qualify"
        );
        let level_one = AgentFilter {
            level: Some(1),
            ..Default::default()
        };
        assert!(data.burner_agent_systems(77, &level_one).is_empty());
        let system_six = AgentFilter {
            system: Some(6),
            ..Default::default()
        };
        assert_eq!(
            data.burner_agent_systems(77, &system_six)
                .iter()
                .map(|row| row.0)
                .collect::<Vec<_>>(),
            vec![6],
            "active system filters also apply to the qualifying agent row"
        );
        let research = AgentFilter {
            agent_type_id: Some(1),
            ..Default::default()
        };
        assert!(data.burner_agent_systems(77, &research).is_empty());
        let distribution = AgentFilter {
            division_id: Some(22),
            ..Default::default()
        };
        assert!(data.burner_agent_systems(77, &distribution).is_empty());
    }

    /// The four fixture systems with one Security agent in each at level 1 to 4.
    fn one_agent_per_system() -> StaticData {
        fixture_with_agents(
            &FOUR_SYSTEMS,
            &[
                (7, 1, 1, 24),
                (7, 2, 2, 24),
                (7, 3, 3, 24),
                (7, 4, 4, 24),
                (7, 2, 2, 22),
                (7, 3, 1, 23),
            ],
        )
    }

    /// One corporation with agents on both sides of a region border, so a region filter has
    /// something to keep and something to drop. Systems 1 and 2 stand in region 7, systems 3 and 4
    /// in region 8. Corporation 7 has an agent in each: level 1 in Alpha, level 2 in Beta, level 4
    /// in Gamma, level 1 in Delta. Corporation 8 has none, so `corporation_passes` has a no answer.
    fn two_regions() -> StaticData {
        let rows: [FixtureRow; 4] = [
            (1, ("Alpha", 0.49, 7, 100)),
            (2, ("Beta", 0.50, 7, 100)),
            (3, ("Gamma", 0.51, 8, 101)),
            (4, ("Delta", 0.60, 8, 101)),
        ];
        fixture_with_agents(
            &rows,
            &[(7, 1, 1, 24), (7, 2, 2, 24), (7, 3, 4, 24), (7, 4, 1, 24)],
        )
    }

    /// The ids a listing returned, in the order the listing gives them.
    fn ids(listed: &[(i32, String, f64)]) -> Vec<i32> {
        listed.iter().map(|(id, _, _)| *id).collect()
    }

    #[test]
    fn a_region_filter_keeps_only_the_agents_standing_there() {
        let data = two_regions();
        let in_eight = AgentFilter {
            region: Some(8),
            ..Default::default()
        };
        assert_eq!(
            ids(&data.security_agent_systems(7, &in_eight)),
            vec![3, 4],
            "the two agents across the border are kept"
        );
        let in_seven = AgentFilter {
            region: Some(7),
            ..Default::default()
        };
        assert_eq!(ids(&data.security_agent_systems(7, &in_seven)), vec![1, 2]);
        let everywhere = AgentFilter::default();
        assert_eq!(
            ids(&data.security_agent_systems(7, &everywhere)),
            vec![1, 2, 3, 4],
            "no region filter keeps all four, so the filter above removed something real"
        );
        assert!(
            !data.corporation_passes(8, &in_eight),
            "a corporation with no agents at all passes nothing, not even an empty filter"
        );
        assert!(data.corporation_passes(7, &in_eight), "and one does pass");
    }

    #[test]
    fn a_system_filter_keeps_that_system_alone() {
        let data = two_regions();
        let only_two = AgentFilter {
            system: Some(2),
            ..Default::default()
        };
        assert_eq!(ids(&data.security_agent_systems(7, &only_two)), vec![2]);
        let unknown = AgentFilter {
            system: Some(99),
            ..Default::default()
        };
        assert!(
            data.security_agent_systems(7, &unknown).is_empty(),
            "a system nothing stands in keeps nothing"
        );
    }

    #[test]
    fn region_and_level_must_hold_for_the_same_agent() {
        // Region 8 has a level 4 agent in Gamma and a level 1 agent in Delta; the level 2 agent is
        // in Beta, which is in region 7. So region 8 together with level 2 matches no single agent,
        // even though each half matches somewhere. The help text promises this reading: "All filters
        // given must match the same agent."
        let data = two_regions();
        let pair = AgentFilter {
            region: Some(8),
            level: Some(2),
            ..Default::default()
        };
        assert!(
            !data.corporation_passes(7, &pair),
            "neither half alone is the answer; they have to be one agent"
        );
        assert_eq!(
            ids(&data.security_agent_systems(7, &pair)),
            Vec::<i32>::new()
        );
        let matches = AgentFilter {
            region: Some(8),
            level: Some(4),
            ..Default::default()
        };
        assert!(
            data.corporation_passes(7, &matches),
            "Gamma is level 4 in region 8"
        );
        assert_eq!(ids(&data.security_agent_systems(7, &matches)), vec![3]);
    }

    #[test]
    fn a_system_named_outside_the_region_named_keeps_nothing() {
        let data = two_regions();
        assert_eq!(data.region_of_system(3), Some(8));
        let agrees = AgentFilter {
            region: Some(8),
            system: Some(3),
            ..Default::default()
        };
        assert_eq!(ids(&data.security_agent_systems(7, &agrees)), vec![3]);
        let disagrees = AgentFilter {
            region: Some(7),
            system: Some(3),
            ..Default::default()
        };
        assert!(
            data.security_agent_systems(7, &disagrees).is_empty(),
            "the pair the view refuses to run anyway keeps nothing on its own"
        );
    }

    /// A `StaticData` that names its two regions, so a filter written as a name can be resolved and
    /// tested against the same ids the flag form produces.
    fn named_places() -> StaticData {
        let mut data = two_regions();
        // Both maps the load fills: names by id for the header, ids by lowercase name for a filter
        // typed as a name.
        data.region_names_by_id =
            HashMap::from([(7, "The Forge".to_string()), (8, "Black Rise".to_string())]);
        data.regions = HashMap::from([("the forge".to_string(), 7), ("black rise".to_string(), 8)]);
        data
    }

    #[test]
    fn a_region_filter_resolved_from_a_name_is_the_same_filter_as_the_id() {
        let data = named_places();
        let by_name = data.region_id("black rise");
        assert_eq!(by_name, Some(8), "region names ignore case");
        let listed: Vec<i32> = data
            .security_agent_systems(
                7,
                &AgentFilter {
                    region: by_name,
                    ..Default::default()
                },
            )
            .iter()
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(
            listed,
            vec![3, 4],
            "the name selects the region it resolves to"
        );
        assert_eq!(
            data.security_agent_systems(
                7,
                &AgentFilter {
                    region: Some(8),
                    ..Default::default()
                },
            )
            .len(),
            listed.len(),
            "and the id form agrees with it"
        );
    }

    #[test]
    fn the_security_division_is_found_by_its_name() {
        let data = one_agent_per_system();
        assert_eq!(data.security_division_id(), Some(24));
        assert_eq!(
            data.division_name(22).map(String::as_str),
            Some("Distribution")
        );
        assert_eq!(data.division_name(99), None);
    }

    #[test]
    fn only_security_agents_are_listed_for_the_model() {
        let data = one_agent_per_system();
        let listed = data.security_agent_systems(7, &AgentFilter::default());
        let ids: Vec<i32> = listed.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            ids,
            vec![1, 2, 3, 4],
            "the Distribution and Mining rows are left out"
        );
        // The same listing without the scope keeps them, which is what the pane used to show.
        let all = data.agent_systems(7, &AgentFilter::default());
        assert!(all.len() >= listed.len(), "the scope can only narrow");
    }

    #[test]
    fn a_filter_for_another_division_leaves_nothing_to_model() {
        let data = one_agent_per_system();
        let courier = AgentFilter {
            division_id: Some(22),
            ..Default::default()
        };
        assert!(
            data.security_agent_systems(7, &courier).is_empty(),
            "courier agents are outside the model, not folded into it"
        );
        // Other conditions still apply inside the scope.
        let level_4 = AgentFilter {
            level: Some(4),
            ..Default::default()
        };
        let listed = data.security_agent_systems(7, &level_4);
        assert_eq!(
            listed.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
            vec![4],
            "level 4 narrows the security rows only"
        );
    }

    #[test]
    fn an_unknown_division_table_lists_nothing_rather_than_everything() {
        // A cache without the division names cannot say which agents are in scope, and guessing by
        // id would show missions the model does not describe.
        let mut data = one_agent_per_system();
        data.divisions = Vec::new();
        assert_eq!(data.security_division_id(), None);
        assert!(
            data.security_agent_systems(7, &AgentFilter::default())
                .is_empty()
        );
    }
}

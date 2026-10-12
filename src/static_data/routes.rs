//! The stargate graph and the mission destination model built on it.

use super::StaticData;
use super::security::displayed_security;
use std::collections::{HashMap, HashSet, VecDeque};

/// Standard NPC trade hubs excluded from burner destination candidates and route transit, as
/// specified for the burner model.
const BURNER_BLACKLIST: [i32; 5] = [
    30000142, // Jita
    30002187, // Amarr
    30002659, // Dodixie
    30002510, // Rens
    30002053, // Hek
];

/// Where a mission from an agent in one system can land, under the model this project assumes:
/// every system in the agent's constellation is an equally likely destination.
///
/// Both groupings are marginals of that one uniform distribution. The band grouping counts systems
/// per displayed security band and so does not depend on the graph at all; the hop grouping counts
/// systems per gate distance and so changes with the shape of the constellation and with which
/// member holds the agent.
#[derive(Debug, Clone, PartialEq)]
pub struct DestinationSpread {
    /// The constellation the counting stays inside.
    pub constellation_id: i32,
    /// Its member systems reachable from the agent system, which is the denominator for every
    /// probability here.
    pub systems: usize,
    /// Bands by the security flown through on the route, in tenths, with their probability. The
    /// lowest security on the route decides the band, and the route starts at the agent system, so
    /// no destination can sit above the agent system's own band.
    pub bands: Vec<(i32, f64)>,
    /// The same destinations grouped by the security of the system itself, kept beside `bands` so a
    /// collapsed route grouping can still be read: `bands` says how far you must fly down, `own_bands`
    /// says what is there.
    pub own_bands: Vec<(i32, f64)>,
    /// Gates on the route flown, with the probability of landing that far out, in hop order.
    pub hops: Vec<(usize, f64)>,
    /// Every destination as (system, hops, route band, own band), in id order.
    pub destinations: Vec<(i32, usize, i32, i32)>,
}

impl DestinationSpread {
    /// The share of destinations in one band or at one hop distance, as `count / systems`.
    fn share(count: usize, systems: usize) -> f64 {
        count as f64 / systems as f64
    }
}

/// A route's standing: (security floor, gates flown), judged highest floor first, then fewest
/// gates. Two routes that tie on both are equal, and the first one found stands; the search walks
/// neighbours in id order, so which one that is does not depend on hash order.
type Route = (f64, usize);

/// Ranking for two candidate routes to the same destination.
fn better_route(candidate: &Route, current: &Route) -> bool {
    let (cf, ch) = *candidate;
    let (bf, bh) = *current;
    if cf > bf + f64::EPSILON {
        return true;
    }
    if cf < bf - f64::EPSILON {
        return false;
    }
    ch < bh
}

/// The two destination marginals over a uniformly weighted system set. Both regular and burner
/// distributions use the same route-band, own-band and hop summaries; only their candidate pools
/// and route graphs differ.
fn destination_spread(
    constellation_id: i32,
    mut destinations: Vec<(i32, usize, i32, i32)>,
) -> DestinationSpread {
    destinations.sort_unstable_by_key(|(system, _, _, _)| *system);
    let systems = destinations.len();
    let mut bands: std::collections::BTreeMap<i32, usize> = Default::default();
    let mut own_bands: std::collections::BTreeMap<i32, usize> = Default::default();
    let mut hop_counts: std::collections::BTreeMap<usize, usize> = Default::default();
    for (_, hops, route_band, own_band) in &destinations {
        *bands.entry(*route_band).or_default() += 1;
        *own_bands.entry(*own_band).or_default() += 1;
        *hop_counts.entry(*hops).or_default() += 1;
    }
    let tally = |counts: std::collections::BTreeMap<i32, usize>| {
        counts
            .into_iter()
            .map(|(band, count)| (band, DestinationSpread::share(count, systems)))
            .collect::<Vec<_>>()
    };

    DestinationSpread {
        constellation_id,
        systems,
        bands: tally(bands),
        own_bands: tally(own_bands),
        hops: hop_counts
            .into_iter()
            .map(|(distance, count)| (distance, DestinationSpread::share(count, systems)))
            .collect(),
        destinations,
    }
}

impl StaticData {
    /// One entry per stargate record, both directions, so hop counting can walk from either side of
    /// a connection. Duplicate entries for a pair are collapsed, and each list is sorted so the same
    /// archive always produces the same map.
    pub(super) fn neighbour_map(gates: &[(i32, i32)]) -> HashMap<i32, Vec<i32>> {
        let mut seen: HashMap<i32, std::collections::HashSet<i32>> = HashMap::new();
        for (from, to) in gates {
            seen.entry(*from).or_default().insert(*to);
        }
        seen.into_iter()
            .map(|(system, list)| {
                let mut list: Vec<i32> = list.into_iter().collect();
                list.sort_unstable();
                (system, list)
            })
            .collect()
    }

    /// The systems one stargate away from `system_id`, in id order. Empty for a system the archive
    /// lists no gates for, which is also the answer for an unknown id, so a caller that needs to
    /// tell those apart should check the system exists first.
    fn gate_neighbours(&self, system_id: i32) -> &[i32] {
        self.neighbours.get(&system_id).map_or(&[], Vec::as_slice)
    }

    /// Each constellation's member systems, from the (system, constellation) pairs the archive
    /// lists. Sorted because the destination lists and band groups are shown to the user and must
    /// not depend on hash order.
    pub(super) fn constellation_index(pairs: &[(i32, i32)]) -> HashMap<i32, Vec<i32>> {
        let mut seen: HashMap<i32, std::collections::HashSet<i32>> = HashMap::new();
        for (system, constellation) in pairs {
            seen.entry(*constellation).or_default().insert(*system);
        }
        seen.into_iter()
            .map(|(constellation, list)| {
                let mut list: Vec<i32> = list.into_iter().collect();
                list.sort_unstable();
                (constellation, list)
            })
            .collect()
    }

    /// The systems in a constellation, in id order. Empty for an unknown constellation.
    fn constellation_members(&self, constellation_id: i32) -> &[i32] {
        self.constellation_members
            .get(&constellation_id)
            .map_or(&[], Vec::as_slice)
    }

    /// The constellation a solar system is in, or `None` when the system is not known.
    fn constellation_of_system(&self, system_id: i32) -> Option<i32> {
        self.systems
            .get(&system_id)
            .map(|system| system.constellation_id)
    }

    /// Where a mission from an agent in `agent_system` can land, or `None` when that system is not
    /// known.
    ///
    /// Destinations are the other systems in the constellation, which is the model this project
    /// assumes rather than a measurement. For each one the route chosen is the one that keeps the
    /// highest possible security floor, staying inside the constellation, because that is what a
    /// pilot flying there would pick; among those, the fewest gates. The floor includes the agent
    /// system itself, so a route can never be safer than where it starts, and every band here is at
    /// or below the agent system's own.
    ///
    /// The floors are exact. The gate counts are not always the fewest: the search keeps one route
    /// per system, so when a safer but longer route replaces a system's route before that system
    /// is expanded, a destination behind it can report more gates than its shortest route at the
    /// same floor. This is recorded in TODO.md and left as it is while the route model is being
    /// compared with the game.
    ///
    /// A member with no path inside the constellation is left out of the counts rather than guessed
    /// at, so `systems` is what was counted and the probabilities sum to one.
    pub fn mission_spread(&self, agent_system: i32) -> Option<DestinationSpread> {
        let constellation_id = self.constellation_of_system(agent_system)?;
        let members = self.constellation_members(constellation_id).to_vec();
        if members.is_empty() {
            return None;
        }
        let inside: std::collections::HashSet<i32> = members.iter().copied().collect();
        self.mission_security(agent_system)?;

        let score = self.best_routes(agent_system, Some(&inside), &HashSet::new())?;

        let mut destinations = Vec::new();
        for system in &members {
            let Some((floor, hops)) = score.get(system) else {
                continue;
            };
            let route_band = displayed_security(*floor);
            let own_band = self.security_band(*system)?;
            destinations.push((*system, *hops, route_band, own_band));
        }
        Some(destination_spread(constellation_id, destinations))
    }

    /// Burner destinations are a separate uniform distribution over systems in the agent's
    /// constellation and any constellation with a direct stargate connection to it. The five
    /// standard NPC trade hubs are excluded as destinations and route transit; route reachability
    /// is checked on the full gate graph with those systems removed, so a detour through another
    /// constellation is allowed but does not make that constellation part of the destination pool.
    /// This is the chosen model rule, not a measured distribution of offers or site spawns.
    ///
    /// `None` means the origin system is unknown. A known origin with no eligible reachable system
    /// returns a spread with zero destinations; callers can explain that exclusion rather than
    /// inventing a 100% probability for the origin.
    pub fn burner_mission_spread(&self, agent_system: i32) -> Option<DestinationSpread> {
        let constellation_id = self.constellation_of_system(agent_system)?;
        if let Ok(cache) = self.burner_spreads.lock()
            && let Some(cached) = cache.get(&agent_system)
        {
            return cached.clone();
        }
        let spread = self.compute_burner_mission_spread(agent_system, constellation_id);
        if let Ok(mut cache) = self.burner_spreads.lock() {
            cache.insert(agent_system, spread.clone());
        }
        spread
    }

    fn compute_burner_mission_spread(
        &self,
        agent_system: i32,
        constellation_id: i32,
    ) -> Option<DestinationSpread> {
        let mut destination_constellations = HashSet::from([constellation_id]);
        for member in self.constellation_members(constellation_id) {
            for neighbour in self.gate_neighbours(*member) {
                if let Some(other) = self.constellation_of_system(*neighbour) {
                    destination_constellations.insert(other);
                }
            }
        }
        let blocked: HashSet<i32> = BURNER_BLACKLIST.into_iter().collect();
        let mut candidates: Vec<i32> = destination_constellations
            .into_iter()
            .flat_map(|constellation| self.constellation_members(constellation).iter().copied())
            .filter(|system| !blocked.contains(system))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();

        // The origin remains a valid starting point even if a data set places it in a blacklisted
        // hub. Blacklisted systems are not entered again as transit, and they are never destinations.
        let route_blocked: HashSet<i32> = BURNER_BLACKLIST
            .into_iter()
            .filter(|system| *system != agent_system)
            .collect();
        let routes = self.best_routes(agent_system, None, &route_blocked)?;
        let mut destinations = Vec::new();
        for system in candidates {
            let Some((floor, hops)) = routes.get(&system) else {
                continue;
            };
            destinations.push((
                system,
                *hops,
                displayed_security(*floor),
                self.security_band(system)?,
            ));
        }
        Some(destination_spread(constellation_id, destinations))
    }

    /// The safest routes from an origin. `inside` restricts traversal when present; `blocked`
    /// systems cannot be entered, except that a blocked origin is allowed as the starting point.
    /// The queue-based widest-path relaxation is deterministic because neighbours are sorted and a
    /// route only replaces one it beats.
    fn best_routes(
        &self,
        origin: i32,
        inside: Option<&HashSet<i32>>,
        blocked: &HashSet<i32>,
    ) -> Option<HashMap<i32, Route>> {
        let start = self.mission_security(origin)?;
        let mut score: HashMap<i32, Route> = HashMap::from([(origin, (start, 0))]);
        let mut queue = VecDeque::from([origin]);
        while let Some(system) = queue.pop_front() {
            let (floor, hops) = *score.get(&system)?;
            for next in self.gate_neighbours(system) {
                if blocked.contains(next) || inside.is_some_and(|set| !set.contains(next)) {
                    continue;
                }
                let Some(security) = self.mission_security(*next) else {
                    continue;
                };
                let candidate = (floor.min(security), hops + 1);
                let improve = score
                    .get(next)
                    .is_none_or(|current| better_route(&candidate, current));
                if improve {
                    score.insert(*next, candidate);
                    queue.push_back(*next);
                }
            }
        }
        Some(score)
    }

    /// The true security of a system, which is what a route's floor is measured in before it is
    /// rounded to a band.
    fn mission_security(&self, system_id: i32) -> Option<f64> {
        self.systems.get(&system_id).map(|system| system.true_sec)
    }

    /// The displayed security band of a system in tenths, the same value `SecurityFilter` compares
    /// on, so a band shown here is a band a user can write in `--security`.
    fn security_band(&self, system_id: i32) -> Option<i32> {
        self.systems
            .get(&system_id)
            .map(|system| displayed_security(system.true_sec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::static_data::fixtures::*;

    #[test]
    fn a_system_reports_its_region_and_constellation() {
        let data = fixture(&[(30000142, ("Jita", 0.9, 10000002, 20000020))], &[]);
        assert_eq!(data.region_of_system(30000142), Some(10000002));
        assert_eq!(data.constellation_of_system(30000142), Some(20000020));
        // Unknown systems answer nothing rather than guessing, so a caller cannot read a missing
        // mapping as a mismatch.
        assert_eq!(data.region_of_system(30000001), None);
        assert_eq!(data.constellation_of_system(30000001), None);
    }

    /// Jita's constellation, Kimotoro, measured from ESI on 2026-10-10: the seven member systems
    /// and every stargate that stays inside the constellation, one entry per direction as the SDE
    /// lists them. Ids are 30000139 Urlen, 30000140 Maurasi, 30000141 Kisogo, 30000142 Jita,
    /// 30000143 Niyabainen, 30000144 Perimeter, 30000145 New Caldari.
    const KIMOTORO: &[(i32, i32)] = &[
        (30000139, 30000141),
        (30000139, 30000144),
        (30000140, 30000142),
        (30000140, 30000144),
        (30000141, 30000139),
        (30000142, 30000140),
        (30000142, 30000143),
        (30000142, 30000144),
        (30000142, 30000145),
        (30000143, 30000142),
        (30000143, 30000144),
        (30000143, 30000145),
        (30000144, 30000139),
        (30000144, 30000140),
        (30000144, 30000142),
        (30000144, 30000143),
        (30000145, 30000142),
        (30000145, 30000143),
    ];

    /// A hand-built static data with one constellation of four systems at the true security values
    /// from the worked example, with the gates as the caller describes them.
    fn four_system_constellation(edges: &[(i32, i32)]) -> StaticData {
        fixture(&FOUR_SYSTEMS, edges)
    }

    /// Sazdih, a real lowsec constellation measured from ESI on 2026-10-10: six systems, five
    /// rounding to 0.4 and one to 0.5. Three of them host agents in this tool's own data, which is
    /// the case the feature exists for, and the agent here is Bayuka. Security is kept to four
    /// decimals, which is finer than the tenths the bands round to, so the values stay readable
    /// without moving any system into a different band.
    const SAZDIH: [FixtureRow; 6] = [
        (30000038, ("Dooz", 0.4237, 10000003, 20000006)),
        (30000039, ("Bayuka", 0.4007, 10000003, 20000006)),
        (30000040, ("Uzistoon", 0.3906, 10000003, 20000006)),
        (30000041, ("Bairshir", 0.3641, 10000003, 20000006)),
        (30000042, ("Moh", 0.4728, 10000003, 20000006)),
        (30000043, ("Sari", 0.3793, 10000003, 20000006)),
    ];

    const SAZDIH_GATES: [(i32, i32); 7] = [
        (30000038, 30000039),
        (30000038, 30000040),
        (30000038, 30000043),
        (30000039, 30000040),
        (30000039, 30000041),
        (30000040, 30000043),
        (30000041, 30000042),
    ];

    #[test]
    fn a_real_lowsec_constellation_splits_into_two_bands() {
        let data = fixture(&SAZDIH, &SAZDIH_GATES);
        let spread = data.mission_spread(30000039).expect("Bayuka");
        assert_eq!(spread.constellation_id, 20000006);
        assert_eq!(spread.systems, 6);
        assert_eq!(
            spread.bands,
            vec![(4, 1.0)],
            "Bayuka is the lowest on every route out"
        );
        assert_eq!(
            spread.own_bands,
            vec![(4, 5.0 / 6.0), (5, 1.0 / 6.0)],
            "five systems round to 0.4, Moh alone to 0.5"
        );
        assert_eq!(
            spread.hops,
            vec![(0, 1.0 / 6.0), (1, 3.0 / 6.0), (2, 2.0 / 6.0)],
            "from Bayuka: one system each side, none three hops out"
        );
        assert_eq!(
            spread
                .destinations
                .iter()
                .map(|(_, _, _, own)| *own)
                .sum::<i32>(),
            4 * 5 + 5,
            "own bands are in tenths: five at 0.4 and Moh at 0.5"
        );
    }

    /// Burner graph fixture. The eligible pool is systems 1-5 plus Jita: systems 1 and 2 share the
    /// agent constellation, 3-5 are in its directly neighbouring constellation, and Jita is in a
    /// second neighbouring constellation. System 4 can only be reached through Jita, which is
    /// blacklisted. System 5 can be reached only by leaving the destination pool through transit
    /// system 6, then returning to its constellation; that route is permitted by the chosen rule.
    fn burner_graph() -> StaticData {
        let rows: [FixtureRow; 7] = [
            (1, ("Origin", 0.9, 7, 100)),
            (2, ("Home", 0.8, 7, 100)),
            (3, ("Neighbour", 0.7, 8, 200)),
            (4, ("HubLocked", 0.6, 8, 200)),
            (5, ("DetourTarget", 0.5, 8, 200)),
            (6, ("TransitOnly", 0.95, 9, 300)),
            (30000142, ("Jita", 0.99, 10, 400)),
        ];
        let gates = [(1, 2), (1, 3), (1, 30000142), (30000142, 4), (3, 6), (6, 5)];
        fixture_with_agents_and_gates(&rows, &gates, &[])
    }

    #[test]
    fn burner_pool_uses_current_and_directly_neighbouring_constellations() {
        assert_eq!(
            BURNER_BLACKLIST,
            [30000142, 30002187, 30002659, 30002510, 30002053],
            "Jita, Amarr, Dodixie, Rens, Hek"
        );
        let data = burner_graph();
        let spread = data.burner_mission_spread(1).unwrap();
        assert_eq!(
            spread.constellation_id, 100,
            "the agent's home constellation"
        );
        assert_eq!(
            spread
                .destinations
                .iter()
                .map(|row| row.0)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 5],
            "home and direct-neighbour systems only; the blacklisted hub, its locked system, and the transit-only system are absent"
        );
        assert_eq!(spread.systems, 4);
        assert_eq!(
            spread
                .destinations
                .iter()
                .find(|row| row.0 == 5)
                .map(|row| row.1),
            Some(3),
            "system 5 is reachable by the allowed detour through another constellation"
        );
        assert!(
            !spread.destinations.iter().any(|row| row.0 == 4),
            "system 4 is reachable only through blacklisted Jita"
        );
        assert!(
            !spread.destinations.iter().any(|row| row.0 == 6),
            "the transit constellation is not a destination constellation"
        );
    }

    #[test]
    fn burner_spread_cache_returns_the_same_computed_distribution() {
        let data = burner_graph();
        let first = data.burner_mission_spread(1).unwrap();
        let second = data.burner_mission_spread(1).unwrap();
        assert_eq!(
            first, second,
            "redrawing the Details pane does not change or recompute its result"
        );
    }

    #[test]
    fn route_bands_follow_the_lowest_security_flown() {
        // The worked example's four systems split 3/1 across bands 0.5 and 0.6 by their own
        // security, but every route starts at A, whose 0.49 is the lowest of the four, so the route
        // grouping puts everything in 0.5. `own_bands` keeps the split visible beside it.
        let chain = four_system_constellation(&[(1, 2), (2, 3), (3, 4)]);
        let star = four_system_constellation(&[(1, 2), (1, 3), (1, 4)]);
        for data in [&chain, &star] {
            let spread = data.mission_spread(1).expect("agent system is known");
            assert_eq!(spread.systems, 4);
            assert_eq!(
                spread.bands,
                vec![(5, 1.0)],
                "the start is the lowest system, so no route can be safer than it"
            );
            assert_eq!(
                spread.own_bands,
                vec![(5, 0.75), (6, 0.25)],
                "by their own security, three systems are 0.5 and one is 0.6"
            );
            assert!(
                spread.bands.iter().map(|(_, p)| p).sum::<f64>() > 0.999,
                "probabilities must sum to one"
            );
        }
    }

    #[test]
    fn a_higher_system_reached_through_a_lower_one_counts_as_lower() {
        // Start at D, the 0.6 system: it is the lowest on its own routes only until a path crosses
        // C at 0.51, so D's neighbours inherit the floor of whatever the route must fly.
        let chain = four_system_constellation(&[(1, 2), (2, 3), (3, 4)]);
        let spread = chain.mission_spread(4).unwrap();
        assert_eq!(
            spread.destinations,
            vec![(1, 3, 5, 5), (2, 2, 5, 5), (3, 1, 5, 5), (4, 0, 6, 6)],
            "(id, hops, route band, own band): only D itself stays in band 0.6"
        );
        assert_eq!(spread.bands, vec![(5, 0.75), (6, 0.25)]);
    }

    #[test]
    fn a_hop_group_follows_the_shape_of_the_constellation() {
        let chain = four_system_constellation(&[(1, 2), (2, 3), (3, 4)]);
        let star = four_system_constellation(&[(1, 2), (1, 3), (1, 4)]);
        assert_eq!(
            chain.mission_spread(1).unwrap().hops,
            vec![(0, 0.25), (1, 0.25), (2, 0.25), (3, 0.25)],
            "a line of four spreads over four distances"
        );
        assert_eq!(
            star.mission_spread(1).unwrap().hops,
            vec![(0, 0.25), (1, 0.75)],
            "a hub reaches everything in one jump"
        );
        assert_eq!(
            chain.mission_spread(3).unwrap().hops,
            vec![(0, 0.25), (1, 0.5), (2, 0.25)],
            "agent in system C: two neighbours, one two hops out"
        );
    }

    #[test]
    fn the_safest_route_is_preferred_and_the_shortest_only_breaks_ties() {
        // D is two gates away through B and two through C, so the hop count cannot choose between
        // the routes and only their lowest security can.
        let raised: [FixtureRow; 4] = [
            (1, ("A", 0.56, 7, 100)),
            (2, ("B", 0.55, 7, 100)),
            (3, ("C", 0.50, 7, 100)),
            (4, ("D", 0.60, 7, 100)),
        ];
        // A -> B -> D keeps a floor of 0.55; A -> C -> D only reaches 0.50.
        let data = fixture(&raised, &[(1, 2), (2, 4), (1, 3), (3, 4)]);
        let spread = data.mission_spread(1).unwrap();
        let to_d = spread
            .destinations
            .iter()
            .find(|(id, ..)| *id == 4)
            .expect("D");
        assert_eq!(
            to_d.2, 6,
            "floor 0.55 through B rounds to band 0.6, not 0.5 through C"
        );
        assert_eq!(to_d.1, 2, "both ways are two gates, so hops are unchanged");
    }

    #[test]
    fn destinations_carry_band_and_distance_together() {
        let star = four_system_constellation(&[(1, 2), (1, 3), (1, 4)]);
        let spread = star.mission_spread(1).unwrap();
        assert_eq!(
            spread.destinations,
            vec![(1, 0, 5, 5), (2, 1, 5, 5), (3, 1, 5, 5), (4, 1, 5, 6)],
            "(id, hops, route band, own band) in id order"
        );
    }

    #[test]
    fn an_unknown_agent_system_produces_no_spread() {
        let star = four_system_constellation(&[(1, 2), (1, 3), (1, 4)]);
        assert_eq!(star.mission_spread(99), None);
    }

    #[test]
    fn a_member_reachable_only_by_leaving_is_left_out_rather_than_guessed() {
        // System 4 hangs off the constellation through nothing: it cannot be reached while staying
        // inside, so it is dropped and the denominator follows the systems actually counted.
        let data = four_system_constellation(&[(1, 2), (2, 3)]);
        let spread = data.mission_spread(1).unwrap();
        assert_eq!(spread.systems, 3, "system 4 is not counted");
        assert_eq!(
            spread.own_bands,
            vec![(5, 1.0)],
            "the 0.6 band is gone with the unreachable system"
        );
    }

    #[test]
    fn hop_counts_from_jita_match_the_measured_constellation() {
        let map = StaticData::neighbour_map(KIMOTORO);
        let mut hops = std::collections::HashMap::from([(30000142, 0usize)]);
        let mut frontier = vec![30000142];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for system in frontier {
                for to in map[&system].iter().copied() {
                    if !hops.contains_key(&to) {
                        hops.insert(to, hops[&system] + 1);
                        next.push(to);
                    }
                }
            }
            frontier = next;
        }
        // Every system in the constellation is reachable without leaving it.
        let members: std::collections::HashSet<i32> =
            KIMOTORO.iter().flat_map(|(a, b)| [*a, *b]).collect();
        assert_eq!(members.len(), 7);
        for system in &members {
            assert!(hops.contains_key(system), "{system} unreachable from Jita");
        }
        let count_at = |d: usize| members.iter().filter(|s| hops[*s] == d).count();
        assert_eq!(count_at(0), 1, "Jita itself");
        assert_eq!(
            count_at(1),
            4,
            "Maurasi, Niyabainen, Perimeter, New Caldari"
        );
        assert_eq!(count_at(2), 1, "Urlen, reached through Perimeter");
        assert_eq!(count_at(3), 1, "Kisogo, reached through Urlen");
    }
}

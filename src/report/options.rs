use super::{Event, RunEvent};
use crate::esi::EsiClient;
use crate::static_data::{AgentFilter, SecurityFilter, StaticData};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// The region prices come from when `--region` is not given.
pub const DEFAULT_REGION_ID: i32 = 10000002;
const DEFAULT_MAX_INFLIGHT: usize = 4;
/// The offers printed per corporation group when `--depth` is not given.
pub const DEFAULT_DEPTH: usize = 10;

/// An option, identified by its command line flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Lpstore,
    Region,
    Security,
    AgentLevel,
    AgentDivision,
    AgentType,
    Faction,
    /// The region the agents must stand in, which is not the region prices are read from.
    AgentRegion,
    /// The system the agents must stand in.
    AgentSystem,
    System,
    CacheMinutes,
    Concurrency,
    MaxInflight,
    Depth,
}

impl Field {
    pub fn flag(self) -> &'static str {
        match self {
            Field::Lpstore => "--lpstore",
            Field::Region => "--region",
            Field::Security => "--security",
            Field::AgentLevel => "--agent-level",
            Field::AgentDivision => "--agent-division",
            Field::AgentType => "--agent-type",
            Field::AgentRegion => "--agent-region",
            Field::AgentSystem => "--agent-system",
            Field::Faction => "--faction",
            Field::System => "--system",
            Field::CacheMinutes => "--cache-minutes",
            Field::Concurrency => "--concurrency",
            Field::MaxInflight => "--max-inflight",
            Field::Depth => "--depth",
        }
    }
}

/// A problem with one option, with a message to show the user.
#[derive(Debug, Clone)]
pub struct OptionError {
    pub field: Field,
    pub message: String,
}

/// The options for `--lpstores`, as typed. Every value is text, so the command line and
/// the TUI form go through the same checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LpStoreOptions {
    /// Corporations as a comma-separated list of IDs or names. Without it, every corporation is used.
    pub lpstore: Option<String>,
    /// Market region as an ID or name. Defaults to The Forge.
    pub region: Option<String>,
    /// Security range such as `0.5..0.7`. See `SecurityFilter::parse`.
    pub security: Option<String>,
    /// Agent level from 1 to 5.
    pub agent_level: Option<String>,
    pub agent_division: Option<String>,
    pub agent_type: Option<String>,
    /// Region the agents must stand in, as an ID or name. Not the region prices come from, which is
    /// `region`: a corporation's agents can be anywhere while its LP store is priced in one region.
    pub agent_region: Option<String>,
    pub faction: Option<String>,
    /// Solar system the agents must stand in, as an ID or name.
    pub agent_system: Option<String>,
    /// Solar system as an ID or name. Prices come only from its NPC stations.
    pub system: Option<String>,
    pub cache_minutes: Option<String>,
    pub concurrency: Option<String>,
    pub max_inflight: Option<String>,
    pub depth: Option<String>,
    /// Where to keep ESI responses instead of the platform cache directory. The static data does
    /// not move with it: this names a cache.
    pub cache_dir: Option<String>,
    /// Ignore cached responses and call ESI again.
    pub expire: bool,
    pub verbose: bool,
}

/// Option values that can be checked without static data.
#[derive(Debug, Clone, Copy)]
pub struct Parsed {
    pub security: Option<SecurityFilter>,
    pub agent_level: Option<i32>,
    pub cache_minutes: Option<u64>,
    /// `None` means twice the number of tokio worker threads.
    pub concurrency: Option<usize>,
    pub max_inflight: usize,
    pub depth: usize,
}

/// A run of `--lpstores` with every option checked and every name resolved to an ID.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Corporations to process, after the agent and faction filters.
    pub corp_ids: Vec<i32>,
    pub filter_summary: Option<FilterSummary>,
    pub agent_filter: AgentFilter,
    /// The agent-side region and system filters as typed and resolved, kept apart from
    /// `agent_filter` because the view asks what they name in order to say so, and to check the
    /// pair against each other the way the market pair is checked.
    pub agent_region: Option<NamedValue>,
    pub agent_system: Option<NamedValue>,
    pub region_id: i32,
    pub region_label: String,
    pub system: Option<SystemSummary>,
    /// Stations that prices come from, when a system is set.
    pub locations: Option<Arc<HashSet<i64>>>,
    pub cache_offset: Duration,
    pub concurrency: usize,
    pub max_inflight: usize,
    pub depth: usize,
    pub expire: bool,
    pub verbose: bool,
}

/// How many corporations the agent and faction filters kept.
#[derive(Debug, Clone, Copy)]
pub struct FilterSummary {
    pub kept: usize,
    pub removed: usize,
}

/// An ID-or-name filter value as typed, and the ID it resolved to. Used by the agent-side region and
/// system filters, which name a place but price nothing, so they carry no station list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedValue {
    pub value: String,
    pub id: i32,
    /// What the id is called in the static data, or the id itself when the data does not name it.
    pub label: String,
}

/// The `--system` value as typed, the system it names, and the number of NPC stations in it.
#[derive(Debug, Clone)]
pub struct SystemSummary {
    pub value: String,
    /// The system the typed value resolved to, so a caller can ask anything of it without
    /// resolving the name a second time.
    pub system_id: i32,
    pub stations: usize,
}

impl LpStoreOptions {
    /// Checks the values that don't need static data.
    pub fn parse(&self) -> Result<Parsed, Vec<OptionError>> {
        let mut errors = Vec::new();
        let parsed = self.parse_values(&mut errors);
        if errors.is_empty() {
            Ok(parsed)
        } else {
            Err(errors)
        }
    }

    /// Checks every option and resolves names to IDs. Returns every problem found.
    ///
    /// Must be called inside a tokio runtime, which supplies the default concurrency.
    pub fn resolve(&self, sdata: &StaticData) -> Result<Resolved, Vec<OptionError>> {
        let mut errors = Vec::new();
        let parsed = self.parse_values(&mut errors);

        let mut corp_ids: Vec<i32> = match self.lpstore.as_deref() {
            Some(text) => text
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .filter_map(|entry| match entry.parse::<i32>() {
                    Ok(id) => Some(id),
                    Err(_) => {
                        let id = sdata.corporation_id(entry);
                        if id.is_none() {
                            errors.push(OptionError {
                                field: Field::Lpstore,
                                message: format!("Corporation '{}' not found", entry),
                            });
                        }
                        id
                    }
                })
                .collect(),
            None => {
                let mut ids: Vec<i32> = sdata.corporation_ids().collect();
                ids.sort_unstable();
                ids
            }
        };

        let division_id = self.agent_division.as_deref().and_then(|name| {
            let id = sdata.division_id(name);
            if id.is_none() {
                errors.push(OptionError {
                    field: Field::AgentDivision,
                    message: format!(
                        "Unknown --agent-division '{}'. Valid divisions: {}",
                        name,
                        sdata.division_names().join(", ")
                    ),
                });
            }
            id
        });
        let agent_type_id = self.agent_type.as_deref().and_then(|name| {
            let id = sdata.agent_type_id(name);
            if id.is_none() {
                errors.push(OptionError {
                    field: Field::AgentType,
                    message: format!(
                        "Unknown --agent-type '{}'. Valid types: {}",
                        name,
                        sdata.agent_type_names().join(", ")
                    ),
                });
            }
            id
        });
        let faction_id = self.faction.as_deref().and_then(|name| {
            let id = sdata.faction_id(name);
            if id.is_none() {
                errors.push(OptionError {
                    field: Field::Faction,
                    message: format!(
                        "Unknown --faction '{}'. Valid factions: {}",
                        name,
                        sdata.faction_names().join(", ")
                    ),
                });
            }
            id
        });

        // Both are resolved the way the market pair is: a number is taken as an ID without checking
        // that it exists, and a name is looked up and refused when nothing answers it.
        let agent_region = self.agent_region.as_deref().and_then(|value| {
            let id = match value.parse::<i32>() {
                Ok(id) => id,
                Err(_) => match sdata.region_id(value) {
                    Some(id) => id,
                    None => {
                        errors.push(OptionError {
                            field: Field::AgentRegion,
                            message: format!("Region '{}' not found", value),
                        });
                        return None;
                    }
                },
            };
            Some(NamedValue {
                label: sdata.region_label(id),
                value: value.to_string(),
                id,
            })
        });
        let agent_system =
            self.agent_system
                .as_deref()
                .and_then(|value| match sdata.system_id(value) {
                    Some(id) => Some(NamedValue {
                        label: sdata
                            .system_name(id)
                            .cloned()
                            .unwrap_or_else(|| id.to_string()),
                        value: value.to_string(),
                        id,
                    }),
                    None => {
                        errors.push(OptionError {
                            field: Field::AgentSystem,
                            message: format!("Unknown --agent-system '{}'", value),
                        });
                        None
                    }
                });

        let agent_filter = AgentFilter {
            security: parsed.security,
            level: parsed.agent_level,
            division_id,
            agent_type_id,
            region: agent_region.as_ref().map(|found| found.id),
            system: agent_system.as_ref().map(|found| found.id),
        };
        let agent_filtering = parsed.security.is_some()
            || parsed.agent_level.is_some()
            || division_id.is_some()
            || agent_type_id.is_some()
            || agent_region.is_some()
            || agent_system.is_some();
        let mut filter_summary = None;
        if agent_filtering || faction_id.is_some() {
            let before = corp_ids.len();
            corp_ids.retain(|&id| {
                faction_id.map_or(true, |f| sdata.corporation_in_faction(id, f))
                    && (!agent_filtering || sdata.corporation_passes(id, &agent_filter))
            });
            filter_summary = Some(FilterSummary {
                kept: corp_ids.len(),
                removed: before - corp_ids.len(),
            });
        }

        let region_id = match self.region.as_deref() {
            Some(value) => match value.parse::<i32>() {
                Ok(id) => id,
                Err(_) => sdata.region_id(value).unwrap_or_else(|| {
                    errors.push(OptionError {
                        field: Field::Region,
                        message: format!("Region '{}' not found", value),
                    });
                    DEFAULT_REGION_ID
                }),
            },
            None => DEFAULT_REGION_ID,
        };
        let region_label = sdata.region_label(region_id);

        let (locations, system) = match self.system.as_deref() {
            Some(value) => match sdata.system_id(value) {
                Some(system_id) => {
                    let stations = sdata.station_ids_in_system(system_id);
                    let summary = SystemSummary {
                        value: value.to_string(),
                        system_id,
                        stations: stations.len(),
                    };
                    (Some(Arc::new(stations)), Some(summary))
                }
                None => {
                    errors.push(OptionError {
                        field: Field::System,
                        message: format!("Unknown --system '{}'", value),
                    });
                    (None, None)
                }
            },
            None => (None, None),
        };

        if !errors.is_empty() {
            return Err(errors);
        }

        let cache_offset =
            Duration::from_secs(parsed.cache_minutes.unwrap_or(corp_ids.len() as u64) * 60);
        let concurrency = parsed
            .concurrency
            .unwrap_or_else(|| 2 * tokio::runtime::Handle::current().metrics().num_workers());

        Ok(Resolved {
            corp_ids,
            filter_summary,
            agent_filter,
            agent_region,
            agent_system,
            region_id,
            region_label,
            system,
            locations,
            cache_offset,
            concurrency,
            max_inflight: parsed.max_inflight,
            depth: parsed.depth,
            expire: self.expire,
            verbose: self.verbose,
        })
    }

    fn parse_values(&self, errors: &mut Vec<OptionError>) -> Parsed {
        Parsed {
            security: parse_option(
                self.security.as_deref(),
                Field::Security,
                errors,
                SecurityFilter::parse,
            ),
            agent_level: parse_option(
                self.agent_level.as_deref(),
                Field::AgentLevel,
                errors,
                parse_agent_level,
            ),
            cache_minutes: parse_option(
                self.cache_minutes.as_deref(),
                Field::CacheMinutes,
                errors,
                parse_minutes,
            ),
            concurrency: parse_option(
                self.concurrency.as_deref(),
                Field::Concurrency,
                errors,
                parse_count,
            ),
            max_inflight: parse_option(
                self.max_inflight.as_deref(),
                Field::MaxInflight,
                errors,
                parse_count,
            )
            .unwrap_or(DEFAULT_MAX_INFLIGHT),
            depth: parse_option(self.depth.as_deref(), Field::Depth, errors, parse_count)
                .unwrap_or(DEFAULT_DEPTH),
        }
    }
}

impl Resolved {
    /// Builds the ESI client for this run. Verbose log lines go to `events` as
    /// `Event::Log`, tagged with `run_id`.
    pub fn client(
        &self,
        cache_dir: &std::path::Path,
        run_id: u64,
        events: UnboundedSender<RunEvent>,
    ) -> EsiClient {
        EsiClient::builder()
            .cache_dir(cache_dir.to_path_buf())
            .expire(self.expire)
            .verbose(self.verbose)
            .cache_expiry_offset(self.cache_offset)
            .max_inflight(self.max_inflight)
            .log(move |message| {
                let _ = events.send(RunEvent {
                    run: run_id,
                    event: Event::Log(message),
                });
            })
            .build()
    }
}

fn parse_option<T>(
    value: Option<&str>,
    field: Field,
    errors: &mut Vec<OptionError>,
    parse: fn(&str) -> Result<T, String>,
) -> Option<T> {
    let value = value?;
    match parse(value) {
        Ok(parsed) => Some(parsed),
        Err(reason) => {
            errors.push(OptionError {
                field,
                message: format!("Invalid {} value '{}': {}", field.flag(), value, reason),
            });
            None
        }
    }
}

fn parse_count(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err("expected a whole number of at least 1".to_string()),
    }
}

fn parse_minutes(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| "expected a whole number".to_string())
}

fn parse_agent_level(value: &str) -> Result<i32, String> {
    match value.parse::<i32>() {
        Ok(level) if (1..=5).contains(&level) => Ok(level),
        _ => Err("expected a whole number from 1 to 5".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reports_each_bad_value_with_its_field() {
        let options = LpStoreOptions {
            security: Some("x0.5".to_string()),
            agent_level: Some("9".to_string()),
            depth: Some("0".to_string()),
            ..Default::default()
        };
        let fields: Vec<Field> = options
            .parse()
            .unwrap_err()
            .iter()
            .map(|e| e.field)
            .collect();
        assert_eq!(fields, [Field::Security, Field::AgentLevel, Field::Depth]);
    }

    // `resolve` asks the runtime for its worker count, so these run inside one.
    #[tokio::test]
    async fn an_agent_region_written_as_an_id_needs_no_static_data() {
        // A numeric region is taken as an ID without checking it exists, exactly as `--region`
        // behaves, so a typo in a number names a region nothing stands in rather than failing to
        // parse. An empty `StaticData` is the state before the archive is fetched.
        let sdata = StaticData::default();
        let options = LpStoreOptions {
            lpstore: Some("1000002".to_string()),
            agent_region: Some("10000043".to_string()),
            ..Default::default()
        };
        let resolved = options.resolve(&sdata).expect("an id is accepted");
        assert_eq!(
            resolved.agent_region,
            Some(NamedValue {
                value: "10000043".to_string(),
                id: 10000043,
                // Nothing names it on an empty cache, so the id stands in for the name.
                label: "10000043".to_string(),
            })
        );
        assert_eq!(resolved.agent_filter.region, Some(10000043));
        assert_eq!(
            resolved.agent_filter.system, None,
            "only the region was asked for"
        );
    }

    // `resolve` asks the runtime for its worker count, so these run inside one.
    #[tokio::test]
    async fn an_agent_region_name_no_data_names_is_refused_on_its_own_field() {
        let sdata = StaticData::default();
        let options = LpStoreOptions {
            agent_region: Some("Black Rise".to_string()),
            ..Default::default()
        };
        let errors = options
            .resolve(&sdata)
            .expect_err("no data names that region");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, Field::AgentRegion);
        assert_eq!(errors[0].message, "Region 'Black Rise' not found");
    }

    // `resolve` asks the runtime for its worker count, so these run inside one.
    #[tokio::test]
    async fn an_agent_system_no_data_names_is_refused_with_its_flag() {
        // `--agent-system` must name a system the static data holds, as `--system` must, because a
        // name is looked up rather than trusted.
        let sdata = StaticData::default();
        let options = LpStoreOptions {
            agent_system: Some("Jita".to_string()),
            ..Default::default()
        };
        let errors = options
            .resolve(&sdata)
            .expect_err("no data names that system");
        assert_eq!(errors[0].field, Field::AgentSystem);
        assert_eq!(errors[0].message, "Unknown --agent-system 'Jita'");
    }

    #[test]
    fn the_agent_place_filters_are_flags_the_filter_list_carries() {
        // The two new filters must reach the field list, or the view could not show an error beside
        // the box that caused it.
        assert_eq!(Field::AgentRegion.flag(), "--agent-region");
        assert_eq!(Field::AgentSystem.flag(), "--agent-system");
    }

    // `resolve` asks the runtime for its worker count, so these run inside one.
    #[tokio::test]
    async fn an_agent_place_filter_narrows_the_corporations_even_on_an_empty_cache() {
        // With no agent rows at all, a corporation list that would otherwise stand is filtered to
        // nothing, which is the honest answer on a cache that cannot confirm an agent stands there.
        let sdata = StaticData::default();
        let plain = LpStoreOptions {
            lpstore: Some("1000002".to_string()),
            ..Default::default()
        };
        assert_eq!(
            plain.resolve(&sdata).expect("no filters").corp_ids,
            vec![1000002],
            "without a filter the named corporation stands"
        );
        // A system, named or numbered, has to be one the data knows, so the narrowing is tested
        // with a region id, which is taken as written.
        let placed = LpStoreOptions {
            lpstore: Some("1000002".to_string()),
            agent_region: Some("10000043".to_string()),
            ..Default::default()
        };
        let resolved = placed.resolve(&sdata).expect("a region id is accepted");
        assert!(
            resolved.corp_ids.is_empty(),
            "the filter asked for a system nothing is recorded in"
        );
        assert_eq!(
            resolved.filter_summary.map(|summary| summary.removed),
            Some(1),
            "and the view is told one corporation was removed"
        );
    }

    #[test]
    fn parse_uses_defaults_when_values_are_missing() {
        let parsed = LpStoreOptions::default().parse().unwrap();
        assert_eq!((parsed.max_inflight, parsed.depth), (4, 10));
        assert!(parsed.concurrency.is_none());
        assert!(parsed.cache_minutes.is_none());
    }
}

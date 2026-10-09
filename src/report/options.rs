use super::{Event, RunEvent};
use crate::esi::EsiClient;
use crate::static_data::{AgentFilter, SecurityFilter, StaticData};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const DEFAULT_REGION_ID: i32 = 10000002;
const DEFAULT_MAX_INFLIGHT: usize = 4;
const DEFAULT_DEPTH: usize = 10;

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
#[derive(Debug, Clone, Default)]
pub struct LpStoreOptions {
    /// Corporations as a comma-separated list of IDs or names. Without it, every corporation is used.
    pub lpstore: Option<String>,
    /// Market region as an ID or name. Defaults to The Forge.
    pub region: Option<String>,
    /// Security filter such as `+0.5`.
    pub security: Option<String>,
    /// Agent level from 1 to 5.
    pub agent_level: Option<String>,
    pub agent_division: Option<String>,
    pub agent_type: Option<String>,
    pub faction: Option<String>,
    /// Solar system as an ID or name. Prices come only from its NPC stations.
    pub system: Option<String>,
    pub cache_minutes: Option<String>,
    pub concurrency: Option<String>,
    pub max_inflight: Option<String>,
    pub depth: Option<String>,
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

/// The `--system` value as typed, and the number of NPC stations in that system.
#[derive(Debug, Clone)]
pub struct SystemSummary {
    pub value: String,
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

        let agent_filter = AgentFilter {
            security: parsed.security,
            level: parsed.agent_level,
            division_id,
            agent_type_id,
        };
        let agent_filtering = parsed.security.is_some()
            || parsed.agent_level.is_some()
            || division_id.is_some()
            || agent_type_id.is_some();
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
        let region_label = match sdata.region_name(region_id) {
            Some(name) => format!("{} ({})", name, region_id),
            None => region_id.to_string(),
        };

        let (locations, system) = match self.system.as_deref() {
            Some(value) => match sdata.system_id(value) {
                Some(system_id) => {
                    let stations = sdata.station_ids_in_system(system_id);
                    let summary = SystemSummary {
                        value: value.to_string(),
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
    pub fn client(&self, run_id: u64, events: UnboundedSender<RunEvent>) -> EsiClient {
        EsiClient::builder()
            .cache_dir("cache")
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

    #[test]
    fn parse_uses_defaults_when_values_are_missing() {
        let parsed = LpStoreOptions::default().parse().unwrap();
        assert_eq!((parsed.max_inflight, parsed.depth), (4, 10));
        assert!(parsed.concurrency.is_none());
        assert!(parsed.cache_minutes.is_none());
    }
}

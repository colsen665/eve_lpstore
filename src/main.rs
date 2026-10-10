mod output;
mod tui;

use eve_lpstore::esi::ClientStats;
use eve_lpstore::report::{self, LpStoreOptions, OptionError};
use eve_lpstore::{EsiClient, Paths, StaticData};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

fn print_status(s: &ClientStats) {
    let now = chrono::Local::now().format("%H:%M:%S");
    println!("[{}] {}", now, output::status_text(s));
}

const HELP_TEXT: &str = "\
Usage: eve_lpstore <mode> [FILTERS] [OPTIONS]

Modes:
  --example              Run the general ESI and static data example.
  --lpstores             Calculate expenses, profit, and ISK per LP for LP store offers and show the top 10.
  --tui                  Show --lpstores in a full-screen view that updates while it runs. Asks for
                         the filters first, and for the static data if it is not on disk. Remembers
                         the filters it applies in filters.json. Implies --lpstores.
  --help                 Show this help and exit.

Filters (apply to --lpstores):
  --lpstore <list>       Corporations to process, as a comma-separated list of IDs or names. Without it, all corporations are processed.
  --security <level|range> Keep corporations with an agent in a system whose rounded security is the level, or inside the range. Write 0.5 for exactly, 0.5..0.7 for a range with both bounds included, 0.5.. for at least, and ..0.7 for at most. A level can be negative, so the old -0.5 no longer means at most. Rounding: https://wiki.eveuniversity.org/System_security
  --agent-level <n>      Keep corporations with an agent of level n (1 to 5).
  --agent-division <name>  Keep corporations with an agent in this division, e.g. security or distribution.
  --agent-type <name>    Keep corporations with an agent of this type, e.g. basic, research, concord (the Agent suffix is optional).
  --system <name|id>     Only use buy and sell orders at NPC stations in this system. Applies to --lpstores.
  --faction <name>       Keep corporations of this faction, e.g. caldari, amarr, gallente. Applies to --lpstores.
  All filters given must match the same agent.

Options:
  --region <id|name>     Market region used for pricing, by ID or name. Defaults to The Forge (10000002).
  --status               Print API call status every second while running. With --tui, the footer shows it instead.
  --expire               Ignore cached responses and call the API again.
  --cache-minutes <n>    Extend how long cached responses are reused, by n minutes past their Expires time. Default 0 for --example, and the number of corporations for --lpstores. Use 0 to turn it off.
  --cache-dir <path>     Where to keep ESI responses, instead of the platform cache directory. The static data does not move with it.
  --concurrency <n>      Corporation groups priced at once with --lpstores. Default 2x the tokio worker threads.
  --max-inflight <n>     Maximum ESI requests in flight at once across all groups. Default 4.
  --depth <n>            Offers printed per corporation group with --lpstores. Default 10.
  --verbose              Print cache hits, misses, and writes, with reasons and file names. With --tui, they go to the log.
  --clear-cache          Delete the cached ESI responses, and the static data archive if one is on disk. The four extracted static data files stay.

Examples:
  eve_lpstore --example
      Run the general ESI and static data example.

  eve_lpstore --lpstores
      Show the top 10 offers across all corporations, priced in The Forge.

  eve_lpstore --lpstores --lpstore 1000002
      Show the top 10 offers for one corporation, by ID.

  eve_lpstore --lpstores --lpstore \"Caldari Navy,Amarr Navy\" --region Domain
      Show the top 10 offers for two corporations by name, priced in Domain.

  eve_lpstore --lpstores --security 0.5..0.7
      Show the top 10 offers for corporations with an agent in a system whose rounded security is between 0.5 and 0.7, both included.

  eve_lpstore --lpstores --expire --verbose
      Refresh cached responses and show cache activity.

  eve_lpstore --lpstores --concurrency 8
      Price up to 8 corporation groups at once.

  eve_lpstore --lpstores --max-inflight 8
      Allow up to 8 ESI requests in flight at once.

  eve_lpstore --tui
      Ask for the filters, then watch every corporation price in a full-screen view. Enter starts
      the run, and ? shows the keys once it is going.

  eve_lpstore --tui --lpstore \"Caldari Navy\" --region Domain --depth 20
      Open that view for one corporation in Domain. These fill the form the view starts on, and
      each one still wins over the value an earlier run left in filters.json.

  eve_lpstore --tui --faction Caldari
      Start from the filters the last run applied, with only the faction overridden. The view keeps
      what it applies, so a field left off the command line comes back on its own.

  eve_lpstore --tui --verbose
      Send cache hits, misses, and writes to the log panel that l opens, not to the terminal.

  eve_lpstore --clear-cache --help
      Show this help without clearing the cache. (Yes, this entry was generated by AI)
";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help") {
        print!("{}", HELP_TEXT);
        return;
    }

    const FLAGS: &[&str] = &[
        "--example",
        "--lpstores",
        "--tui",
        "--status",
        "--expire",
        "--verbose",
        "--clear-cache",
        "--help",
    ];
    const VALUE_FLAGS: &[&str] = &[
        "--lpstore",
        "--region",
        "--cache-minutes",
        "--concurrency",
        "--max-inflight",
        "--security",
        "--agent-level",
        "--agent-division",
        "--agent-type",
        "--faction",
        "--depth",
        "--system",
        "--cache-dir",
    ];

    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();
        if VALUE_FLAGS.contains(&arg) {
            i += 2;
            continue;
        }
        if !FLAGS.contains(&arg) {
            eprintln!("Unknown argument '{}'", arg);
            eprintln!("Run {} --help to see the available arguments", args[0]);
            std::process::exit(1);
        }
        i += 1;
    }

    let tui = args.iter().any(|a| a == "--tui");
    if tui && args.iter().any(|a| a == "--example") {
        eprintln!("--tui cannot be combined with --example");
        std::process::exit(1);
    }
    if tui && !std::io::stdout().is_terminal() {
        eprintln!("--tui needs an interactive terminal on stdout");
        std::process::exit(1);
    }

    let value = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let status = args.iter().any(|a| a == "--status");
    let expire = args.iter().any(|a| a == "--expire");
    let clear_cache = args.iter().any(|a| a == "--clear-cache");
    let verbose = args.iter().any(|a| a == "--verbose");
    let options = LpStoreOptions {
        lpstore: value("--lpstore"),
        region: value("--region"),
        security: value("--security"),
        agent_level: value("--agent-level"),
        agent_division: value("--agent-division"),
        agent_type: value("--agent-type"),
        faction: value("--faction"),
        system: value("--system"),
        cache_minutes: value("--cache-minutes"),
        concurrency: value("--concurrency"),
        max_inflight: value("--max-inflight"),
        depth: value("--depth"),
        cache_dir: value("--cache-dir"),
        expire,
        verbose,
    };
    // Every mode that runs needs both roots, including a bare --clear-cache, so the home directory
    // is settled before any of them. Refusing here rather than falling back to the working
    // directory keeps the relative-path behaviour this replaces from creeping back in.
    let paths = match Paths::discover()
        .map(|found| found.with_cache_override(options.cache_dir.as_deref()))
    {
        Some(paths) => paths,
        None => {
            eprintln!(
                "No home directory is known to this system, so there is nowhere to keep the ESI \
                 cache or the static data. Set HOME, or USERPROFILE on Windows, and run again."
            );
            std::process::exit(1);
        }
    };
    if verbose {
        println!("Cache: {}", paths.cache.display());
        println!("Static data: {}", paths.data.display());
        warn_ignored_local_dirs();
    }
    if args.iter().any(|a| a == "--example") {
        let parsed = match options.parse() {
            Ok(parsed) => parsed,
            Err(errors) => exit_with_errors(errors),
        };
        if clear_cache {
            clear_cached_files(&paths.cache, &paths.data);
        }
        let cache_offset = std::time::Duration::from_secs(parsed.cache_minutes.unwrap_or(0) * 60);
        if let Err(e) = run_example(
            status,
            expire,
            verbose,
            options.region.as_deref(),
            cache_offset,
            parsed.max_inflight,
            &paths,
        )
        .await
        {
            eprintln!("Example failed: {}", e);
            std::process::exit(1);
        }
    } else if tui || args.iter().any(|a| a == "--lpstores") {
        run_lpstores(status, options, clear_cache, tui, paths).await;
    } else {
        if let Err(errors) = options.parse() {
            exit_with_errors(errors);
        }
        if clear_cache {
            clear_cached_files(&paths.cache, &paths.data);
        } else {
            println!(
                "Usage: {} [--example|--lpstores|--tui] [--status] [--expire] [--cache-minutes <n>] [--cache-dir <path>] [--concurrency <n>] [--max-inflight <n>] [--depth <n>] [--system <name|id>] [--security <level|range>] [--agent-level <n>] [--agent-division <name>] [--agent-type <name>] [--faction <name>] [--verbose] [--clear-cache] [--lpstore <id|name,...>] [--region <id|name>]",
                args[0]
            );
        }
    }
}

fn exit_with_errors(errors: Vec<OptionError>) -> ! {
    for error in errors {
        eprintln!("{}", error.message);
    }
    std::process::exit(1);
}

/// Deletes the cached responses and the archive, leaving the directories and the extracted static
/// data in place.
///
/// `--clear-cache` used to be `remove_dir_all("cache")`. With the roots now absolute and shared,
/// wiping a whole directory is a heavier thing to do by accident, and the static data root costs
/// an archive download to refill, so this removes files it recognises instead: the JSON responses
/// and the orphaned temporary files of the cache root, and the SDE archive in the data root if one
/// is ever there.
fn clear_cached_files(cache: &Path, data: &Path) {
    let mut removed = 0;
    let mut failures = 0;
    match std::fs::read_dir(cache) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let cached = name.ends_with(".json");
                // The atomic write names its temporary file `.{name}.json.{pid}.tmp`, so a
                // json-only rule would leave a crash's leftovers behind for good.
                let orphan = name.starts_with('.') && name.ends_with(".tmp");
                if !cached && !orphan {
                    continue;
                }
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => removed += 1,
                    Err(error) => {
                        failures += 1;
                        eprintln!("Failed to remove {}: {}", entry.path().display(), error);
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("No cache to clear in {}", cache.display());
        }
        Err(error) => eprintln!("Failed to read {}: {}", cache.display(), error),
    }
    if removed > 0 || failures == 0 {
        println!(
            "Cleared {removed} cached responses from {}",
            cache.display()
        );
    }

    // Nothing writes this today, since the archive is unzipped in memory, so the check is here in
    // case that changes rather than because it matches anything now.
    let archive = data.join(eve_lpstore::static_data::SDE_ARCHIVE_NAME);
    match std::fs::remove_file(&archive) {
        Ok(()) => println!("Removed the static data archive {}", archive.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("Failed to remove {}: {}", archive.display(), error),
    }
}

/// Notes the two directories this used to write in the working directory. They are not read any
/// more, so their contents are dead weight until someone deletes them.
fn warn_ignored_local_dirs() {
    let mut left = Vec::new();
    for name in ["cache", "data"] {
        if Path::new(name).is_dir() {
            left.push(name);
        }
    }
    if !left.is_empty() {
        println!(
            "Note: {} in the working directory is no longer used; the files there can be deleted.",
            left.join(" and ")
        );
    }
}

/// Says what is about to happen before the static data archive is fetched. Loading it pulls the
/// whole SDE download, which takes long enough that a silent terminal looks hung.
fn announce_static_data_download(data: &Path) {
    if StaticData::is_cached(data) {
        return;
    }
    println!(
        "Static data not found, downloading it to {}...",
        data.display()
    );
}

async fn run_example(
    status: bool,
    expire: bool,
    verbose: bool,
    region: Option<&str>,
    cache_offset: std::time::Duration,
    max_inflight: usize,
    paths: &Paths,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load static data (only downloads if cache is missing or forced)
    announce_static_data_download(&paths.data);
    let sdata = match StaticData::load(false, &paths.data).await {
        Ok(data) => {
            println!(
                "Static data loaded: {} regions",
                data.region_names().count()
            );
            data
        }
        Err(e) => {
            eprintln!("Failed to load static data: {}", e);
            return Err(e.into());
        }
    };

    // Look up region IDs by name
    for name in &["The Forge", "Domain", "Sinq Laison", "Heimatar"] {
        match sdata.region_id(name) {
            Some(id) => println!("  {} → region_id {}", name, id),
            None => println!("  {} → not found", name),
        }
    }

    // Look up corporation IDs by name
    for name in &[
        "Amarr Navy",
        "Caldari Navy",
        "Guristas Production",
        "Non-existent Corp",
    ] {
        match sdata.corporation_id(name) {
            Some(id) => println!("  {} → corporation_id {}", name, id),
            None => println!("  {} → not found", name),
        }
    }

    // Resolve the region from --region, or default to The Forge
    let region_id = match region {
        Some(val) => match val.parse::<i32>() {
            Ok(id) => id,
            Err(_) => match sdata.region_id(val) {
                Some(id) => id,
                None => {
                    eprintln!("Region '{}' not found", val);
                    std::process::exit(1);
                }
            },
        },
        None => 10000002,
    };
    let region_label = match sdata.region_name(region_id) {
        Some(name) => format!("{} ({})", name, region_id),
        None => region_id.to_string(),
    };
    println!("Region: {}", region_label);

    let esi = EsiClient::builder()
        .cache_dir(paths.cache.clone())
        .expire(expire)
        .verbose(verbose)
        .cache_expiry_offset(cache_offset)
        .max_inflight(max_inflight)
        .log(|message| println!("{}", message))
        .build();

    // Spawn status monitoring task if --status is set
    let status_task = if status {
        Some(esi.spawn_status_monitor(print_status))
    } else {
        None
    };

    // NPC corporations
    match esi.corporations().npc_corps().await {
        Ok(corps) => println!("NPC corporations: {} found", corps.len()),
        Err(e) => eprintln!("Failed to fetch NPC corporations: {}", e),
    }

    // Market orders in the resolved region, all types
    match esi.markets().orders(region_id, None).await {
        Ok(orders) => println!("Market orders: {} found", orders.len()),
        Err(e) => eprintln!("Failed to fetch market orders: {}", e),
    }

    // All published type IDs
    match esi.universe().types().await {
        Ok(types) => println!("Published types: {} found", types.len()),
        Err(e) => eprintln!("Failed to fetch types: {}", e),
    }

    // Detailed info for a specific type (example: Tritanium, type_id 34)
    match esi.universe().type_info(34).await {
        Ok(info) => println!("Type {}: {}", info.type_id, info.name),
        Err(e) => eprintln!("Failed to fetch type info: {}", e),
    }

    // Stop the status monitoring task
    if let Some(task) = status_task {
        task.abort();
    }

    Ok(())
}

async fn run_lpstores(
    status: bool,
    options: LpStoreOptions,
    clear_cache: bool,
    tui: bool,
    paths: Paths,
) {
    if tui {
        if clear_cache {
            clear_cached_files(&paths.cache, &paths.data);
        }
        // The view asks about the static data itself, and resolves the filters when they are
        // applied, so nothing is loaded or fetched before it can say so on screen.
        if let Err(e) = tui::run(options, paths).await {
            eprintln!("TUI failed: {}", e);
            std::process::exit(1);
        }
        return;
    }
    announce_static_data_download(&paths.data);
    let sdata = match StaticData::load(false, &paths.data).await {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to load static data: {}", e);
            std::process::exit(1);
        }
    };
    let resolved = match options.resolve(&sdata) {
        Ok(resolved) => resolved,
        Err(errors) => exit_with_errors(errors),
    };
    if clear_cache {
        clear_cached_files(&paths.cache, &paths.data);
    }
    output::print_resolution(&resolved);

    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let client = Arc::new(resolved.client(&paths.cache, 0, events.clone()));
    let status_task = status.then(|| client.spawn_status_monitor(print_status));
    let printing = async {
        while let Some(run_event) = received.recv().await {
            output::print_event(&run_event.event, &sdata);
        }
    };
    let (report_data, ()) = tokio::join!(report::run(&resolved, client, 0, events), printing);
    output::print_report(&report_data, &resolved, &sdata);

    if let Some(task) = status_task {
        task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh directory under the system temp directory, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "eve_lpstore_clear_{}_{}_{}",
                std::process::id(),
                name,
                uuid_like()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, dir: &Path, name: &str) {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(name), "x").unwrap();
        }
        fn names(&self, dir: &Path) -> Vec<String> {
            match std::fs::read_dir(dir) {
                Ok(entries) => entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn uuid_like() -> u32 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::time::Instant::now().hash(&mut hasher);
        (hasher.finish() % 100_000) as u32
    }

    #[test]
    fn clearing_removes_responses_and_temp_orphans_but_not_the_directory() {
        let temp = TempDir::new("files");
        let cache = temp.0.join("cache");
        for name in [
            "loyalty_stores_1000002_offers.json",
            "markets_10000002_orders.json",
            ".markets_10000002_orders.json.4242.tmp",
        ] {
            temp.write(&cache, name);
        }
        // Something the program did not put there is left alone.
        temp.write(&cache, "notes.txt");
        let data = temp.0.join("data");
        for name in ["regions.json", "corporations.json", "types.json"] {
            temp.write(&data, name);
        }

        clear_cached_files(&cache, &data);

        assert!(cache.is_dir(), "the directory itself survives");
        assert_eq!(temp.names(&cache), ["notes.txt"]);
        assert!(data.is_dir(), "the static data root survives");
        for name in ["regions.json", "corporations.json", "types.json"] {
            assert!(data.join(name).exists(), "{name} was deleted");
        }
    }

    #[test]
    fn clearing_removes_a_stored_archive_but_keeps_the_extracted_files() {
        // Nothing writes the archive today, since it is unzipped in memory, so this is the case
        // where it does.
        let temp = TempDir::new("archive");
        let cache = temp.0.join("cache");
        let data = temp.0.join("data");
        temp.write(&cache, "a.json");
        temp.write(&data, eve_lpstore::static_data::SDE_ARCHIVE_NAME);
        temp.write(&data, "regions.json");

        clear_cached_files(&cache, &data);

        assert!(
            !data
                .join(eve_lpstore::static_data::SDE_ARCHIVE_NAME)
                .exists()
        );
        assert!(data.join("regions.json").exists());
        assert!(!cache.join("a.json").exists());
    }

    #[test]
    fn clearing_missing_directories_is_not_an_error() {
        let temp = TempDir::new("absent");
        let cache = temp.0.join("no-cache");
        let data = temp.0.join("no-data");
        clear_cached_files(&cache, &data);
        assert!(!cache.exists());
        assert!(!data.exists());
    }
}

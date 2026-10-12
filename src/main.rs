mod output;
mod tui;

use eve_lpstore::esi::ClientStats;
use eve_lpstore::report::{self, Field, LpStoreOptions, OptionError};
use eve_lpstore::static_data::StaticDataError;
use eve_lpstore::{EsiClient, Paths, StaticData};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

fn print_status(s: &ClientStats) {
    let now = chrono::Local::now().format("%H:%M:%S");
    println!("[{}] {}", now, output::status_text(s));
}

const HELP_TEXT: &str = "\
Usage: eve_lpstore [MODE] [FILTERS] [OPTIONS]

Modes:
  --example              Run the general ESI and static data example.
  --lpstores             Calculate expenses, profit, and ISK per LP for LP store offers and show the top 10, as text on standard output.
  --tui                  Show --lpstores in a full-screen view that updates while it runs. Asks for
                         the filters first, and for the static data if it is not on disk. Remembers
                         the filters it applies in a file in the platform config directory.
                         This is the default when no mode is given and standard output is a terminal;
                         with no terminal it prints the usage line instead.
  --help                 Show this help and exit.
  --version              Show the version and exit.

Filters (apply to --lpstores):
  --lpstore <id|name,...> Corporations to process, as a comma-separated list of IDs or names. Without it, all corporations are processed.
  --security <level|range> Keep corporations with an agent in a system whose rounded security is the level, or inside the range. Write 0.5 for exactly, 0.5..0.7 for a range with both bounds included, 0.5.. for at least, and ..0.7 for at most. A level can be negative, so the old -0.5 no longer means at most. Rounding: https://wiki.eveuniversity.org/System_security
  --agent-level <n>      Keep corporations with an agent of level n (1 to 5).
  --agent-division <name>  Keep corporations with an agent in this division, e.g. security or distribution.
  --agent-type <name>    Keep corporations with an agent of this type, e.g. basic, research, concord (the Agent suffix is optional).
  --agent-region <id|name>  Keep corporations with an agent standing in a system inside this region, by ID or name. Where the agents are, not where prices come from: that is --region.
  --agent-system <id|name>  Keep corporations with an agent standing in this system, by ID or name. Where an agent is, not which stations price: that is --system.
  --system <name|id>     Only use buy and sell orders at NPC stations in this system. Applies to --lpstores.
  --faction <name>       Keep corporations of this faction, e.g. caldari, amarr, gallente. Applies to --lpstores.
  All filters given must match the same agent.

Options:
  --region <id|name>     Market region used for pricing, by ID or name. Defaults to The Forge (10000002).
  --status               Print API call status every second while running. In the full-screen view, the default when no mode is given, the footer shows it instead.
  --expire               Ignore cached responses and call the API again.
  --cache-minutes <n>    Extend how long cached responses are reused, by n minutes past their Expires time. Default 0 for --example, and the number of corporations for --lpstores. Use 0 to turn it off.
  --cache-dir <path>     Where to keep ESI responses, instead of the platform cache directory. The static data does not move with it.
  --tasks <n>            Fetch and pricing tasks that run at once with --lpstores. Default 64. The ESI requests they make are capped separately, by --max-inflight.
  --max-inflight <n>     Maximum ESI requests in flight at once across the whole run. Default 4.
  --single-thread        Run every task on one thread, instead of on a worker thread per CPU core. File reads, DNS lookups, and the full-screen view still use tokio's blocking threads.
  --depth <n>            Offers listed per group with --lpstores. Corporations whose top n offers are the same are listed as one group. Default 10.
  --verbose              Print cache hits, misses, and writes, with reasons and file names, and the cache and data directories. In the full-screen view, the default when no mode is given, they go to the log, which names the two directories with or without this flag.
  --clear-cache          Delete the cached ESI responses, and the static data archive if one is on disk. The four extracted static data files stay. With no other mode this stops there rather than opening the full-screen view.

Examples:
  eve_lpstore
      Open the full-screen view over every corporation, asking for the filters first. This is the
      same as --tui, and prints the usage line when standard output is not a terminal.

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

  eve_lpstore --lpstores --tasks 16
      Run up to 16 fetch and pricing tasks at once.

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

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help") {
        print!("{}", HELP_TEXT);
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("{}", version_line());
        return;
    }

    // The runtime is chosen before it exists, so this flag is read here rather than by
    // `parse_options`, which only has to accept it.
    let single_thread = args.iter().any(|a| a == "--single-thread");
    runtime(single_thread).block_on(run(args));
}

/// The runtime every mode runs on. By default, tokio's multi-thread runtime with a worker per CPU
/// core, which is what `#[tokio::main]` builds; with `single_thread`, its current-thread runtime,
/// which runs every task on the thread that calls `block_on`. Either way the blocking pool is
/// there for file reads, DNS lookups, and the full-screen view's loop.
fn runtime(single_thread: bool) -> tokio::runtime::Runtime {
    let mut builder = if single_thread {
        tokio::runtime::Builder::new_current_thread()
    } else {
        tokio::runtime::Builder::new_multi_thread()
    };
    builder
        .enable_all()
        .build()
        .expect("the tokio runtime starts")
}

async fn run(args: Vec<String>) {
    let options = match parse_options(&args[1..]) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{}", message);
            eprintln!("Run {} --help to see the available arguments", args[0]);
            std::process::exit(1);
        }
    };

    // Only an explicit `--tui` is checked against the terminal. A bare run reaches `mode_of`, which
    // falls back to the usage line when there is nowhere to draw, so a pipe is not made to look like
    // a failure it did not ask for.
    let tui = args.iter().any(|a| a == "--tui");
    let on_a_terminal = std::io::stdout().is_terminal();
    if tui && args.iter().any(|a| a == "--example") {
        eprintln!("--tui cannot be combined with --example");
        std::process::exit(1);
    }
    if tui && !on_a_terminal {
        eprintln!("--tui needs an interactive terminal on stdout");
        std::process::exit(1);
    }

    // `parse_options` refuses a value that starts with `--`, so every argument equal to one of these
    // flags is that flag and not some option's value.
    let status = args.iter().any(|a| a == "--status");
    let clear_cache = args.iter().any(|a| a == "--clear-cache");
    let (expire, verbose) = (options.expire, options.verbose);
    let mode = mode_of(&args, clear_cache, on_a_terminal);

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
    // The view draws over stdout, so the paths it needs go to its log instead of the screen it is
    // about to take over. `tui::run` reports both roots there unconditionally.
    if verbose && mode != Mode::Tui {
        println!("Cache: {}", paths.cache.display());
        println!("Static data: {}", paths.data.display());
        warn_ignored_local_dirs();
    }
    match mode {
        Mode::Example => {
            let parsed = match options.parse() {
                Ok(parsed) => parsed,
                Err(errors) => exit_with_errors(errors),
            };
            if clear_cache {
                clear_cached_files(&paths.cache, &paths.data);
            }
            let cache_offset =
                std::time::Duration::from_secs(parsed.cache_minutes.unwrap_or(0) * 60);
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
        }
        Mode::Lpstores | Mode::Tui => {
            let tui = mode == Mode::Tui;
            run_lpstores(status, options, clear_cache, tui, paths).await;
        }
        Mode::ClearOnly => {
            if let Err(errors) = options.parse() {
                exit_with_errors(errors);
            }
            clear_cached_files(&paths.cache, &paths.data);
        }
        Mode::Usage => {
            if let Err(errors) = options.parse() {
                exit_with_errors(errors);
            }
            println!("{}", usage_line(&args[0]));
        }
    }
}

/// The arguments that take no value. Every one that does is a row of `report::Field`.
const FLAGS: &[&str] = &[
    "--example",
    "--lpstores",
    "--tui",
    "--status",
    "--expire",
    "--verbose",
    "--clear-cache",
    "--help",
    "--version",
    "--single-thread",
];

/// What `--version` prints, so a bug report can name the release it is against.
fn version_line() -> String {
    format!("eve_lpstore {}", env!("CARGO_PKG_VERSION"))
}

/// Reads the arguments after the program name into options, and refuses one it does not know.
///
/// A value flag needs its value: one at the end, or followed by an argument starting with `--`, is
/// refused rather than read as unset. A value with a single leading dash is still a value, since
/// `--security -0.5` is how a negative level is written. Given twice, the first value wins.
fn parse_options(args: &[String]) -> Result<LpStoreOptions, String> {
    let mut options = LpStoreOptions::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if let Some(field) = Field::from_flag(arg) {
            let value = match rest.next() {
                Some(value) if !value.starts_with("--") => value,
                _ => return Err(format!("{} needs a value", arg)),
            };
            options
                .value_mut(field)
                .get_or_insert_with(|| value.clone());
            continue;
        }
        match arg.as_str() {
            "--expire" => options.expire = true,
            "--verbose" => options.verbose = true,
            flag if FLAGS.contains(&flag) => {}
            _ => return Err(format!("Unknown argument '{}'", arg)),
        }
    }
    Ok(options)
}

/// What a run with no mode and no terminal prints: every flag, then what a bare run does instead.
fn usage_line(program: &str) -> String {
    let mut usage = format!(
        "Usage: {program} [--example|--lpstores|--tui] [--status] [--expire] [--verbose] \
         [--clear-cache] [--single-thread]"
    );
    for field in Field::ALL {
        let spec = field.spec();
        usage.push_str(&format!(" [{} {}]", spec.flag, spec.value_name));
    }
    usage.push_str(
        "\nWith no mode, opens the --tui view on a terminal; the modes above are the ways to ask \
         for something else.",
    );
    usage
}

/// What the arguments ask for, once whether stdout is a terminal is known.
///
/// A function rather than a test spread over `main`, because the choice decides four things at
/// once — what runs, whether the cache is cleared and the program exits, and whether the usage line
/// is printed — and each of those reads a different argument today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The general ESI and static data example.
    Example,
    /// The text report on standard output.
    Lpstores,
    /// The full-screen view. What no mode asks for, when there is a terminal to draw on.
    Tui,
    /// Only the `--clear-cache` action, then stop.
    ClearOnly,
    /// No mode, and nowhere to draw the view, so print how to use the program.
    Usage,
}

/// Pick the mode. Explicit modes win in the order the arguments already test them, so
/// `--example --lpstores` keeps meaning the example.
///
/// With no mode at all, `--clear-cache` stays a maintenance action that finishes when the deleting
/// is done, and anything else opens the view. That falls back to the usage line when stdout is not a
/// terminal, so a pipe and a CI job see help rather than a failure they did not ask for. Asking for
/// `--tui` by name with no terminal is a different case: the program cannot do what was asked, and
/// `main` reports that instead of quietly printing usage.
fn mode_of(args: &[String], clear_cache: bool, on_a_terminal: bool) -> Mode {
    if args.iter().any(|a| a == "--example") {
        return Mode::Example;
    }
    if args.iter().any(|a| a == "--lpstores") {
        return Mode::Lpstores;
    }
    if args.iter().any(|a| a == "--tui") {
        return Mode::Tui;
    }
    if clear_cache {
        Mode::ClearOnly
    } else if on_a_terminal {
        Mode::Tui
    } else {
        Mode::Usage
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

/// The static data on disk, or downloaded when it is not there. Says so before downloading: the
/// fetch is the whole SDE archive, which takes long enough that a silent terminal looks hung.
async fn load_static_data(data: &Path) -> Result<StaticData, StaticDataError> {
    if let Some(loaded) = StaticData::load_cached(data)? {
        return Ok(loaded);
    }
    println!(
        "Static data not found, downloading it to {}...",
        data.display()
    );
    StaticData::download(data).await
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
    let sdata = match load_static_data(&paths.data).await {
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
        Some(value) => match report::region_id(value, &sdata, Field::Region) {
            Ok(id) => id,
            Err(error) => {
                eprintln!("{}", error.message);
                std::process::exit(1);
            }
        },
        None => report::DEFAULT_REGION_ID,
    };
    println!("Region: {}", sdata.region_label(region_id));

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
    let sdata = match load_static_data(&paths.data).await {
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

    fn args(text: &str) -> Vec<String> {
        let mut args = vec!["eve_lpstore".to_string()];
        args.extend(text.split_whitespace().map(str::to_string));
        args
    }

    #[test]
    fn every_option_is_documented_and_read() {
        // Three ways a flag can be added and not work: missing from the help, missing from the
        // usage line, or not reaching its option. All three are checked against the option table,
        // so a new option cannot slip past. The help and the usage line also have to agree on how
        // the value is written.
        let usage = usage_line("eve_lpstore");
        for field in Field::ALL {
            let spec = field.spec();
            let written = format!("{} {}", spec.flag, spec.value_name);
            assert!(HELP_TEXT.contains(&written), "{written} is not in the help");
            assert!(
                usage.contains(&format!("[{written}]")),
                "{written} is not in the usage line"
            );
            let options = parse_options(&args(&format!("{} value", spec.flag))[1..])
                .unwrap_or_else(|error| panic!("{}: {error}", spec.flag));
            assert_eq!(
                options.value(field).as_deref(),
                Some("value"),
                "{}",
                spec.flag
            );
        }
    }

    #[test]
    fn single_thread_picks_the_current_thread_runtime() {
        use tokio::runtime::RuntimeFlavor;
        assert_eq!(
            runtime(true).handle().runtime_flavor(),
            RuntimeFlavor::CurrentThread
        );
        assert_eq!(
            runtime(false).handle().runtime_flavor(),
            RuntimeFlavor::MultiThread
        );
        assert!(HELP_TEXT.contains("  --single-thread  "));
        assert!(usage_line("eve_lpstore").contains("[--single-thread]"));
        assert!(parse_options(&args("--lpstores --single-thread")[1..]).is_ok());
    }

    #[test]
    fn the_version_is_the_packages() {
        assert_eq!(
            version_line(),
            concat!("eve_lpstore ", env!("CARGO_PKG_VERSION"))
        );
        assert!(HELP_TEXT.contains("--version"));
        assert!(parse_options(&args("--lpstores --version")[1..]).is_ok());
    }

    #[test]
    fn a_value_flag_without_its_value_is_refused() {
        for text in ["--depth", "--depth --tui", "--lpstore --region Domain"] {
            let error = parse_options(&args(text)[1..]).expect_err(text);
            assert!(error.ends_with(" needs a value"), "{text}: {error}");
        }
        assert_eq!(
            parse_options(&args("--lpstores --depth")[1..]),
            Err("--depth needs a value".to_string())
        );
    }

    #[test]
    fn a_value_with_one_leading_dash_is_still_a_value() {
        let options = parse_options(&args("--security -0.5..-0.1")[1..]).unwrap();
        assert_eq!(options.security.as_deref(), Some("-0.5..-0.1"));
    }

    #[test]
    fn the_old_concurrency_flag_is_refused() {
        // Renamed to --tasks without an alias, so a script that still passes it is told.
        assert_eq!(
            parse_options(&args("--lpstores --concurrency 8")[1..]),
            Err("Unknown argument '--concurrency'".to_string())
        );
        let options = parse_options(&args("--tasks 8")[1..]).unwrap();
        assert_eq!(options.tasks.as_deref(), Some("8"));
    }

    #[test]
    fn an_unknown_argument_is_named() {
        assert_eq!(
            parse_options(&args("--lpstores --colour")[1..]),
            Err("Unknown argument '--colour'".to_string())
        );
    }

    #[test]
    fn the_first_of_two_values_wins_and_the_switches_are_read() {
        let options =
            parse_options(&args("--region Domain --expire --region Heimatar --verbose")[1..])
                .unwrap();
        assert_eq!(options.region.as_deref(), Some("Domain"));
        assert!(options.expire && options.verbose);
        let options = parse_options(&args("--tui --status --clear-cache")[1..]).unwrap();
        assert_eq!(options, LpStoreOptions::default());
    }

    #[test]
    fn a_bare_run_opens_the_view_on_a_terminal_and_usage_otherwise() {
        assert_eq!(mode_of(&args(""), false, true), Mode::Tui);
        assert_eq!(mode_of(&args(""), false, false), Mode::Usage);
    }

    #[test]
    fn naming_the_view_keeps_its_own_meaning_on_either_output() {
        // `main` reports the no-terminal case as an error, because the program cannot do what was
        // asked. The mode function only says what was asked for, which is the same either way.
        assert_eq!(mode_of(&args("--tui"), false, true), Mode::Tui);
        assert_eq!(mode_of(&args("--tui"), false, false), Mode::Tui);
    }

    #[test]
    fn the_other_two_modes_still_select_themselves() {
        assert_eq!(mode_of(&args("--lpstores"), false, true), Mode::Lpstores);
        assert_eq!(mode_of(&args("--example"), false, true), Mode::Example);
        // Filters and options do not change the mode, with or without a terminal.
        assert_eq!(
            mode_of(&args("--lpstores --depth 5 --verbose"), true, true),
            Mode::Lpstores
        );
    }

    #[test]
    fn clearing_the_cache_alone_is_its_own_mode() {
        assert_eq!(mode_of(&args("--clear-cache"), true, true), Mode::ClearOnly);
        assert_eq!(
            mode_of(&args("--clear-cache"), true, false),
            Mode::ClearOnly
        );
        // With a mode named, clearing is something that run does on the way past it.
        assert_eq!(
            mode_of(&args("--clear-cache --lpstores"), true, true),
            Mode::Lpstores
        );
    }

    #[test]
    fn an_explicit_mode_wins_over_the_default() {
        // --example and --tui together is refused in main before this is reached, so the order here
        // only decides what a lone --example or --lpstores means.
        assert_eq!(mode_of(&args("--example"), false, false), Mode::Example);
        assert_eq!(mode_of(&args("--lpstores"), false, false), Mode::Lpstores);
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

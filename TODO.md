# TODO

## Open

### TUI with ratatui and crossterm

Add a `--tui` flag that opens an interactive view of `--lpstores`. Filters are edited in the UI, and offers are grouped the same way as the CLI.

Assumptions:
- The filter form covers the CLI filter flags, depth, and `--expire`. `--concurrency`, `--max-inflight`, and `--cache-minutes` stay command-line only, and the TUI still honors them when they are passed.
- Re-runs are usually fast because ESI responses come from the cache. Changing the system or narrowing the corp list fetches nothing new. A new region fetches new market orders.

#### Phase 1: Separate the pipeline from printing

CLI output stays the same, except that it prints every filter error, and lists corporations in ID order within each group when `--lpstore` is not given. This phase can ship on its own.

- [x] Before changing code, save a baseline `--lpstores` output. Set `--cache-minutes` high enough that both runs read the same cached responses.
- [x] Move filter parsing and validation into the library. Put the options in one struct and collect every error with its field name.
- [x] Move the pipeline into `src/report/`. It returns errors instead of calling `std::process::exit`, and it returns a `Report` with groups sorted best first.
  - [x] Break ties by the group's first corp ID. Today, ties depend on task completion order, so the output can change between runs.
- [x] Emit progress events on a `tokio::sync::mpsc` channel. Each event carries a run id.
- [x] Send the 7 cache `println!` calls in `src/esi/mod.rs` through a log sink set on `EsiClientBuilder`.
- [x] Move `format_number`, `format_isk`, and `group_digits` into `src/format.rs`.
- [x] Make the CLI print from `Report` and progress events. It reverses the group order to match today's output and reprints the same progress lines. Diff against the baseline, ignoring timing numbers.

#### Phase 2: Dependencies, `--tui`, and terminal setup

- [x] Add `ratatui = "0.30"` and `crossterm = "0.29"`. ratatui needs a recent Rust, and 0.30.2 declares `rust-version` 1.88.0, which is this project's floor since no other dependency declares one; the 1.86 written here first was wrong. Pin crossterm to 0.29 so only one copy is built. Cargo needs network access to fetch them.
- [x] Add `--tui` to `FLAGS`, `HELP_TEXT`, and the usage line. It implies `--lpstores` and is rejected with `--example`. It exits with an error if stdout is not a terminal. Inside the TUI, the footer shows the status instead of `--status`, and `--verbose` output goes to the log panel.
- [x] Put the TUI in `src/tui/`, declared from `main.rs`. Start it after static data loads and the arguments validate, so existing errors still print to a normal terminal. The header shows the CLI filter flags. Pre-filling the filter form from them waits for Phase 4.
- [x] Run the UI on a blocking thread with `spawn_blocking` around `ratatui::run`. It polls keys with `crossterm::event::poll`, drains progress with `try_recv`, and redraws every 250 ms. The pipeline stays on the async runtime. This fits `ratatui::run`, which takes a sync closure, and it avoids `EventStream` and `futures`.
- [x] Restore the terminal on quit, on Ctrl-C (raw mode delivers it as a key event), and on panic. ratatui's `init` already installs a panic hook that restores the terminal before the panic message prints, so no hook was added.

#### Phase 3: Results view

- [x] Keep app state in `src/tui/app.rs` as plain data with update functions (event in, state out), with no terminal I/O. Rendering goes in `src/tui/ui.rs`.
- [x] Progress view, shown until the first run finishes: gauges for fetch and pricing, plus recent log lines.
- [x] Results layout, using the CLI's grouping:
  - [x] Groups pane on the left: each group's top ISK per LP, offer count, and corp names, best first.
  - [x] Offers pane on the right: the selected group's top `--depth` offers. Columns are `#`, Item, ISK/LP, Spread, and Profit ±10%, with Imbal and Vol ±10% added once the pane reaches `WIDE_OFFERS` (73) columns. LP cost, reward, unit cost, units, and best ask are in the detail pane, because the table has no room for them beside the groups list.
  - [x] Detail pane: the group's corps with their agent systems, as the CLI prints them, plus the selected offer's breakdown, the within-10% volume on both sides of the book, the spread, the imbalance, and the profit.
- [x] Status bar: in flight, waiting, rate limit, errors, and cache hits, misses, and writes.
- [x] Log panel (`l`): cache lines under `--verbose`, plus fetch and pricing errors.
- [x] Keys: arrows or j/k to move, Tab to switch panes, PgUp/PgDn to scroll, `?` for help, and `q` to quit. Ctrl-C also quits. Quitting aborts the current run.
- [x] Show a "terminal too small" message below the minimum size, then 80x20. The height is 80x24 now, raised when the Filters page grew to twelve fields; see *Filter agents by where they stand*.

#### Phase 4: Filter editing

- [x] Filter form (`f`): text fields for corporations, region, security, agent level, division, type, faction, system, and depth, plus an `--expire` toggle. Each field shows its valid values. Tab and Shift+Tab move between fields, Enter applies, and Esc cancels.
- [x] While the form is open, keys go to the form, so typing `q` enters a letter.
- [x] Apply validates every field first. If any field fails, its error shows inline and nothing runs.
- [x] Apply on success cancels the running pipeline and starts a new run with a new run id. Results from the previous run stay visible, marked stale, until the new run finishes.
- [x] Each run builds its own `EsiClient`, because the default cache offset depends on the number of corps. The status counters therefore reset for each run.
- [x] Depth changes re-render only. The pipeline uses depth only when printing.
- [x] Name suggestions for corporations, systems, and regions. Systems need a new `StaticData` accessor, since system names aren't exposed yet. This can be cut from v1 to keep the first version smaller.

#### Phase 5: Tests and docs

- [x] Unit tests for state transitions: key sequences, progress events, stale run ids, and validation errors. 102 tests live in `src/tui/`: `app.rs` covers keys, progress, stale run ids, offer errors, the static data dialog, and the startup form's Esc rule, `form.rs` covers per-field and general validation errors, the pre-filled defaults, and the saved-file note, `mod.rs` covers run identity, the filter summary, the first-Enter repeat, the `opening` decision, and remembering an applied run, `qr.rs` covers the QR rendering, and `ui.rs` covers text fitting, the offer cells, the spread and imbalance formatting and colours, full-view renders, and both dialogs.
- [x] Render tests with `ratatui::backend::TestBackend` at 80x24 and 120x40. `src/tui/ui.rs` renders the donation overlay at 80x24 and 80x20, and the whole results view at 80x24, 120x40, the wide threshold, and every width from 80 to 160. Needs an empty `StaticData`, so it gained a `Default`, and `ClientStats` likewise; the `data/` cache is gitignored, so loading it in a test is not an option.
- [x] Update `HELP_TEXT` with `--tui` examples: the bare view with its keys, one corporation in another region with `--depth`, and `--verbose` going to the log panel.
- [x] Manual check in a real terminal: resize, filter apply during a run, quit, Ctrl-C, and a forced panic. Done by hand; the resize check is what the width sweep from 80 to 160 now covers in a test.

#### Phase 6: Later

- [x] Export the current view to CSV or JSON. `e` in the results view opens a prompt pre-filled from
  the region and filters; Enter writes the file, Esc cancels, and the extension chooses the shape, so
  an unknown one is refused beside the name rather than guessed. `src/export/mod.rs` renders both
  shapes from one tree, so a renamed field cannot end up in only one of them. It follows the view:
  groups best first, only the top `depth` offers each, no new ESI request. Numbers are plain rather
  than comma-grouped, missing figures are empty in CSV and `null` in JSON, ISK and percentages are
  rounded to two decimals, and there is no timestamp, so the same report always writes the same
  bytes. The required items and the offers that failed to price are JSON only, since a flat row has
  nowhere for them.

#### Phase 7: Filters first on startup

- [x] Extract the cache check from `load` as `StaticData::is_cached`. It was already there as four `exists()` calls on `data/`, and `data/` is gitignored, so a fresh clone takes the uncached path on first run.
- [x] When static data is missing under `--tui`, start the TUI with `StaticData::default()` and show a download/exit dialog as a new overlay, beside Log, Help, Filters, Donate, and Export. Download writes the four `data/` files and then the filters dialog opens; exit stops the application. Esc and `x` both exit, Enter and `d` both download.
- [x] Run the download on the async runtime, not the UI thread. `ui_loop` is sync inside `spawn_blocking` and `StaticData::load` is async, so it is spawned on `session.handle` with a oneshot for the result, the way `spawn_run` already does. While it runs the dialog reads "Downloading the EVE Online static data…" with only Ctrl+C offered, and every other key is swallowed: `download_and_cache` reads the whole response in one `bytes().await`, so there is nothing to report per byte without switching it to a streamed read.
- [x] On success, assign the loaded data to `session.sdata`, which every draw reads fresh, and open the filters dialog. On failure, keep the dialog open with the reason and offer to try again or exit, instead of the `eprintln!` and `exit(1)` in `main`. A retry calls `load(true)`, which replaces the files, so data that is present but unreadable is recoverable from the dialog rather than failing the same way again.
- [x] Move `resolve` and `run_info` to the Enter that starts the first run, so the cached and uncached paths take one route. Consequence, as predicted: an unresolvable command-line value, such as a region name that does not exist, now lands in the filter form through `set_errors` rather than printing to a normal terminal and exiting. Unknown flags are still rejected in `main` before the TUI starts, so the Phase 2 guarantee now covers parsing but not name resolution.
- [x] Open the filters dialog before the first run is spawned, so no ESI request goes out until Enter is pressed. Checked in a real pseudo-terminal: the cached view opens on the filters dialog and draws no fetch or pricing pane at all, and the uncached one opens on the download dialog without creating `data/` or `cache/`.
- [x] Pre-fill region and depth from their defaults when the command line did not set them, and leave the seven filter fields blank. Blank is what means "all" or "any" for corporations, security, agent level, division, type, faction, and system, and the hint text already says so; a pre-filled value would silently narrow the run. The region shows its name when the static data names it, and its id otherwise. `FilterForm::from_options_with_defaults` takes the two values rather than reading defaults itself, so the form still does not know where they come from.
- [x] Make the first Enter start a run even when nothing changed. `apply` compared against `app.applied` with `same_run`, and the startup dialog opens with the command-line options already in `applied`, so an unchanged Enter matched and only re-rendered, leaving the view with no run. The guard is `is_repeat(has_run, options, applied)`, extracted so the trap is a unit test rather than a comment.
- [x] Give `Session.current` a no-run state. It is `Option<Run>` now, so `Session::stats()` returns `ClientStats::default()` until the first run exists, and `apply` aborts the previous run only when there is one.
- [x] Esc in the startup filters dialog quits. There are no results to return to, and no ESI request should go out unless Enter was pressed. The rule is `stage == Stage::Idle`, which is also what the header and the progress pane use to say nothing has started. The dialog opened later with `f` keeps its current Esc-cancels behaviour.
- [x] In `--example` and `--lpstores`, print one line before downloading, then keep the current behaviour. They have no TUI to host a dialog. `announce_static_data_download` derives the directory from `regions_cache_path()` instead of repeating the `data` literal, and is silent when the cache is present.
- [x] `Stage::Idle` and the header. The first live render showed the header reading " | 0 corporations | Filters: none" before a run, so Idle prints "Not started | Filters: …" instead, and the progress pane leaves out the two gauges that would read "waiting" and gives their space to the log.
- [x] `--tui` in `HELP_TEXT` now says the view asks for the filters first and for the static data if it is missing, and the bare `--tui` example says Enter starts the run. Both described the old behaviour of fetching immediately.
- [x] Tests: 23 added, covering the download dialog offering download or exit and swallowing keys while fetching, a finished download opening the filters, a failed download keeping the reason, the idle header and progress view, the pre-filled form and its round trip, the first Enter not being a repeat, Esc quitting the startup dialog but cancelling later, the footer keys, and `opening` deciding the cached, uncached, and unreadable cases.
- [x] `opening` replaces the boolean that decided the start. The first version mapped "not cached" to no reason, which read back as success, so the uncached view opened on the filters dialog and never asked — found by running the binary in a pseudo-terminal with no `data/`, not by a unit test, since `run` needs a terminal. It now returns `Filters(StaticData)` or `Ask { reason }`, so the two cannot be confused.

#### Phase 7.1: Remember the filters

- [x] `src/saved_filters.rs` holds the nine fields the form carries, as its own struct rather than serde on `LpStoreOptions`, so `--expire` and the command-line-only fields cannot be written at all. `--expire` is a one-shot cache bypass, and remembering it would refetch every offer on every later session. `#[serde(default)]` lets a file from an older version, or one edited by hand, still load.
- [x] A successful apply in the view writes `filters.json`, and the next session reads it before showing the filters dialog. Both branches of `apply` reach the save, including a depth-only apply, because "applied" is what the user pressed. `--lpstores` runs never write it, so a script or CI job cannot quietly change what an interactive session starts with.
- [x] The command line wins field by field over the file. Every field of `LpStoreOptions` is `Option<String>`, so "not given" is distinguishable from "given empty", and `--tui --region Domain` keeps the saved corporation list while overriding only the region. The command-line-only settings come through untouched from `cli`.
- [x] `Loaded` is three cases, `Absent`, `Filters`, and `Unusable`, because "no file yet" is the normal state of a first run and must not be reported, while "there is a file and it is broken" must. Collapsing those into a boolean is the same mistake Phase 7 made in `opening`, and it is why that one is now an enum too.
- [x] A broken file falls back to the command line and the defaults, and says so on the form as well as in the log. The form note is not optional: the filters dialog owns the keyboard, `l` cannot open the log over it, and Esc quits at that point, so a log line alone would never be read. `FilterForm::set_note` shows it where a field error would be, and a later `set_errors` replaces it.
- [x] The note is only set on the startup dialog. Pressing `f` mid-session still pre-fills from the current run's options, because overwriting that base with an older session's filters would be surprising.
- [x] A saved value that is only whitespace is treated as absent, so a hand-edited empty field reads the way an emptied form field does instead of as a filter that matches nothing. An empty value given on the command line keeps its current meaning, so the flags behave exactly as they did.
- [x] Clearing every field and applying is the reset. The file holds the empties, so there is no `--no-remember` flag to design and no second source of truth to keep in step.
- [x] `filters.json` sits at the top level, a sibling of `cache/` and `data/`, rather than inside either: it is a user file, not derived data, and it must survive a static-data reset. Added to `.gitignore`.
- [x] `remember` takes its path as an argument, the way `write_export` does, so the write is covered by unit tests without a terminal and without touching the project directory. A failed save is only a log line: the run is already going.
- [x] HELP_TEXT says the view remembers, and a second `--tui` example shows the per-field precedence, which is the part a reader would otherwise guess wrong.
- [x] Tests: 25 added, covering the round trip, a missing file, five kinds of damaged file, the fields that are never written, per-field precedence, the command-line-only settings surviving a merge, blank handling on both sides, the reset by clearing, a file with a missing field and one with an extra field, `set_note` and its replacement by a validation failure, the note rendering on the startup form, and a remembered run being written and read back.
- [x] One behaviour that is not a bug: a file containing `[]` loads as every field absent rather than being rejected, because serde's derived deserializer accepts the sequence form. That is the same run the file being absent would give, so there is nothing to report. **Superseded** by the option table in the 2026-10-12 refactoring: the sequence form also read `["a", "b"]` positionally into the first two fields, and the file is now read as an object only, so any array is reported as unusable.

#### Phase 7.2: Name the market fields for what they filter

The two fields that choose where prices come from were called `Region` and `System`, sitting apart
from each other and reading like more corporation attributes. This is the dialog only: `--region`
and `--system` keep their names, `filters.json` keeps its keys, and the export keeps its columns.

- [x] Labels are `Market region` and `Market system`, first word capitalised only, matching `Agent level`, `Agent type`, and `Expire cache`. Both are 13 characters, inside the 14-column label width, so grouping cost no layout change.
- [x] The form is ordered in three groups: which corporations and agents, then the two market fields side by side, then how results are shown. `Expire` stays last so the space toggle is always the final stop, and Tab follows the same order because it walks `ALL`.
- [x] Each hint is split between the row and the note under it. A row is one line tall, so `hint_line` shows just the purpose there; the focused field's note below carries purpose then syntax, which is what the two spare rows at 80x20 are for. `wrap_text` now honours a line break in the source, so a hint can ask for two lines instead of leaving the split to the width.
- [x] The purpose wording is the point of the change, and a mismatch is worth preventing: `station_ids_in_system` returns the stations of whatever system was typed, `in_locations` then filters the priced region's orders against them, and a system from another region leaves every lookup failing with 404 `no sell orders found … in region`.
- [x] Two tests that walked to a field by `next_field` now name it through `focus` instead, so they do not move when the fields are regrouped. The two that still step, `space_toggles_the_expire_option` and `focus_wraps_around_the_fields`, assert on `Expire` and `Corporations`, which stay last and first, so they are unaffected.
- [x] Tests: 6 added. The labels and their width, adjacency and group order, wrapping on a source break, the inline hint being one line and inside the detail column, all ten labels rendering at the minimum terminal size, and both hint lines showing under a focused market field at 120x40.
- [x] Closed as **unverified by choice**, not as done. The note under a focused market field and the clearing of `Market system` when `Market region` is changed and applied are covered by `TestBackend` renders and direct calls, never by a real key press: three pseudo-terminal attempts each failed to show lines those tests prove exist, so the harness is the suspect. The constructed-state coverage was accepted as enough, which leaves both behaviours unobserved where they actually happen.

##### Remembering and validating the pair

- [x] On apply, `Market system` is cleared when `Market region` moved, by `region_moved`, which compares resolved region ids so `Domain` and `10000043` count as the same region. The cleared field is dropped before the run is built, and the options are resolved a second time, because the stations come from that result: leaving the first one in place would price against the old system's stations while the form showed an empty box. The clearing is logged, since a value the user typed disappeared. The second resolve's error branch is unreachable, clearing a filter cannot add one, and is noted as such rather than pretending otherwise.
- [x] `Market system` is left alone when the region did not move, and on the first Enter, where there is no previous region to compare against. Both are assertions in their own tests.
- [x] The rule is unit-tested, not live-tested, and the difference matters here. `clear_system_on_region_move` is a separate function, and four tests cover the clear, the re-resolve that drops the stations, the surviving system when the region did not move, and the first Enter. What is *not* confirmed is the wiring behind a real key press: three attempts to drive it through a pseudo-terminal failed to show the clearing message, but each also failed to show lines the unit tests prove exist, so the harness is the suspect rather than the program. Recorded with the live-check item below rather than claimed as verified.
- [x] The system-to-region mapping was settled against the downloaded archive rather than guessed. `mapSolarSystems.jsonl` keys each row by `_key`, carries `regionID`, and holds `name` per language, so `.en` is what the existing reader already takes. 8,490 systems, and Jita resolves to 10000002 as it should. It joins `AgentCache.systems` as a fourth element rather than becoming a sixth cache file, since `extract_agent_cache` already parses that file for the name and the security status.
- [x] A cache from an older build is detected by a `version` field in the cached agent data, now 2. `is_cached` reads that file instead of only checking existence, because the old file *does* exist and is the wrong shape: its `systems` rows have three elements, so deserialising would fail inside `load` rather than prompting a download. Checked end to end in a temporary home seeded with the stale file: it printed the download notice, rewrote the cache at version 2 with four-element rows, and priced a corporation. The extra read costs about 21 ms over a 2.2 MB file, measured.
- [x] `system_outside_region` reports a system not in the priced region through `set_errors` on the System field, naming the region, and runs after the clearing above so a value already removed is not also complained about. It takes the system's region as an `Option`, so a cache with no mapping passes rather than rejecting every system, which is something the user did nothing wrong about.
- [x] The rule is the dialog's alone, and that was verified rather than asserted: `--lpstores --system Jita --region Domain` still runs and prints `Pricing from 18 NPC stations in Jita`, because `resolve` is shared with the command line and was not touched. `SystemSummary` gained the resolved `system_id`, which is what lets the dialog check the mapping without resolving the name twice.

#### Phase 8: Cache and data in platform directories

Store the ESI response cache, the static data, and the remembered filters in the platform's standard locations instead of `cache/`, `data/`, and `filters.json` in the working directory. They were relative, so running the binary from anywhere else silently re-downloaded the static data and re-fetched every ESI response.

- [x] `directories = "6"` added. Rust std cannot do this: `std::env::home_dir` has been deprecated since 1.29 with "consider using a crate from crates.io instead", std has no XDG or Known Folder support, and on Windows the correct source is the Known Folder API rather than `%APPDATA%`, which misses OneDrive redirection and UWP. Version 6.0.0 is MIT or Apache-2.0 with one runtime dependency, `dirs-sys`.
- [x] `src/paths.rs` resolves the roots once from `ProjectDirs::from("", "", "eve_lpstore")` into a `Paths { cache, data, config }`. An empty qualifier and organization give `~/.cache/eve_lpstore`, `~/.local/share/eve_lpstore`, and `~/.config/eve_lpstore` on Linux, and `%LOCALAPPDATA%\eve_lpstore\{cache,data}` plus `%APPDATA%\eve_lpstore\config` on Windows. Changing those strings later orphans the data a second time, so they are named as constants here.
- [x] The ESI cache is `cache_dir()`, the static data is `data_local_dir()`, and `filters.json` is `config_dir()`. The first two are Local rather than Roaming on Windows, which keeps regenerable data off a domain profile. The plan named two roots; a third was needed, because `filters.json` is a user file and putting it in either of the other two would have it deleted by a cache reset or a data reset.
- [x] `Paths::discover()` returns `None` when the operating system reports no home directory, and `main` refuses to start with a message naming `HOME` and `USERPROFILE` rather than falling back to the working directory. Not observable on this machine: with `HOME` unset *and* with `HOME` empty, `dirs-sys` falls back to the password database and returns `/home/user`, which is the documented behaviour, so the refusal branch has no test.
- [x] All five hardcoded sites replaced: `CACHE_DIR` and the four path accessors in `src/static_data/mod.rs` (now taking the data root, with a `files()` helper returning all four), the literal in `clear_cache_dir`, the two `.cache_dir("cache")` calls, and `FILE_NAME` in `src/saved_filters.rs`. `announce_static_data_download` and the static data dialog now say the real root instead of `data`.
- [x] `EsiConfig.cache_dir` is `Option<PathBuf>` and the builder takes `impl Into<PathBuf>`, so the spaces and backslashes of a Windows location survive. `StaticData::load` and `is_cached` take the data root, and `Resolved::client` takes the cache root, which are signature changes beyond the plan's list; `LpStoreOptions` gained `cache_dir` beside `cache_minutes`, `concurrency`, and `max_inflight`, so the form still does not carry it.
- [x] `--cache-dir <path>` added to `VALUE_FLAGS`, `HELP_TEXT`, and the usage line, overriding the ESI cache root only. Checked live: with the flag the cache went to `/tmp/opencode/altcache` while the static data stayed at `~/.local/share/eve_lpstore`.
- [x] `--clear-cache` deletes files rather than the tree: `*.json` and the `.*.tmp` orphans in the cache root, which is the `--cache-dir` value when given, plus the archive in the data root. The directory itself and the four extracted static data files survive, so the next run does not re-fetch the archive. Three tests cover it, including that a file the program did not write is left alone and that two absent roots are not an error.
- [x] The `.*.tmp` pattern is needed because the atomic write names its temporary file `.{entry}_{id}_{expiry}.json.{pid}.tmp`, which a json-only rule misses after a crash mid-write. A recursive walk is not required: the cache is flat, verified at 4,584 entries and no subdirectories.
- [x] The archive clause is defensive and matches nothing today. `download_and_cache` reads the response with one `bytes().await` and hands it to `zip::ZipArchive` in memory, so no archive is ever written to disk. It is now named by `SDE_ARCHIVE_NAME` rather than an inline literal, costs one `remove_file` whose `NotFound` is ignored, and is covered by a test that plants the file.
- [x] `--clear-cache` reports a count and an absolute path, `Cleared {n} cached responses from {abs}`, because the old wording claimed a directory was cleared and had a branch for it not existing; neither is true now that the directory survives.
- [x] Nothing migrates the old `cache/` and `data/`. Under `--verbose` the program notes that they are no longer used and can be deleted, which is the only place the change is visible, since the abandoned files are otherwise just dead weight. Confirmed live: `Note: cache and data in the working directory is no longer used`.
- [x] `.gitignore` keeps `/data` and `/cache` and drops `/filters.json`. The two directories are kept deliberately, against the plan: nothing writes them, but existing checkouts hold 122 MB and 13 MB of leftovers, and an entry that stops those being committed is still worth having. `filters.json` is gone because nothing writes it at the top level any more. Both directories are now labelled as leftovers in the file.
- [x] MSRV checked with `cargo metadata`: `directories` 6.0.0, `dirs-sys` 0.5.0, and `option-ext` 0.2.0 all declare **no** `rust-version`, so they set no floor and the crate is on the current toolchain. The floor is elsewhere: `ratatui` 0.30.2 declares `1.88.0`, not the 1.86 this file claimed, which is corrected on the Phase 2 item. `Cargo.toml` still declares no `rust-version` of its own.
- [x] The resolved roots print under `--verbose`, and go into the TUI log always, since a log is the only place a TUI user can read them: on stdout a line before the terminal is taken over scrolls away. Confirmed live, both lines, with the override applied.
- [x] Tests: 5 in `src/paths.rs` for absolute roots, neither being the working directory, cache and data differing, the override moving only the cache, and a blank override changing nothing; 3 for `--clear-cache`; the saved-filters path test now asserts the file sits in the directory it is given; and the dialog test asserts it names the root it was told about rather than assuming `data/`. The existing `esi` cache test still passes, since it already sets its own directory.
- [x] Two consequences recorded, since they are not visible in the code. The files in the working directory are now ignored, so the next run fetches the archive again, which happened during verification into `~/.local/share/eve_lpstore`. And a process started before the security syntax change, such as one still running `--security =0.5`, keeps working only because it holds the old code; restarting it needs `--security 0.5`.
- [x] A `filters.json` left in the working directory by a Phase 7.1 build was orphaned when the file moved to `~/.config/eve_lpstore/filters.json`. One existed in this checkout, written 2026-10-10 11:11 with `lpstore: The Scope` and the pre-filled region and depth. It was deleted rather than moved or migrated, so those remembered filters are gone and the next session starts from the command line and the defaults. Nothing reads or writes the top-level path, which is why `.gitignore` no longer lists it.
- [x] The seven files in `data/` that nothing read are covered by the download path writing only four. `load_from_disk` reads four files and only those four have filename constants. This checkout's `data/` was created by a run and holds only the four, so there was nothing to remove here; the seven existed only in the predecessor checkout, which this code never wrote them into either.

#### Phase 9: The TUI as the default mode

A bare `eve_lpstore` prints the usage line. It should open the view instead, with `--tui` kept as an
accepted flag that says what now happens anyway.

- [x] A bare `eve_lpstore` starts the view, and `--tui` stays accepted and behaves identically, so a script that passes it is unaffected. `--help`, `--example`, and `--lpstores` each still select themselves. Checked in a pseudo-terminal: a bare run drew the filters dialog with `Market region` and `Market system` present and no usage line, and `--lpstores` still ran as text on a terminal.
- [x] The decision is now one function, `mode_of(args, clear_cache, on_a_terminal)`, called once before the paths are resolved, returning `Example`, `Lpstores`, `Tui`, `ClearOnly`, or `Usage`. It replaced the boolean at `:143`, the terminal test at `:149`, the `else if tui ||` branch at `:223`, and the `else` at `:229` that mixed clearing with the usage print. `main` keeps the two `--tui` errors, since refusing a mode the user named is not the same decision as choosing one they did not.
- [x] No mode and no terminal on stdout prints the usage line, so a pipe and a CI job see help and exit 0. Verified side by side: a bare piped run printed the usage line, and `--tui` piped printed `--tui needs an interactive terminal on stdout`.
- [x] `--clear-cache` with no mode clears and exits, and does not open the view. Verified in a pseudo-terminal with a populated cache: it reported `Cleared 544 cached responses from ...`, exited on its own, and drew no dialog.
- [x] `--tui --example` is still refused before the mode is picked. `mode_of` never sees the pair, so the order among explicit modes only matters for `--example --lpstores`, which keeps meaning the example as it always did.
- [x] `HELP_TEXT` leads with `Usage: eve_lpstore [MODE] [FILTERS] [OPTIONS]`, `--tui` says it is the default and what happens with no terminal, `--example` and `--lpstores` read as the ways to ask otherwise, `--status` and `--verbose` now describe the default view rather than an opt-in, `--clear-cache` says it stops there with no other mode, and a bare run is the first example. The runtime usage line gained a second line saying the same, because the first draft of it closed the `--region` bracket around the note.
- [x] The `--verbose` path lines were the loose end the todo flagged, and the fix is a one-line condition rather than a move: with `mode != Mode::Tui` they print to stdout, and in the view `eve_lpstore::tui::run` already logs both roots unconditionally, so they are reported in both cases and never on the screen the view is about to take over.
- [x] Tests: 5 added for `mode_of`, covering a bare run with and without a terminal, `--tui` with and without one, the other two modes, filters that do not change the mode, `--clear-cache` alone and beside `--lpstores`, and `--example` alone. `src/main.rs` had 3 tests before, all for `--clear-cache`, and none for argument handling.

#### Phase 10: Where a mission can actually take you

The details pane lists each corporation's agent systems, but a mission from an agent in one of those
systems does not necessarily run there, so the listed security is not the security you will be flying
in. For each listed system, list the possible destination systems with the probability of a mission
appearing in each, grouped by rounded security rating.

Scope for this iteration: **security-division missions from basic agents, excluding burners**. Measured
against the cached agent data that is 2,929 of 9,057 agent rows, 152 of the 178 corporations, levels
1 through 5 at 856, 861, 673, 502 and 37 rows, across 1,700 systems that host any cached agent.

Scope of the model, as decided: **a destination is reachable through jump gates only.** That settles
the earlier open question as a scoping choice rather than a discovered fact, and it carries an accepted
limitation: if the game also routes missions over links that are not gates, those cases are outside what
this tool models, and the numbers it shows are then incomplete rather than wrong. Say which it is in
any text the view shows, so a user is not told a set is complete when it is defined as gate-only.

Model for regular missions, as given: **a level 1 to 5 mission from a basic security agent lands
inside the constellation the agent sits in**, and every system in that constellation is an equally
likely destination. Probabilities are then a count of systems aggregated into rounded security bands.
Worked example, verified against `displayed_security`: a constellation of four systems with true
security 0.49, 0.5, 0.51 and 0.6 rounds to 0.5, 0.5, 0.5 and 0.6, so band 0.5 holds three systems and
shows 0.75, and band 0.6 holds one and shows 0.25. That is uniform over systems, not over bands, so a
band with three systems is three times as likely as one with a single system.

Burner destination rule, supplied for the deferred model: a Level 4 agent whose division is Security
and type is Basic can offer burners, whose destination may be in the agent's own constellation or any
system in a neighbouring constellation. This is a different pool from regular missions, not a second
variant of the same uniform draw.

The same weights apply for a second grouping by **distance from the agent system in jumps**: 0 jumps is
the agent's own system, 1 jump the systems one gate away, and so on. Both groupings are marginals of one
uniform distribution over the constellation's systems, so a system contributes its `1/n` to whichever
band and whichever distance it belongs to. The distance in the details pane is unambiguous per row,
because each listed system is itself an agent system and the counting starts there.

Worked through the four-system example, which shows why the two groupings behave differently. The
marginal **by the systems' own security** is `0.75 / 0.25` in every case, because it counts systems per
band and ignores the graph. The grouping the pane leads with is by the security flown on the route, and
the graph does affect that: a route starts at the agent system, so when the start is the lowest of the
four the whole example groups as `1.00` at the start's band, and it splits only from a start that is
not the lowest. The distance grouping depends on the graph and on where the agent sits too, and these
are the possibilities for one constellation:

| topology and agent system | 0 jumps | 1 jump | 2 jumps | 3 jumps |
| --- | --- | --- | --- | --- |
| chain A-B-C-D, agent at 0.49 | 0.25 | 0.25 | 0.25 | 0.25 |
| chain A-B-C-D, agent at 0.51 | 0.25 | 0.50 | 0.25 | - |
| star, hub 0.49 | 0.25 | 0.75 | - | - |
| star, hub 0.60 | 0.25 | 0.75 | - | - |

Two readings of the same four systems differ as much as 0.25 against 0.75 at one jump, so the distance
table is only as good as the gate topology behind it, which the data does not have yet. The security
table can be built today from `constellationID` plus the cached security values, the distance table
needs the gate edges as well.



What the data supports, after one assumed gap closed and one question opened:

- [x] Constellation membership is loaded. `SdeSolarSystem` keeps `constellationID` beside the id, name,
      true security and region it already kept, so `AgentCache.systems` is a five-element tuple and
      `constellation_of_system` answers without a second source. Measured on a fresh download: 8,490
      systems across 1,184 constellation ids.
- [x] Gate edges are in the archive and simply not read. `mapStargates.jsonl` and
      `mapConstellations.jsonl` are among the archive's 102 files, confirmed by reading the zip's
      central directory with a range request rather than refetching 99 MB. `mapStargates` is the
      connections table the route guide names, so the graph is available offline from the download the
      tool already performs. This corrects an earlier claim here that the archive holds no jumps file:
      that scan filtered names on containing `system` or `region`, and `mapStargates` matches neither,
      so the absence was an artifact of the filter rather than of the data.
- [x] Adjacency is loaded and cached. `mapStargates.jsonl` is parsed into `AgentCache.gates`, 13,978
      records measured on a fresh download, and `StaticData::neighbour_map` turns them into a sorted,
      deduplicated map carrying both directions, which held with zero asymmetric edges.
      `AGENT_CACHE_VERSION` went to 3, so the version 2 cache an existing install holds now reads as
      uncached and asks before refetching. Verified end to end against the real archive with `HOME`
      pointed at a temporary directory: Jita's row reads `[30000142, "Jita", 0.945913, 10000002,
      20000020]` and its seven gates include the four that stay inside Kimotoro.
- [ ] Under uniform weighting, an incomplete membership list skews every number rather than merely
      omitting a row. A constellation missing one of its real systems has too small a denominator, so
      the remaining probabilities are inflated and a band can shift: if a four-system constellation is
      really five, the worked example reads 0.6 and 0.2 instead of 0.75 and 0.25. Membership is now
      cheap to check properly, since `mapConstellations.jsonl` is in the archive and
      `GET /universe/constellations/{id}` answers the same question for a spot check, so a few
      constellations are worth comparing against the game before any table is trusted. What is not
      settled is whether mission destinations sit inside that set at all, below.
- [x] A destination **is** reached as a solar system, so counting gates between systems and grouping
      systems by band is the right unit. Documented, not measured: EVE Info's mission overview
      (`eveinfo.com/missions`, fetched 2026-10-10) says an Encounter mission "will always create a
      mission space when the mission is accepted", and defines a mission space as "a region of space
      in some solar system", with the journal bookmark naming only that solar system. The site is
      therefore inside a system and does not change the route to it. Mining missions are described the
      same way; Courier missions end at a station, which is also in a system.
- [x] The reachability worry that made the separate-system reading plausible is settled the other way:
      a constellation-bound destination set never contains a system you cannot reach by gate. Measured
      from the archive, of 8,490 systems 5,268 have at least one stargate and 3,222 have none: 2,604 in
      the 310xxxxx range, which is wormhole space and expected to have no gates, 400 in the 320xxxxx and
      340xxxxx ranges, and 217 in the 300xxxxx range. No constellation mixes gated and gate-less
      systems, the 418 all-gate-less constellations being wholly separate, so there is no pocket system
      sitting in Kimotoro beside Jita, which is the shape a `Jita Space` destination would need. Two
      sources agree no such system exists: the cached `mapSolarSystems` rows, and ESI's
      `POST /universe/ids`, which resolves the name only as a character. The claim that the data
      "excludes space systems" stays withdrawn, since 14 of the 15 probed names may not be real.
- [x] **Scope is security missions, which excludes courier and mining agents.** Stated as a correction
      while this work was in progress: the constellation bound quoted below comes from EVE Info's
      *courier* section, and courier work is a different agent division, so it was never evidence about
      the missions this tool models. The pane now lists Security division agents only, and the line it
      claimed about levels 2 and up was removed together with that evidence.
- [ ] What bounds a security (Encounter) mission's destination above level 1 is **undocumented by
      anything found so far.** The sentences that do exist — level 1 "will keep you within the agent's
      constellation", levels 2 and 3 "will possibly send you to a neighboring constellation", level 4
      couriers "always" — belong to Distribution division agents and are recorded here only to explain
      why they were set aside, as are EVE-Survival's jump ranges (0 to 5 for levels 1 and 2, 0 to 12 at
      level 3, 1 to 12 for level 4 couriers), which are courier figures too. Nothing located states the
      reach of an Encounter mission at any level, so the constellation bound is this project's own
      assumption at every level, and the pane's first line says it is a model.
- [ ] Level 5 carries a constraint the uniform model cannot express, and which reading applies is not
      settled. The E-Uni mission page says level 5 missions "are exclusively located in Low Security
      space", and an archived forum thread agrees from the other direction, reporting level 5 sites in a
      mostly-highsec constellation concentrating in its few lowsec systems. Read as a claim about the
      *agent*, it contradicts the cached archive: of its 84 level 5 Security agents, 66 sit in systems
      whose true security is below 0.5 and 18 at 0.5 or above (by displayed band, 65 and 19). Read as a
      claim about the *site*, it is unchecked by this tool, since sites are in no public data. Either
      way the uniform model does not capture it, because it lets a mixed constellation's 0.5 systems be
      destinations at every level.
- [x] The scope is applied where the data allows it: `StaticData::security_agent_systems` keeps the
      corporation's agents in the archive's `Security` division, looked up by internal name rather than
      a hard-coded id (24 today), and a filter naming another division answers empty instead of quietly
      widening what is modelled. On the real archive this narrows listings — X-Sense from 27 agent
      systems to 12, Ammatar Consulate from 10 to 8, Guristas from 8 to 7 — and of the 178 corporations
      cached, 20 have no Security agent at all, which reads as "no security agents match the filters".
      A run filtered to Distribution gets "Distribution agents are outside this model: security
      missions only", with the name cut to fit the pane because the longest division name in the
      archive is 28 characters.
- [x] The bands are coloured. Ratatui's palette is ANSI and has **no `Purple`**, only `Magenta` and
      `LightMagenta`, so the asked-for purple is `LightMagenta` — the shade that reads as purple on the
      usual terminal themes — and not `Magenta`, which the offers table already spends on an inverted
      spread. The buckets are as given: 0 and below `LightMagenta`, 1 to 4 `Red`, 5 `Yellow`, 6 to 8
      `Green`, 9 and 10 `Cyan`, tested at every boundary including `-1` and `10`.
- [x] Colouring reads the printed band label rather than converting a security again, which is what
      keeps a system's own true security plain while the groups beside it are coloured: `(0.18)` is two
      decimals and stays in the line's style, `0.4/83%`, `route 0.4` and `own 0.5` are coloured, and
      `0J/50%` is not a band so it is left alone — the request named bands only, so the hop tokens are
      uncoloured by that reading rather than by oversight. A test asserts colouring loses no character.
The requested shape, kept as the spec the tests are written against:

      ```
      Corporations:
      Material Acquisition (1000097)
        Babirmoult (0.18) 0.1/50%, 0.2/50%  0J/50%, 1J/50%
      Astral Mining Inc. (1000098)
        Arraron (0.18) 0.1/50%, 0.2/50%  0J/50%, 1J/50%
        Annancale (0.09) 0.1/50%, 0.2/50%  0J/50%, 1J/50%
      ```

      Measured against the cache before building it: all three system names and both corporation ids
      are real, and the securities match the archive to the printed two decimals (Babirmoult 0.1788,
      Annancale 0.0941), so only the probabilities were invented. Both corporations have Security
      agents, so the scope applied today keeps them in: 29 agents in 16 systems and 26 in 14. Across
      the 158 corporations with any Security agent the systems per corporation run 1 to 53, median 11,
      and 81 of them have more than the 10 that `corp_line` used to cut at. Settled: **capped at the
      same `MAX_SYSTEMS_SHOWN` the CLI report cuts at**, with the rest named by count on a line that
      points at the pane that shows them all — `  +43 more systems, press m for all of them`. Chosen
      over listing them all because a 53-system corporation pushed the offer breakdown 107 rows down at
      80 columns (each summary wraps to two rows there), and over reordering the pane because the
      economics of the selected offer are what the pane is for. Data order is kept so the visible ten
      are the same ten `corp_line` prints, and exactly-at-cap prints no count line, because nothing is
      withheld at ten. Verified against the archive: the 53-system corporation shows ten rows plus the
      count line, and the `m` pane really lists all 53 systems and their 317 destination rows.
      A summary line measured 65 characters at the median and 126 at the longest, against the 78
      columns a border leaves at the minimum width, so both panes wrap rather than clip; the indent is
      kept on every wrapped row, because a continuation at column 0 reads as a new system.

- [x] Both views stay. The decision recorded above was "a pane of its own, opened by `m`", and this
      request adds the details pane beside it rather than reversing that decision. Chosen with the cost
      named: the same numbers now appear in two places, and the two say different things — the `m` pane
      carries the per-destination lines and the two model notes, the details pane carries neither. So
      the details summary is unlabelled, and the only place the tool states that this is a model is
      behind `m`. Worth revisiting if a user ever reads the details lines as measured fact.
- [x] Two bugs the real render caught, neither visible from the tests that existed. The destinations
      pane had inherited the log's bottom-first window while being given the details pane's key
      directions, so it **opened showing the tail of the list and Up moved down the table**; the window
      is now `pane_window`, which starts at the top and is bounded by the rows on screen, tested at
      scroll 0, past the end, and with a list shorter than the pane. And wrapping dropped the leading
      indent, so a summary's second row began at column 0 — exactly where a new system starts.
      `wrap_text` now takes the indent off, wraps into what is left of the width, and puts it back on
      every row. Observed on the rendered summary rows, which sat flush left at column 0 before the
      fix. The offer breakdown's item lines go through the same helper and so had the same defect —
      inference from the shared function, not something rendered and seen. A clipped-but-aligned line was the alternative, and the pane's own
      minimum width makes the degenerate case unreachable, guarded so it stays that way.
- [ ] The gateless-pocket and split-constellation cases, with the options each one leaves open, are
      measured and recorded in Phase 11 rather than decided here. One correction against what this
      bullet first said: widening the graph to the archive's wormhole links is **not** an option — the
      current SDE has no wormhole-link table at all, and ESI's router refuses pocket routes.
- [ ] There is no public way to measure any of this. Probed on 2026-10-10:
      `GET /characters/{id}/missions/` and `GET /characters/{id}/missionlog/` both return 404, while
      `GET /characters/{id}/assets/` returns 401 for the same unauthenticated request. The 401 is the
      control: an authenticated route that exists answers 401, so these two do not exist. A destination
      distribution could only come from a character's own mission journal inside the game client, which
      is out of scope for a tool that holds no credentials.

Two ESI endpoints are verified as useful here: `POST /route/{origin}/{destination}` with a JSON body
carrying `preference` and `security_penalty` plus an `X-Compatibility-Date` header returns
`{"route": [system ids]}`, measured as 7 systems and 6 jumps for Jita to Amamalka, so it is an
independent oracle for counts computed from `mapStargates`; `GET /universe/constellations/{id}`
returns a constellation's member systems, measured as 7 for Kimotoro, which holds Jita. Neither
`/universe/systems` nor `/universe/stargates` takes a POST, so a full ESI crawl would cost a request
per system and per gate; the graph should come from the archive instead.


Sources named for the research on how many jumps, none of it consulted yet: the EVE University wiki at
`https://wiki.eveuniversity.org/`, whose mission and system-security pages are the likeliest to state
where complexes spawn and which rounding the wiki already documents; the official forums at
`https://forums.eveonline.com/`, where long-running theory threads often carry the measurements; and
`https://www.reddit.com/r/Eve/`, which tends to hold the practical player data and any tooling that
already records offered missions. Weigh any finding against the other two, prefer a source that states
when it was written, and treat a source describing non-gate routing as evidence about the game rather
than a reason to reopen the gate-only scope without deciding to.

What remains open is now one display decision plus the data gaps listed above; the earlier questions
about what a probability means and whether the level matters were settled by the uniform-weighting
model rather than by a dialog, so they are not re-asked:

The uniform-weighting choice settles the question that was open about what a probability means, and it
removes the reason to fold in the agent level: levels 1 to 5 share one destination set, so one table
per listed system describes the scoped case. Two things follow from the model rather than being
decisions, and belong to it: the share of missions that are burner missions is still level dependent,
since only level 4 offers them, and that is deferred with burners; and the weighting is a stated
assumption about the game, not a measurement, so the view should present it as the model it is.
- [x] It appears in a pane of its own, opened by `m` for the selected group: one summary line per
      agent system with its band groups and hop groups, then one line per destination. Probabilities
      are whole percents, since the counts are single digits and a decimal would suggest a precision
      the model does not have. Up and Down scroll it in the details-pane direction, not the log's
      inverted one, and `Esc` closes while `q` still quits. Two italic lines sit above the body so the
      disclaimer cannot be scrolled away: that these are a model in which security missions stay in the
      constellation, and that a band is the lowest security on the safest route rather than the system's
      own.
- [x] Grouping uses the same rounding the security filter already applies, so a band on screen is a
      band a user can type: `security_band` returns tenths through `displayed_security` rather than
      rounding a second time, and the negative and zero cases the filter accepts are reachable here
      too.
- [x] The model itself exists as `StaticData::mission_spread(agent_system)`, returning a
      `DestinationSpread` with the band grouping, the hop grouping and the per-destination detail.
      For each destination the route is the one that keeps the highest security floor while staying
      inside the constellation, since that is what a pilot flying there would pick, with fewest gates
      and then the smaller system order as tie-breaks so the answer does not depend on hash order.
      The band a destination lands in is the lowest security on that route, **including the agent
      system itself**, so a destination can sit below its own band and never above the band it starts
      in. Hop counts are the gates on that same route. A member unreachable inside the constellation
      is dropped from the count rather than guessed at, so `systems` is what was counted and the
      probabilities sum to one. Unknown agent systems and empty constellations give `None`.
- [x] Because that rule collapses the grouping whenever the agent system is among the constellation's
      lowest, the spread also carries `own_bands`, the same destinations grouped by their own
      security, and a destination line names its own band when it differs from the route band.
      Sazdih from Bayuka is that case: `0.4/100%` as the grouping and `own 0.5` on Moh.
- [x] What the model assumes is now labelled on screen, which this item used to say was missing. The
      `m` pane opens with two italic lines that cannot be scrolled away: that the numbers are a model
      in which security missions stay in the agent's constellation, and that a band is the lowest
      security on the safest route rather than the system's own. Per-destination rows show `route` and
      `own` so the derivation is readable, and hop counts are the gates on that same route.
      Corrected because the claim that nothing marked it was written before the notes existed and
      survived an edit whose pattern matched a different bullet, so the file contradicted itself for
      two commits.
- [ ] Still unlabelled, because it is still unsourced: that every system in the constellation is
      equally likely, and that the agent's own system carries the same weight as a neighbour. The
      notes say the constellation bound is a model; they do not say the weighting is one, and no
      public data can settle either, per the probe below.
Deferred to a later phase:

- [x] **Correction:** the earlier TODO claim that burners are fixed named sites with one location was
      wrong. The E-Uni anomic page names variants (Guristas - Worm, Sansha's Nation - Succubus, the
      Angel Dramiel, the base Wreathe Industrial, Ashimmu and others); those are mission/variant names,
      not a single destination system. Burner sites can spawn in the agent's constellation or in a
      neighbouring constellation, so modelling burners is a separate destination distribution, as
      specified for this work below.
- [x] The Details pane has a second, explicitly labelled Burner distribution; it is not added to the
      regular-mission percentages. It appears only if an agent row for the displayed corporation and
      system passes the active filters **and** is level 4, Basic type, Security division. That makes a
      level-1 row for a system with a level-4 agent elsewhere show no burner summary unless that
      level-4 row also passes the active filters. Measured from the cached SDE: 502 such agent rows,
      across 125 corporations and 457 distinct systems.
- [x] Chosen model for that second marginal, supplied for the feature rather than measured from offered
      missions: candidate systems are in the agent's constellation or any constellation with a direct
      stargate connection to it, weighted uniformly over systems. The five standard NPC trade hubs are
      excluded both as destinations and as route transit: Jita (30000142), Amarr (30002187), Dodixie
      (30002659), Rens (30002510) and Hek (30002053). A candidate is eligible only if a route to it
      exists over the full gate graph with those systems removed; such a route may detour through other
      constellations, but those transit systems do not join the destination pool. For each candidate the
      route floor is currently the safest full-graph gate route under that restriction, including the
      origin as for the regular marginal; hops count that route too. Tests pin the current/neighbor pool
      boundary, a destination reachable only through blacklisted Jita being rejected, a permitted detour
      through a non-destination constellation, and the Level-4/Basic/Security intersection with active
      filters.
- [ ] Investigate the difference between ESI route preferences and the in-game burner route before
      changing the route rule. User-reported for Heorah -> Iderion: a 9-jump route via Rayeret, whose true
      security is 0.454546 and displayed band is 0.5. ESI `Safer` with security_penalty 0 or 1 returns a
      7-jump route through systems displaying 0.4; penalties 50, 99 and 100 return 52 jumps. Rayeret's
      cached true security is 0.454546, ESI reports 0.45454591512680054, and Dotlan reports 0.45455, so
      these values agree to their displayed precision; this is not evidence of a float-conversion error.
      The current local burner route maximizes the minimum raw true-security floor, then minimizes jumps,
      which selects the 52-jump result. ESI preferences are weighted route choices, not a hard rounded
      security cutoff. Compare route settings and displayed-band constraints against the client before
      altering the general algorithm; do not add a Heorah-specific exception.
- [ ] Check the Details pane's height with both summaries for a burner-capable system. The ten-system
      cap remains; each matching system can now add a Burner line as well as its regular summary, and
      at 80x24 some summaries may wrap. The `m` pane continues to enumerate regular destinations only,
      as requested; whether it should also show burners remains undecided.
- [ ] The remaining agent rows, 6,128 of them, covering non-security divisions and the research,
      story, factional warfare, epic arc and event types.
- [ ] Authenticated character data of any kind, and the CSV and JSON exports, neither of which has a
      shape today for a list of destinations per system.
- [x] Tests for the layer now in place: `neighbour_map` sorted and free of duplicate edges, every gate
      reaching back to where it came from, empty neighbours for an unknown system, hop counts over the
      measured Kimotoro graph at 0, 1, 2 and 3 jumps for 1, 4, 1 and 1 systems, the region and
      constellation accessors answering `None` rather than guessing, and the cache-version gate for both
      a missing version key and the version 2 shape.
- [x] Tests for the grouping: the worked four-system example, three systems at 0.5 and one at 0.6,
      groups 0.75 and 0.25 by own security under both a chain and a star, and 100% in the start's own
      band by route, because the start is the lowest of the four and every route begins there. The
      hop grouping still differs between those two shapes, and again when the agent sits further in,
      so the two groupings are tested against the topology separately. Where the safest way and the
      shortest way disagree the test pins the safest: two routes of equal length, one holding a floor
      of 0.55 through B and one 0.50 through C, and the destination is grouped at 0.6. Destinations
      carry id, hops, route band and own band together; an unknown agent system gives `None`; a member
      reachable only by leaving the constellation is dropped and the denominator follows.
- [x] The earlier invariant that uniform weight makes the band grouping independent of the graph holds
      for `own_bands` only, not for the route grouping: a route is a property of the gate graph as
      well as of membership, which is what the safest-versus-shortest test above exercises. Stated
      here because the bullet it replaces claimed it for both groupings.
- [x] A real constellation is in the fixtures as well: Sazdih, six systems measured from ESI on
      2026-10-10, five rounding to 0.4 and Moh alone to 0.5, giving 1/6, 3/6 and 2/6 by hops from
      Bayuka, 5/6 and 1/6 by own band, and a single 0.4/100% by route band because Bayuka itself is
      the lowest security on every way out. Three of its systems host agents in this tool's own data,
      so it is a case the feature will meet rather than an invented one. Security is stored to four
      decimals, finer than the tenths bands round to, which keeps fixture lines readable without
      moving any system between bands. Kimotoro stays as the fixture for the hop counts measured
      against ESI; it is not exercised as a band case, since all seven of its systems share one band
      and so it would prove nothing about a split.
- [ ] No live terminal check is claimed for any of it, per the standing note on such checks. The
      pane's text is pinned by assertions on the exact rendered lines, including the collapsed
      Sazdih case, a destination whose id is unknown and the notes at the narrowest supported width,
      and the whole pane has been printed from a TestBackend at 80 columns, but nobody has opened it
      with `m` in a real terminal.
- [x] The pane's *numbers* are now checked against the version 3 data on disk, through a temporary test
      that read the real cache and printed the rows — no download was needed, the archive was already
      fetched. That harness later rendered **both** panes as grids at 80x24 and 120x40 from the real
      cache, which is how the two bugs below were found; it was deleted afterwards, so the numbers and
      the shapes they produced are recorded here instead of in a test that cannot run on another
      machine. Still nobody has opened `m` in a real terminal. Bayuka's spread from the live map is `bands [(4, 1.0)]`, `own_bands [(4, 5/6), (5, 1/6)]`
      and `hops [(0, 1/6), (1, 3/6), (2, 2/6)]`, exactly the fixture, with `Moh (2, 4, 5)` the one
      destination whose own band differs from its route band. The test was deleted afterwards and the
      numbers recorded here instead.
- [ ] **Real data exposed a shape the fixtures never had, and it is a live problem.** 9-ERCP, a nullsec
      Security agent system, printed `route -0.1/100%  0J/100%`, which reads as "every mission lands in
      your own system". It is not a one-system constellation: 9-ERCP belongs to 20000218, which has 7
      members, none of them reachable by stargate — a wormhole pocket, whose systems are joined by
      connections the model deliberately ignores. The spread drops members unreachable inside the
      constellation, so the denominator fell from 7 to 1 and the row shows a certainty about a
      constellation it only counted one system of. Measured on the archive: 1,184 constellations split
      766 all-gated and 418 all-gate-less with none mixing the two, which confirms the earlier claim
      that no gated constellation contains a gateless member, and 52 of the 1,239 distinct Security
      agent systems (4.2%) sit in those gateless pockets, where every row the pane prints is like this
      one. What such a row should say is left open in Phase 11.
#### Phase 11: The systems the gate model cannot reach

Phase 10 bounds destinations to the agent's constellation and counts gates between them. For some
agent systems that produces a row that reads as a certainty about a constellation it only counted one
system of. The two groups below are measured, and both are **left as open decisions rather than
changed**: the levels involved are small, the fix for one group needs data that does not exist, and
the LP stores affected are a narrow set of corporations.

Measured from the version 3 cache on disk (283 corporations, 8,490 systems, 13,978 gate records) with
constellation names and system counts read from `GET /universe/constellations/{id}` on 2026-10-10.
Region ids resolved from the cached `regions.json`: 10000017 is **J7HZ-F**, 10000019 is **A821-A**.

**Group A — 52 agent systems standing in a system with no stargate at all**, in 15 constellations, all
of them in those two regions: 9 constellations and 29 systems in J7HZ-F, 6 and 23 in A821-A. Every one
displays a negative band, from `-0.008` (6-23NU) to `-0.650` (N-FKXV).

| constellation | region | members | agents here | the systems |
| --- | --- | --- | --- | --- |
| B9J-NT (20000240) | A821-A | 7 | 6 | 5V-YL6, 69A-54, 8-UWFS, BWO-UU, PQWA-L, SQVI-U |
| I4-DK8 (20000218) | J7HZ-F | 7 | 5 | 54-VNO, 9-ERCP, IAMZ-5, KN7M-N, PBXG-A |
| F-0CTG (20000242) | A821-A | 7 | 5 | 0IRK-R, 7BA-TK, FNS3-F, FR-RCH, IAWJ-X |
| 3A-CND (20000224) | J7HZ-F | 8 | 5 | 0M-M64, 1C-TD6, H7OL-I, TO21-U, YBYX-1 |
| 4I-WP1 (20000219) | J7HZ-F | 6 | 4 | 6-QXE6, FO-3PJ, N-FKXV, X7-8IG |
| NSM-6F (20000220) | J7HZ-F | 8 | 4 | 6-NCE7, BKK4-H, CZ-CED, L-TPN0 |
| W1-F3U (20000237) | A821-A | 6 | 4 | 7-8XK0, 90G-OA, G-N6MC, P3UD-M |
| O-BSIB (20000236) | A821-A | 6 | 3 | 6-HFD6, 9-BUSQ, II-1B3 |
| 826-A5 (20000238) | A821-A | 8 | 3 | B-Y06L, CLDT-L, Z-KPAR |
| F-8DMX (20000221) | J7HZ-F | 6 | 3 | 3-XORH, 6-23NU, W6H6-K |
| 2RX9-6 (20000223) | J7HZ-F | 6 | 3 | 8-ULAA, H-EDXD, KF1-DU |
| 1-Z95W (20000213) | J7HZ-F | 6 | 2 | 1SR-HT, SQ-2XA |
| UN2H-D (20000241) | A821-A | 6 | 2 | T-C5A0, T-YWDD |
| D-YMGR (20000222) | J7HZ-F | 6 | 2 | I9D-0D, LH-J8H |
| WSC-AL (20000216) | J7HZ-F | 6 | 1 | PQA-9K |

The corporations are a short list: **Jovian Directorate, Jove Navy and Shapeset** hold most of the 52,
with **Prosper, X-Sense, Genolution and Impro** in the J7HZ-F pockets. So the affected LP stores are
Jove-associated and a handful of nullsec ones, not the empire running most users do.

**Group B — 14 agent systems in gated constellations that are not one gate component.** These systems
have stargates; the constellation just falls into two pieces, and the other piece is reachable only by
leaving it, which the model forbids.

| constellation | region | members | the two components (`*` hosts a Security agent) |
| --- | --- | --- | --- |
| 20000354 | Molden Heath | 5 | Fegomenko, Osvetur, Mimiror, Unertek\* \| Ennur\* |
| 20000367 | Heimatar | 8 | Hulm, Ameinaka, Malukker \| Osoggur\*, Abudban\*, Trytedald\*, Odatrik\*, Rens\* |
| 20000388 | Sinq Laison | 6 | Ardene, Boillair, Ney\*, Fasse \| Gratesier, Schoorasana\* |
| 20000410 | The Citadel | 5 | Yria\*, Oshaima\*, Hysera\*, Oisio\* \| Venilen\* |

Two consequences worth keeping straight. **Group B is fixable with the data already on disk**, because
a gate route exists — it just crosses a constellation border, so hop counts and route floors would have
to be allowed to leave. **Group A is not fixable at all from public data**: the current SDE carries no
wormhole-link table, `mapStargates.jsonl` being the only edge list, and `GET /route/{origin}/{dest}`
answers `No route found` for a pocket pair (`30001484` to `30001480`, same constellation) while the
control pair still returns a 10-system route. The live hole topology is not static data.

- [ ] Group A rows: what should a pocket system's row say? Options are naming the members the gate
      graph cannot reach (`0J/100%  6 of 7 systems unreachable by gate`), suppressing such rows and
      counting them once at the top of the pane, or leaving the row as it is. Deferred by choice.
- [ ] Group B: allow routes to leave the constellation while keeping destinations inside it. This is a
      semantics change to hop counts, not just a label, and it would need measuring across all 1,239
      agent systems before being trusted. Deferred by choice.
- [ ] Whether a mission site inside a pocket constellation is even reachable from the agent's own
      system without a wormhole is **not measured here**, and neither group's numbers should be read as
      saying anything about it. The cache says only which systems have gates.

### A progress popup while a re-run is in flight

Applying new filters starts a second run while the first run's results are still on screen. Until now
the only sign of it was the header line prefixed with the stage text and `(stale)` on each pane title,
because the two progress bars live in `draw_progress`, which `draw` reaches only when no report exists.

- [x] Trigger is `App::stale` with the stage still in flight — any run over existing results, not only
      one started from the filters dialog. No new flag for the trigger: `stale` already means "a run
      superseded the report on screen", set in `start_run` and cleared in `finish`.
- [x] `App::awaiting_run_summary` holds the end of a run on screen until any key is pressed, because
      the loop drains run events before it draws and ticks every 250 ms, so a warm run can finish
      inside one drain and never be drawn mid-flight. Set in `finish` and `fail` only when a report
      was superseded, cleared by `start_run` and by any key press.
- [x] Purely visual: the popup takes no key and is not an `Overlay` variant, so Tab, the arrows, `l`,
      `m`, `e` and `q` all behave exactly as before, and the press that acknowledges a summary does
      whatever it was already going to do. A new `Overlay` variant would have had to be considered by
      every existing overlay-conditional branch, which is where the risk was.
- [x] Other overlays win: the popup draws only when `app.overlay == Overlay::None`, so the log, help,
      donate, export and the dialogs cover it and closing them brings it back.
- [x] Shows both bars, from `app.gauges()` through the existing `gauge()`, so the idle one reads
      `waiting` as it does inline; the stage line, red on a stopped run; and one key line, present
      only for the summary, since that is the only state where a key changes anything. Nothing else
      goes in it — the filters and the ESI counts are already in the header and footer, which stay
      visible behind a centred popup.
- [x] `fail()` still leaves `stale` set, so a stopped re-run keeps its panes titled `(stale)`. Kept
      deliberately: the results on screen are from a superseded run. It does mean the popup can be
      acknowledged while the titles still claim staleness, which is the honest reading.
- [ ] Pre-existing wording, made more visible by this change, left unfixed: `stage_text()` prints
      "Finished: 1 groups priced". It appears in the header and the progress view too, so correcting
      the singular is a separate change with its own tests, not part of adding a popup.

### Filter agents by where they stand

`--agent-region` and `--agent-system`, with the same pair of boxes on the Filters page directly
below `Security`. They choose which corporations are processed by asking where the corporation's
agents stand, which is a different question from `--region` and `--system`, where the prices are read
from. Measured on the real archive to keep the two apart in the wording: 283 corporations are cached,
114 regions and 8,490 systems are known, and the two pairs name the same kind of value for entirely
separate purposes.

- [x] Resolution follows the market pair exactly: a number is taken as an ID without checking it
      exists, a name is looked up and refused when nothing answers it, and a blank field is no filter.
      `--agent-region Blach Riss` prints `Region 'Blach Riss' not found`; `--agent-system "Jita Space"`
      prints `Unknown --agent-system 'Jita Space'`, which is the answer ESI gives too — there is no
      such system.
- [x] All the conditions still have to hold for the **same agent**, as the help text has always
      promised. A corporation with a level 4 agent in region 8 and a level 2 agent in region 7 fails
      `--agent-region 8 --agent-level 2` even though each half matches somewhere. Tested against a
      fixture with agents on both sides of a region border.
- [x] The Filters page is twelve fields now, one line each, and the notes beneath them carry the
      hints, the valid values, and every validation error. At the old 20-row minimum the notes had two
      rows of room, and two more fields would have taken that to zero: a form that cannot show its own
      errors. `MIN_HEIGHT` is 24, with a test that each new field's hint and a planted error are
      visible at exactly the minimum size, so the room is not lost again silently.
- [x] The TUI checks the agent pair the way it checks the market pair: `Agent system` is cleared when
      the agent region moves out from under it, with a log line saying so, and a pair that disagrees
      is refused on the system field before the run starts rather than quietly keeping nothing. The
      command line keeps its existing behaviour of letting a mismatched pair through, which is what
      `--region` and `--system` do today: measured, `--agent-region "Black Rise" --agent-system Jita`
      prints `Filtered to 0 corporations ... (1 removed)`.
- [x] Verified against the live data rather than only fixtures: `--agent-system Jita` keeps Paragon
      and prices it; the region `Black Rise` keeps 9 corporations, and its ID `10000069` keeps the
      same 9, so the name and the number are the same filter; `--agent-system Bayuka --agent-level 1`
      keeps Ammatar Consulate, which is the corporation the Phase 10 destination work found in that
      constellation; `--agent-region "Black Rise" --security 0.4..0.5` keeps 5, fewer than the 9 the
      region alone keeps, which is the same-agent rule narrowing further.
- [x] The remembered filters in `filters.json` carry both new values, and a filter named in the
      header line reads `--security 0.5..0.7 --agent-region Black Rise --agent-system 4-3L53` in the
      order the form shows them.
- [x] Two style notes from keeping the checks at their baseline counts. The new conditions in
      `AgentFilter::matches` are written `is_none_or`, while the four that were already there stay
      `map_or(true, ..)`: clippy asks for the newer form, and rewriting lines this change did not
      touch would bury the diff. The file therefore mixes the two until someone sets the older four
      right on their own.
- [x] `Loaded::Filters` is boxed. Adding two fields to `SavedFilters` pushed that variant past
      clippy's `large_enum_variant` threshold beside a bare `String`, and the payload is read once at
      start-up, so the box is cheaper than the warning or an `#[allow]`.
- [ ] One sharp edge left as the market pair has it: a numeric region is not checked to exist, so a
      mistyped `--agent-region 1000069` keeps nothing without saying anything. A name is checked, a
      number is not. Making the ID form also ask the static data would be a behaviour change for
      `--region` too, which was not asked for; noted rather than done.
- [ ] The refusal message for a region names no flag — `Region 'Blach Riss' not found` — copied from
      `--region`, while the system message does name its flag. In the TUI the error sits beside the
      box it came from, so only the command line can leave a user guessing which of the two they
      mistyped. Worth unifying, but that changes `--region`'s existing text.

### Security range syntax

`--security` reads `<op><level>`, where the operator is `+`, `-`, or `=` and the level follows. The
first character is therefore an operator and not a sign, so `--security -0.5` means *at most 0.5*
even though security is negative in nullsec. A range wants two bounds and the dash is already taken.

New syntax, where each bound is a security level from -1.0 to 1.0:

| Written | Means |
| --- | --- |
| `0.5` | exactly 0.5 |
| `0.5..0.7` | at least 0.5 and at most 0.7, both bounds included |
| `0.5..` | at least 0.5, the old `+0.5` |
| `..0.5` | at most 0.5, the old `-0.5` |
| `-0.9..-0.5` | a range below zero, which the `..` separator makes unambiguous |

- [x] `SecurityFilter::parse` rewritten for the table above. It splits on `..` rather than on a single character, so a leading `-` stays a sign. No existing form is accepted: `+0.5`, `-0.5`, and `=0.5` are errors, and each says what to write instead, because `-0.5` would otherwise parse as a number and quietly change meaning from *at most 0.5* to *exactly −0.5*. Checked live against all six refusals, each naming its replacement.
- [x] Both ends of a closed range are inclusive, and the single bound of an open range is inclusive too, so `0.5..` matches what `+0.5` matched before. Verified on real data: `0.5..` keeps 152 corporations, `..0.5` keeps 167, `0.5` keeps 125, `0.5..0.7` keeps 140.
- [x] Each bound goes through `displayed_security`, as a single level already did, so `0.45..0.65` becomes `0.5..0.7` rather than being rejected. Surrounding space on the value and on either side is trimmed, so `.. ` reads as the bare `..` it means.
- [x] A lower bound above the upper one is an error naming both, `the lower bound 0.7 is above the upper bound 0.5`, rather than being swapped or allowed to match nothing.
- [x] `SecurityFilter` is now a pair of bounds, `{ low: Option<i32>, high: Option<i32> }`, with an empty side meaning unbounded. `parse` and `matches` were the only readers, both in `src/static_data/mod.rs`, so no caller changed. It derives `Copy` to keep `AgentFilter`'s derive, and gained `PartialEq` and `Eq` so a parsed filter can be compared in a test.
- [x] `..` on its own is refused, `bounds nothing. Use 0.5..0.7, 0.5.., or ..0.7`, as is an empty or non-numeric side.
- [x] First tests for `parse`, which had none: 11 covering every row of the table, both open directions, negative bounds including `-0.5..-0.5`, reversed and empty ranges, rounding, levels outside -1.0 to 1.0, each rejected old spelling, and non-numeric sides. `matches` is tested through the parsed filters rather than directly.
- [x] `src/static_data/mod.rs` — the doc comment above `SecurityFilter` named `+0.5`, `-0.5`, and `=0.5`, and `parse` described `<op><level>`. Both rewritten, and the two old error strings, `expected an operator (+, -, or =) and a level` and `the operator must be +, -, or =`, are gone.
- [x] `src/main.rs` — `--security <level|range>` in the filter list and in the usage line, the sentence rewritten to spell out all four forms and warn that a negative level makes the old `-0.5` reading gone, and a `--security 0.5..0.7` example added. The `https://wiki.eveuniversity.org/System_security` link stays, since rounding still applies.
- [x] `src/tui/ui.rs` — the `Security` hint reads `0.5 exact, 0.5..0.7 for a range.` inline, which fits the 36-column detail column at the minimum terminal width, with `0.5.. is at least, ..0.5 is at most. Blank for any.` on the second line that `hint` splits. The width test now covers `Security` alongside the two market fields.
- [x] `src/tui/form.rs`, `src/tui/mod.rs`, and `src/saved_filters.rs` used `"+0.5"` as a sample value, now `"0.5..0.7"`, and the filter summary assertion moved with it.
- [x] Consequence of rejecting the old forms: a `filters.json` written by Phase 7.1 can hold `"security": "+0.5"`, and on the next session Enter fails validation on that field with the replacement named. The saved value is not silently reinterpreted, and the run does not start on a filter the user did not choose. Shown through `set_errors`, whose inline display is covered by `a_validation_failure_replaces_the_note`; not seen in a real terminal.

### Refactoring from the 2026-10-12 review

A read-through of the whole codebase, in five pieces, each its own commit: the ESI pagination fix,
cheap cleanups, one table for the filter fields, the static data module, and the TUI state and
render code. Behaviour changes are the ones listed under each piece; everything else is meant to
leave the program doing what it did.

#### ESI pagination

- [x] A cached page 1 of a multi-page response came back as the only page. `fetch_single_page`
      answered every cache hit with a page count of 1, and `fetch_all_pages` followed each page's
      own count, so the walk stopped after page 1 with pages 2 onward cached beside it. Shown
      before the fix with page 1 and page 2 both cached: `get` returned `[1]`. Affected `--example`,
      which reads every order in a region and every type id, and any type with more than 1,000
      orders in the priced region; the cache on this machine held no multi-page entries, so no
      `--lpstores` run here is known to have been cut short.
- [x] Cache files now hold `{"pages": n, "body": ...}`, and the page count comes from page 1 alone,
      cached or not. A file without the wrapper, which is every file written before this, reads as
      one page, as it always did, so nothing is refetched on upgrade; an old multi-page entry would
      stay cut short until it expires. File names did not change, so `--clear-cache` and the
      sibling lookup are unaffected. The page loop is a `for` over `2..=pages` with a `with_page`
      helper, replacing the query rebuild that found and incremented the `page` parameter.
- [x] A cache write that fails no longer fails the request. `write_cache` returning an error, from
      an Expires header that is not a date or a full disk, used to turn a response already in hand
      into an error through `?`. `store` now logs `[CACHE WRITE FAILED] <path>: <reason>` to the
      log sink, unconditionally rather than only under `--verbose`, and the body is returned.
- [x] Checked live against ESI: `universe/types` fetched 53,288 ids over 54 pages into an empty
      cache, then a second client read the same 53,288 from 54 cache hits with no requests.
- [x] Tests: 6 added, covering every page read back from cache, a file without the wrapper reading
      as one page, a written page keeping its count, an object body not mistaken for the wrapper,
      the `page` parameter replaced rather than repeated, and a failed write being logged. The
      first fails on the old loop: mutated to stop after page 1, it returns `[1]` against
      `[1, 2, 3]`.

#### Cheap cleanups

- [x] Removed code nothing called: `ExpensesEndpoint::offer_expenses`,
      `MarketsEndpoint::orders_by_type`, `EsiClient::stats_clone`, `StaticData::corporation_names`,
      and `saved_filters::save`, whose one caller had moved to `save_to`. Each was checked with a
      grep over `src/`; there is no `tests/` or `examples/` directory. The ESI builder setters stay,
      since they set public config fields and `base_url` is what the offline cache tests use.
      Making the static data module's internal `pub` items private is left to the static data
      piece, which moves them between files anyway.
- [x] `OfferResult::profit_per_unit` replaces the reward-less-unit-cost match written out in
      `output.rs`, the details pane, and the export. `format::format_optional` replaces the
      `map(format_number).unwrap_or("n/a")` chains in `output.rs` and backs the details pane's
      `optional`.
- [x] `OfferResult`, `LoyaltyOffer`, and `ExpenseBreakdown` derive `Default`, so a test fixture names
      only the fields it is about. The two `group` helpers in `src/report/` are one, and the four
      sort tests moved from `pipeline.rs` to `report/mod.rs`, beside `Report::sort_by_depth`, which
      is what they test. The full priced-offer fixtures in the TUI are left for the TUI piece,
      which gives them one home.
- [x] `corp_line` cut its system list at a literal `10` while `MAX_SYSTEMS_SHOWN` named it, and
      `--example` defaulted to a literal `10000002` and rebuilt `region_label` by hand. Both use the
      names now. `output::corp_label` was a private copy behind `corporation_label` and is gone.
- [x] Docs that the code contradicted: the field counts in `saved_filters.rs` ("nine",
      "eleven"; there are twelve, and the text now gives no number), the `StaticData` summary
      (three `data/` files, name-to-ID only), the archive size ("several hundred megabyte";
      `curl -I` gives a `content-length` of 99,297,801), the stargate note saying the cache builder
      deduplicates (`neighbour_map` does), `status_text` claiming the TUI footer as a caller, the
      comment on the offers table's column 4, `main.rs` naming `eve_lpstore::tui::run` when the TUI
      is part of the binary, and `--verbose` in `HELP_TEXT` implying the view logs the two
      directories only with the flag. Two test comments described values their fixtures do not
      hold, and one doc comment sat above the wrong item.
- [x] Clippy: 26 warnings before, 17 after. The 16 in `src/static_data/mod.rs` go with that piece,
      and `large_enum_variant` on `Opening` with the TUI piece. `format_imbalance`'s `Some(0.0)`
      pattern compares with `==`, so it still matches `-0.0`, which
      `the_imbalance_shows_its_sign_and_never_a_negative_zero` covers and still passes.
- [x] Checked by output: `--lpstores` over all 283 corporations, run by the binary from before
      this review and by this one against the same copied cache with `--cache-minutes 1000000`,
      printed the same 2,499 lines apart from the timings, from 24,233 cache hits and no misses.
- [x] Observed during that check, and not in any piece: pricing took 78 to 81 seconds in a debug
      build with every lookup a cache hit. Each lookup reads the whole cache directory, 3,734
      files at the time, which is the likely cost but was not profiled. Indexing the directory
      once per client was offered with the ESI piece and not chosen. Done afterwards; see *The
      ESI cache index* below.

#### One table for the options that take a value

- [x] `report::Field` is the table. Each variant has one row in `Field::spec`: the flag, how its
      value is written, its label in the form, its key in `filters.json`, and the accessors into
      `LpStoreOptions`. `Field::ALL` lists them in the form's order, then the command-line-only
      ones, and `CacheDir` joined as the fifteenth, since `--cache-dir` takes a value like the rest.
      `LpStoreOptions::value` and `value_mut` read and write through the row.
- [x] What reads the table rather than a list of its own: argument parsing in `main.rs`
      (`parse_options`, replacing `VALUE_FLAGS` and the fifteen `value(...)` calls), the usage line
      (`usage_line`), `SavedFilters` (`From`, `overlay`, and its serde), the form (`FormField`
      became `Text(Field)` or `Expire`, and `FilterForm` keeps its text in an `LpStoreOptions`),
      the form's labels, the mapping of an option error to its box, and `filter_summary`.
- [x] Adding an option was about 27 edits in 6 files, 8 of which compiled when forgotten. Now it is
      the variant, its row, its `ALL` entry, the struct field, and its meaning in `resolve`, plus a
      help line and a form hint. The compiler forces the row, the hint, and the struct literal in
      `every_text_option_is_reached_through_exactly_one_field`; that test then finds an entry
      missing from `ALL` or two rows sharing a field, and `every_option_is_documented_and_read`
      finds a flag missing from the help or the usage line or not reaching its option. The two
      that still default quietly are deliberate: a new option is not a name field for
      suggestions, and is included in the filter summary.
- [x] Behaviour change, as chosen: a value flag at the end, or followed by an argument starting
      with `--`, is refused with `<flag> needs a value`. It used to be read as unset, so
      `eve_lpstore --depth` ran with the default depth, and `--lpstore --tui` took `--tui` as the
      corporation list while also opening the view. A value with one leading dash is still a
      value, so `--security -0.5` reaches the security check, which refuses it as before. Given
      twice, the first value still wins. Checked with the binary for `--depth`, `--lpstore --tui`,
      and `--security -0.5`; first-wins is checked by a unit test only.
- [x] The usage line gained `--agent-region` and `--agent-system`, which it had been missing, and
      now lists the switches first and the value flags in the form's order. The help's
      `--lpstore <list>` reads `--lpstore <id|name,...>`, as the usage line already had it, because
      the test now requires the two to agree.
- [x] `filters.json` keeps its keys, and a file written before this loads every field, checked on
      a literal copy of the old shape and on the file in `~/.config/eve_lpstore`. Two differences:
      keys are written in the form's order, and any JSON array is now reported as unusable rather
      than read by position. A value of the wrong type names its key: `depth is 10, where a string
      or null was expected`. A command-line-only key typed into the file is ignored.
- [x] Checked by output: `--lpstores` over all 283 corporations, and again with
      `--security 0.5..0.7 --agent-level 4 --agent-division security --faction caldari --depth 5`,
      printed the same lines as the binary from before the review, apart from timings.
- [x] Tests: 11 added and the rest moved to the table's names, covering the table against the
      struct, every flag naming its field, every option in the help, the usage line, and the
      parser, the missing-value refusals, the one-dash value, an unknown argument, first-wins,
      the old file shape, the written shape, an array, a wrongly typed value, a command-line-only
      key, and the form carrying only its own fields.
- [x] Observed while checking: `~/.config/eve_lpstore/filters.json` was rewritten at 01:09:50
      during this piece, with `security` going from `0.5..0.6` to `0.6`. Its key order is the
      old serializer's, so a binary from before this change wrote it, and a test run left its
      timestamp unchanged; inferred to be a session of the view outside this work.

#### The static data module

- [x] `src/static_data/mod.rs` was one 2,351-line file. It is now `mod.rs` (the struct, `SystemInfo`,
      and the name and id lookups), `security.rs` (displayed security and `SecurityFilter`),
      `agents.rs` (`AgentRow`, `AgentFilter`, and the agent queries), `routes.rs` (the gate graph,
      `DestinationSpread`, and both mission spreads), `sde.rs` (the archive's record shapes and
      extraction), `cache.rs` (the four files, `is_cached`, `load`, and reading them back),
      `error.rs`, and a test-only `fixtures.rs` for the fixtures more than one test module uses.
      Each test moved beside what it tests. The move was done by cutting the file at item
      boundaries, and a line-for-line comparison of before and after shows only visibility,
      imports, and module headers changed. Phases above that name `src/static_data/mod.rs` for
      something now in a submodule were right when written.
- [x] `StaticDataError` replaces `EsiError` as what `StaticData::load` returns. Archive errors
      were wrapped as I/O errors and JSON write errors were reported as "Deserialization error",
      none naming a file. Each case now names the file it is about: `mapRegions.jsonl in the
      static data archive, line 3: ...`, or the cache file's path. This is the text `main` prints
      after `Failed to load static data:` and the static data dialog shows as the reason.
- [x] The archive is opened once rather than eleven times, and one `records::<T>(file)` replaces
      the eight copies of the read-split-parse loop and the three extractors that did it their
      own way. Those three found their files by substring, `region` and `corporation`, which
      depended on archive order; they are now looked up by exact name, `mapRegions.jsonl` and
      `npcCorporations.jsonl`. The archive's central directory, read with a range request, lists
      `npcCorporations.jsonl` first and `mapRegions.jsonl` as the first file containing `region`,
      so the same files are read. `zip.by_index(i).unwrap()`, which panicked on a damaged archive,
      is gone with the substring search. The four `{ en }` name records are one `SdeName`.
- [x] Systems are a named `SystemInfo { name, true_sec, region_id, constellation_id }` instead of
      a 4-tuple taken apart by position. The cache file keeps its tuple rows, so its version stays
      at 3 and nothing is downloaded again.
- [x] Fifteen items that were `pub` with no use outside the module, and `files`, are private or
      `pub(super)`. `agent_systems` got its own doc comment, which had been sitting above
      `security_division_id`. Clippy's 16 warnings in the module are fixed, which leaves one in the
      crate, `large_enum_variant` on the TUI's `Opening`.
- [x] Checked end to end: the binary from before the review and this one each downloaded the
      archive into a temporary `HOME`. `regions.json`, `corporations.json`, and `types.json` came
      out byte-identical. The agent file has the same size and the same contents, but its
      `corporations` and `stations` lists are in a different order; both are collected from a
      `HashMap`, so their order varies between runs of either binary (inferred from the
      `RandomState` hashing, not shown with a second download). The `--lpstores` output of the
      two runs matched.
- [x] Tests: 4 added, for a failing line named by number, a missing archive file named, the
      English name read beside another language, and a damaged cache file named by path. The
      last reads through `load_from_disk` rather than `load`, so a regression in `is_cached`
      cannot turn it into a download.
- [x] Not done here, and noted by the review: the burner route search copies each route's full
      path at every step and then discards it (about 14 ms per agent system, 25 ms at worst, in
      the review's release-build measurement), and the agent file is read three times at start-up.
      The two `HashMap`-ordered lists could be sorted before writing so the file is the same bytes
      on every download. Done afterwards; see *Static data performance* below.

#### The TUI state and render code

- [x] The export prompt and the static data dialog carry their own state:
      `Overlay::Export(ExportPrompt { path, error })` and
      `Overlay::StaticData(StaticDataDialog { downloading, error })` replace four `App` fields that
      outlived their popup. A download flag with no dialog open, or a refusal with no prompt, can
      no longer be represented. One consequence: the export name is gone once the prompt closes,
      which nothing read, since opening the prompt again always rebuilt it.
- [x] `on_key` is one `match` on the overlay. The order that decides whether `q` quits, after the
      two text prompts and before everything else, is a guard arm in that match instead of the
      position of an `if` in a chain of five. The keys of the results view and of the overlays
      that only show something are `on_view_key`.
- [x] The run's region, region id, agent region, corporation count, and agent filter are one
      `run: Option<RunInfo>`, read through `region()`, `corporations()`, and `agent_filter()`.
      This removes the region id's zero placeholder for "no run yet". The filter summary and the
      depth stay on `App`, because a depth-only apply changes the depth without a run, so
      `RunInfo` no longer carries them and `start_run` takes them beside it. `set_applied`, a
      setter for a public field, is gone.
- [x] The market and agent pairs are one `Pair` with accessors, so `region_moved`,
      `clear_system_on_region_move`, and the outside-the-region check are each written once and
      `apply` walks both pairs in a loop, market first, as before. `system_outside_region`, the
      testable rule, is unchanged.
- [x] `opening` takes only what loading returned, and `None` means not on disk, which removes the
      `cached` flag and the arm the old comment called unreachable. `Opening::Filters` boxes the
      data, which was clippy's last warning in the crate; there are none now.
- [x] `src/tui/test_support.rs` holds `press`, `forge_run`, `priced_offer`, and `results_app`.
      The run info was written out six times across `app.rs`, `mod.rs`, and `ui.rs`, and the other
      three twice each. `app.rs` keeps its own minimal `offer` and `app_with`, which build
      different values.
- [x] `ui.rs` was one 2,871-line file. It is now `ui/mod.rs` (`draw`, the header and footer, the
      results layout), `text.rs`, `popups.rs`, `progress.rs`, `filters.rs`, `offers.rs`,
      `details.rs`, `destinations.rs`, and `tests.rs`, which holds the tests as they were, since
      most render the whole view. The cut was at item boundaries again, and a line comparison
      against the file formatted the same way shows only visibility, module headers, and
      signatures rustfmt re-wrapped for the longer prefix.
- [x] Three popups share one `popup` helper and `POPUP_WIDTH`, instead of writing 64 three times.
      The offers table is one `OFFER_COLUMNS` array of title, width per layout, and cell
      function, replacing three parallel lists that a test had to keep the same length and a
      comment about column 4 that was wrong. Three doc comments that sat on the wrong item are
      on the right one.
- [x] Checked in a pseudo-terminal, 120x40, against temporary homes: with no static data the view
      opened on the download dialog and Esc exited 0 without creating a directory; with the
      static data it opened on the filters dialog and Esc exited 0; a one-corporation run against
      the copied cache, then `e` and Enter, drew the export prompt and wrote
      `lpstores-the-forge--10000002----lpstore-1000002.csv` with the run's rows. The render sweep
      from 80 to 160 columns still passes, which is what covers the offers table layout.
- [x] Tests: 3 added and 1 replaced, for each pair reading its own fields, each layout's column
      titles and widths, and only the spread and imbalance cells being coloured; the length check
      between the three old lists is gone with the lists.
- [x] Not done here, and noted by the review: `mission_spread` runs on every redraw, 6.9 ms
      across all agent systems by the review's measurement; the run and download results arrive
      by `oneshot` polled beside the event channel; the export prompt and the form edit text with
      two copies of the same key handling; and `Session.sdata` is an `Arc` nothing clones. Done or
      closed afterwards; see *TUI follow-ups* below.

#### The ESI cache index

- [x] `check_cache` listed the whole cache directory on every lookup, because a cache file's name
      ends in its expiry and cannot be opened without knowing it. It built a prefix string for
      every entry it passed and stopped only at a match, so a hit read about half the directory
      and a miss all of it. A full listing of 2,808 entries measured 3.85 ms in a release build
      and 8.00 ms in debug, in a scratch crate using the same `tokio::fs::read_dir` loop.
- [x] Each `EsiClient` now keeps a `CacheIndex`, a map from the file name without its
      `_{expiry}.json` ending to the expiries on disk for it. A lookup reads from the index, so a
      hit lists nothing. A miss lists the directory once more before going to the network, which
      is how a file another process wrote is still found, and every write adds its file. The
      view builds one client per run, so each run starts from a fresh listing.
- [x] Chosen over a lock file in the data directory, which was proposed and checked first. The
      cache directory, not the data directory, is what two processes share and what
      `--cache-dir` moves. A check at startup only tells the second process about the first. A
      lock file left by a crash or a Ctrl-C on `--lpstores` would warn and re-list on every later
      run, unless it were an OS lock, which needs Rust 1.89 against the current 1.88 floor. And
      the saving is small: the listing on a miss is a few percent of the ESI request after it,
      about 120 ms in the 54-page live check, and missing another process's file only costs a
      duplicate request.
- [x] Behaviour that changed beside the speed: when two usable files exist for one entry, the
      newest expiry answers, where directory order used to decide; every expired file for the
      looked-up entry is deleted, not only those the scan passed before a match; and a file that
      vanished from disk since the index saw it is dropped from the index and counted as a miss
      rather than as corrupt. The verbose `[CACHE HIT]`, `[CACHE MISS]`, and `[CACHE WRITE]` lines
      read as before, including `(cache dir not found)` before the first write.
- [x] Measured on one copied, warm cache of 3,734 files with `--cache-minutes 1000000`, both
      binaries making 24,233 lookups that all hit: release went from 46.7 s to 2.0 s wall (pricing
      45.96 s to 1.90 s) and debug from 88.9 s to 9.3 s (87.06 s to 8.58 s), with the same output
      apart from timings. No profiler is installed, so this attributes the time by before and
      after rather than by a profile. A cold single-corporation run into an empty directory
      logged one `cache dir not found`, 205 misses and 206 writes, and 119 hits on files it had
      just written; the second run hit all 325 lookups.
- [x] Tests: 7 added, for a hit listing nothing after the first lookup, a file another process
      wrote being found on the next miss, the client's own write found without a listing, a
      removed file being a miss that leaves the index, the newest file answering while expired
      ones are deleted, a missing cache directory, and which file names are entries. The first
      fails when every lookup is made to list again: 4 listings against 1.

#### Static data performance

- [x] `best_routes` no longer keeps each route's path. The path was copied at every step and
      used only as the last tie-break, between routes equal on floor and gates, and both callers
      dropped it. A route is now `(floor, gates)`, and the first of two equal ones stands.
      Dropping the tie-break also changes which systems are queued again, and the bug below makes
      the result depend on that order, so sameness was checked rather than assumed: every
      mission spread of all 8,490 systems and every burner spread of the 457 burner origins,
      dumped from the real static data before and after, are byte-identical. A burner spread went
      from 12.93 ms to 3.92 ms each in a release build.
- [ ] Open, recorded and not fixed, as chosen: the gate counts are not always the fewest at the
      best floor. The search keeps one route per system, so when a safer but longer route replaces
      a system's route before that system is expanded, the shorter route never extends past it.
      Shown on a six-system graph: O to X by A1 and A2 at 0.6 (3 gates) or by B at 0.5 (2 gates),
      Y at 0.4 one gate past X; every route to Y has floor 0.4, and the shortest is 3 gates through
      B, but Y reports 4. Bands are exact; only the hop shares can be off. From the user: the route
      model is shaped by an ESI bug, a rounding error that rounds some systems with true security
      between 0.4 and 0.5 down to 0.4 in ESI's route calculation and so applies the security
      penalty, and the user is in contact with FC about it. Not verified here. The
      `mission_spread` doc comment now says the gate counts can be high.
- [x] The agent file is read once at start-up instead of three times. `StaticData::load_cached`
      returns the data, `None` when it has to be downloaded (a file missing, or the agent file from
      an older build or unreadable), or an error when current files do not load; `download`
      fetches and reads back. `is_cached`, `load`, and `agent_cache_version`'s file read are gone,
      and the CLI's `announce_static_data_download` became `load_static_data`, which says the same
      line before downloading. Measured in three rounds of ten starts: release 37.0 to 39.7 ms
      before and 32.4 to 33.8 ms after, debug 409 to 422 ms before and 331 to 338 ms after. One
      version check alone is 6.11 ms in release and 71.6 ms in debug, which the old start ran
      twice.
- [x] The agent file's `corporations` and `stations` lists are sorted by id before writing, so
      the same archive always writes the same bytes. They came out of a `HashMap`.
- [x] Tests: 1 added, for the two lists coming out in id order, which fails with the sorting
      removed; the three cache tests moved from `is_cached` and `agent_cache_version` to
      `load_cached` and the parse of the text, and the damaged-file test no longer has a path to a
      download.

#### TUI follow-ups

- [x] A run and a download each return their result from the task, and the UI loop reads it with
      `take_finished` once the `JoinHandle` reports it is finished: `block_on` on a finished task
      returns at once, and is allowed because the loop runs on a blocking thread. This replaces a
      `oneshot` channel per task beside its handle, and the two `run.done = None` writes through a
      second borrow. A task that panicked still reads as a failure, through the `JoinError`.
- [x] `Edit` decodes Backspace, Ctrl+U, and a character typed without Ctrl or Alt once, for both
      the filter form and the export prompt, which had each written the same three arms and the
      same modifier test.
- [x] `Session.sdata` is a plain `StaticData`. The `Arc` around it was never cloned.
- [x] Closed without a change: caching `mission_spread`. The review measured 6.9 ms across all
      1,239 agent systems, a few microseconds each, so a cache would save microseconds per
      redraw.
- [x] Checked in a pseudo-terminal at 120x40 against temporary homes: a one-corporation run against
      a copied cache drew its results, and `e` then Enter wrote the export, which needs the report
      the run handed over; with no static data, Enter in the dialog showed the download, then the
      filters dialog, and wrote the four files. Both sessions exited 0.
- [x] Tests: 2 added, for a finished task being read once with a panic reading as an error and a
      running one left alone, and for the editing keys.

#### Small correctness items

- [x] An item nobody sells where the run prices is `PricingError::NoSellOrders`, not an
      `EsiError::Esi` with a made-up status 404, since no request failed. `OfferError` carries a
      `PricingError`, whose `Esi` case writes an ESI error exactly as before. The text the CLI,
      the log, and the JSON export show lost its prefix: `no sell orders found for type_id 87618
      in region 10000002` where it read `ESI error (status 404): no sell orders found ...`.
      Checked by output: `--lpstores` over all 283 corporations, before and after against one
      warm copied cache, differs in exactly the 55 offer-error lines, each by that prefix.
- [x] `report::region_id` is the one rule for a region given by ID or name: a number is taken as
      an ID without checking, a name has to be in the static data, and the error names the field.
      `--region`, `--agent-region`, and `--example` each had their own copy. `--example --region
      Nowhere` still prints `Region 'Nowhere' not found` and exits 1, checked before and after.
- [x] A successful response that reports no errors left in ESI's window keeps its body, where it
      used to be thrown away and returned as a rate-limit error. Every request not sent yet is
      held until the window resets, the `x-esi-error-limit-reset` seconds plus one, since the
      header is whole seconds; a request that was waiting for a permit checks again once it has
      one. A held client logs `[ERROR LIMIT] ... holding requests for Ns` once per hold. An error
      response at the limit is still reported as the limit. Before, the other requests kept going
      out into 420 responses and retried them.
- [x] Tests: 4 added. Three run against a local HTTP server started by the test, so no network is
      needed: a success at the limit keeps its body and the next request arrives at least 1.9 s
      later, which fails at 1.2 ms with the hold removed; an error at the limit reads as the
      limit; and nothing is held while errors remain. The fourth covers `region_id`. The offer
      error fixtures in the export and the view tests use the new case, and the view's test now
      pins the logged text.

#### The pricing schedule

- [x] Observed: while pricing, the status line could sit at one request in flight for minutes.
      Checked against the code and two cold runs. `run_children` decided spawn or inline once per
      input, when a group started, and the inputs that got no permit then ran one at a time on the
      group's own permit for the rest of the phase; a permit freed later was never offered to
      them. While groups were still waiting to start, the pricing loop's pending `acquire_owned`
      also took every freed permit, since tokio 1.53.2 gives a released permit to a queued waiter
      before `try_acquire` can see it (`add_permits_locked`). So each group ran its offers almost
      entirely in sequence, and the largest group, 127 offers for CBD Corporation, was left
      running alone at the end.
- [x] Measured on cold runs (`--cache-minutes 0`, copied cache, full ESI budget): at the default
      of 8 tasks, pricing took 317.0 s, 65% of seconds had one request in flight, and the last
      210 s never had more than one. `--concurrency 80` took 256.7 s with a 164 s tail of the
      same kind, since 71 groups took 71 of its permits at once. A first run at 80 looked twice as
      fast, but started inside the 15-minute window the previous run had nearly emptied, and lost
      766 offers to HTTP 429; see *ESI rate limiting* below.
- [x] `run_children` now keeps its inputs in a queue and, before each one it runs itself, hands
      every permit free at that moment to an input still waiting. It still never waits for a
      permit, so a parent can always make progress.
- [x] Groups are priced largest first, by offer count with ties to the lower first corporation
      ID (`pricing_order`), so the longest lists start early rather than being what is left at
      the end. The report is sorted by ISK per LP afterwards, so the order is not visible: a fully
      cached `--lpstores` run printed the same 2,480 lines before and after, apart from timings.
- [x] Measured the same way at the same default of 8: pricing took 144.0 s instead of 317.0 s,
      81% of seconds had four requests in flight, which is `--max-inflight`'s cap, and no stretch
      at the end had one or fewer. Same 50 offer errors, all no sell orders, and 3,504 cache writes
      against 3,518, so the same work was done. The cap of four is now what bounds a cold run.
- [x] Tests: 4 added. Three for `run_children`: with no free permit every input runs in turn,
      free permits at the start are used, and permits released partway through are taken up,
      which fails with spawning limited to the start (one running at a time against four). One for
      `pricing_order`.

#### Tasks instead of concurrency

- [x] `--concurrency` is `--tasks`, and its default is a fixed 64 instead of twice the tokio
      worker threads, which was 8 on this four-core machine. The pool it sets bounds every fetch
      and pricing task together: corporation fetches, groups, offers, and each offer's lookups.
      The help used to say "corporation groups priced at once", which described only one of
      those; it now says "Fetch and pricing tasks that run at once", and that ESI requests are
      capped separately by `--max-inflight`. 64 lets all 71 groups of a full run start at once
      bar seven.
- [x] Corrected: this entry and the `DEFAULT_TASKS` comment first said 64 left "the rest of the
      pool for their offers". It does not. The first 64 groups take every slot; while groups 65
      to 71 wait, their queued acquire gets each freed slot first, and offers get spare slots
      only after that, as groups finish.
- [x] Why the pool is bounded at all, recorded on 2026-10-12. Throughput is set by the slowest
      stage, the four ESI requests in flight, and the pool only has to keep work ready for them;
      at 8, 81% of seconds were at that cap and pricing took 144.0 s, at 64 it took 127.7 s and
      128.9 s, on different binaries and starting budgets. Nothing was measured at 16 or 32, so
      64 is shown to be enough, not to be needed. The bound is not there against starvation:
      without it every request would queue on the client's request semaphore, and tokio's
      `Semaphore` hands out permits in the order they were asked for, as its `Mutex`, the
      low-budget gate, does too, so no request would wait forever. What it does bound, by
      inference rather than measurement, is the work in progress, the memory it holds as the
      input grows, and the backlog behind a 429 hold.
- [x] `--concurrency` is not kept as an alias, as chosen, so it is now refused as an unknown
      argument. The renamed field runs through the option table: `Field::Tasks`, and `tasks` in
      `LpStoreOptions`, `Parsed`, and `Resolved`.
- [x] With a fixed default, `resolve` no longer reads the tokio runtime's worker count, so it no
      longer has to be called inside a runtime. The five tests that started one for that are plain
      tests now.
- [x] Tests: 1 added, for `--concurrency` being refused. The option table's tests pick up the
      renamed flag on their own, including its line in the help and the usage line.

#### ESI rate limiting

- [x] ESI now limits most routes with a token bucket per rate-limit group over a 15-minute
      sliding window, reported in `X-Ratelimit-Group`, `-Limit`, and `-Remaining`, and answers
      429 with `Retry-After` once the bucket is spent; a successful request costs two tokens.
      Market orders are the `market-order` group at `12000/15m`, checked with one request; the
      loyalty route still reports the older error limit instead. A cold full run spends about
      7,000 of the 12,000, so a second one inside the window runs out. The client retried only
      420 and 5xx, so every 429 was a lost offer, and the status line showed only the error limit,
      written as out of 200 where ESI documents 100 non-2xx/3xx responses a minute.
- [x] A 429 with `Retry-After` holds every request not yet sent for that long plus one second,
      logs `[RATE LIMIT] ESI answered 429 on <path>` once per hold, and sends the request again,
      up to ten times. A 429 without the header, which ESI says comes from the game server with
      no rate-limit headers, backs off that request alone. This shares the hold the error limit
      already used.
- [x] Below a tenth of a group's budget its requests are spaced at the window's average rate, two
      tokens per request, 150 ms apart for `market-order`, and only one of them is in flight at a
      time: each takes the group's gate until its answer is in, so the end of a hold releases one
      request rather than every one waiting. A request that was already waiting for a permit when
      the budget turned low goes round again for the gate. Pacing is logged once per client and
      group.
- [x] The status line reads `Budget: 4,940/12,000 per 15m | Error limit: 100 left, resets in
      50s`, the group with the least left and the older limit, each once reported. The footer
      puts the error count before them, so a narrow terminal cuts the limits rather than the
      errors. Running out of 420 or 5xx retries now says which status it ended on: `Max retries
      exceeded (last status 503)`.
- [x] Measured on back-to-back cold runs with `--tasks 64`, each on a copied cache. With the
      hold, the retry, and the pacing but before the gate, the first run priced in 128.9 s and the
      second, started straight after, lost no offer to 429, against 766 before this work, but
      took 21.5 minutes and lost 77 to `Max retries exceeded`. The status line's error count went
      from 0 to 95 between 13:06:13 and 13:07:33, near the end of the run and about six minutes
      after the last 429, while the budget was rising. Their status is not known, since
      the error did not say. The rising budget fits 5xx, which costs no tokens by ESI's
      documentation, where a 4xx costs five; that is an inference. With the gate, a second pair:
      127.7 s for the first run, 524.4 s for the second, no offer lost to anything but no sell
      orders, and no 429 at all; the budget stayed between 864 and 4,168 and was below a tenth
      for 412 s.
- [x] Corrected after the first pair. Pacing at the window's average rate was meant to keep a
      long session from reaching 429s, and did not: tokens come back 15 minutes after they were
      spent, so after a burst there is little to pace against, and the budget reached 0. And the
      burst after a hold was overstated at first as every waiting request at once; the pacing
      already spaced them 150 ms apart, so it was more likely one or two, plus up to four in
      flight, which is an inference, since the log does not show it. Not understood: the second
      pair's budget held up better than a strict 15-minute return predicts, so the difference
      between the pairs is not attributed to the gate.
- [x] Tests: 8 added. Five against local HTTP servers: a 429 with `Retry-After` held and then
      retried, a 429 without it backing off alone, a low budget spacing requests, a budget above
      a tenth left alone, and one request in flight at a time while low. Three plain: reading
      `12000/15m`, the status text, and the retry error naming its status. Mutation checks: with
      the 429 branch removed both 429 tests fail, with the pacing removed the spacing test fails,
      and with the gate removed the one-in-flight test fails.

#### The task pool's type

- [x] Observed: the `--tasks` pool was a bare `Arc<Semaphore>`, passed from the pipeline through
      `LpStore` to `run_children` and `price_offer`. Anything holding it could wait on it, close
      it, or add permits, and `run_children`'s need for its caller to hold a permit was a doc
      comment. The hazard that matters is a waiting acquire below the top level: every slot can
      be held by a parent waiting for a slot for its children, with nothing left to free one. The
      ESI client's `request_permits` had the same type for a different budget.
- [x] `TaskPool` in `tasks` is not `Clone` and has `new` and `spawn(&mut JoinSet, FnOnce(Slot))`,
      the only call that waits for a slot. `Slot` is a handle only that module makes, and
      `run_children` is now `Slot::run_children(inputs, Fn(Slot, I))`, which never waits. In both,
      the task that runs the work holds the permit, not the `Slot`, so a task written
      `move |_slot| async move { .. }` keeps its slot after dropping the handle. The plan had the
      `Slot` hold the permit, which would have freed the slot before such a task's work ran.
- [x] `LpStore` is `pub(crate)`, keeps only the client, and is no longer re-exported from
      `lib.rs`; `offer_results` takes the group's `&Slot`. This removes `LpStore` and
      `LpStore::new(client, Arc<Semaphore>)` from the library's public API, which nothing outside
      the crate used here. `OfferResult` and `OfferError` stay public.
- [x] What the types enforce: only the pool's owner, the pipeline's top level, can wait for a
      slot; a task cannot reach the pool, since it is not `Clone` and tasks are `'static`, so it
      would need an `Arc` around it that review would see; `run_children` needs a `Slot`; and the
      raw semaphore API is unreachable outside `tasks`. What they do not: a `Slot` moved into a
      bare `tokio::spawn` runs that work without a slot. No test shows the compile-time part,
      since a `compile_fail` doctest cannot reach the private `tasks` module and would fail for
      that reason instead.
- [x] `request_permits` is left as it is, as chosen: private to `EsiClient`, with one use.
- [x] Checked on a fully cached `--lpstores` run: the same 2,468 lines as before, apart from
      timings. No cold run, as chosen, since the scheduling logic moved without changing.
- [x] Tests: 2 added, a task keeping its slot after dropping the handle, and three nested levels
      staying within four slots; the 3 existing ones now go through `TaskPool::spawn`. Mutation
      checks: freeing the slot before the work runs fails all five, spawning children without a
      slot fails four, and spawning only before the first input fails the freed-slots test.

#### Pricing each offer once

- [x] Observed: each group was priced on its own, so an offer several groups hold was priced once
      per group. The repeats came from the cache, and every cache hit reads and parses the file:
      the client keeps no response bodies in memory, and its in-flight map only shares requests
      running at the same moment. A fully cached run made 24,233 cache hits pricing 71 groups.
- [x] Corporations whose offers are identical still share a store, but each distinct offer is now
      priced once, as its own task through `TaskPool::spawn`, with its reward and required items
      as its children. The results stay in memory as `Arc<OfferResult>`, and each store ranks its
      offers from them. `Report` holds the stores, the errors, and the groups `Report::regroup`
      builds for a depth, for now one per store. The view regroups from memory on a depth change,
      and when a report arrives, since its depth may have changed while the run went on.
- [x] Ranking: ISK per LP, best first, then offers without a value, with ties to the lower offer
      ID. Ties used to keep the order of the first corporation's list from ESI. A total order is
      what lets stores holding the same offers rank them alike, which top-n grouping needs.
- [x] Errors once per offer, as chosen: the CLI prints them before the groups, the view logs each
      once, and the JSON export lists them after the groups with the corporations whose stores
      hold the offer; groups no longer carry `errors`. A pricing task that panics is an offer
      error now, `pricing task failed: ..`, found by task ID through `join_next_with_id`, as
      chosen; a panicked group used to drop the whole group. The `GroupFailed` event and the
      group-failure count are gone.
- [x] Progress counts offers: `Pricing 3222 offers...`, `Priced 3222 offers in ..`, and in the
      view `Pricing offers: x / y` and `Finished: N offers priced, G groups`. Largest groups first
      (`pricing_order`) is removed, since the tasks are offers. The `DEFAULT_TASKS` comment said
      the first 64 groups take every slot; it now says offers do.
- [x] Measured fully cached, on one warm copy of the cache: before, 71 groups, 24,233 cache hits,
      pricing 1.77 s; after, 3,222 offers, 5,823 cache hits, 0.40 s. Reading the cache files
      beforehand had counted 3,074 distinct offers; the difference was not traced. The same 71
      groups print in the same order with the same corporations, offer counts, and ISK per LP
      sequence. Offer order differs only within ties, in 3 groups: two whose ten listed offers
      have no value, and one with three pairs of equal values; in 2 of them the tie at the end
      crosses the top-10 cut, so different tied offers are listed. The 50 error lines were 11
      distinct errors, and the 11 lines now are the same 11. No cold run, as chosen.
- [x] Tests: 5 added, for each offer priced once in the order first listed, the ranking's
      tie-break, an offer counted once with every corporation that holds it, a report grouped for
      the view's depth on arrival, and a failed join naming the ID `spawn` returned. The fixtures
      build stores through `Report::new` and `Store::new`.

#### Grouping by the listed offers

- [x] Observed: corporations were one group only when their whole stores matched, so groups
      listing the same top offers printed the same lines more than once.
- [x] `Report::regroup(depth)` joins the stores whose top `depth` offers are the same, in the same
      order; the ranking is a total order, so the same offers always come in the same order. A
      store with fewer offers than the depth joins only stores listing exactly its offers, and
      stores listing nothing are one group. A group lists only the offers its stores share.
      Corporations are listed in the run's order, which `Report` keeps, since joined stores can
      interleave in it: stores of 1 and 5 and of 3 list as 1, 3, 5. The view regroups on every
      depth change and keeps the group holding the selected group's first corporation selected.
- [x] Offer counts as a range, as chosen: `Loyalty offers: 319–322 found` in the CLI and
      `| 319–322 offers` in the groups pane, one number when equal. JSON groups have
      `min_offer_count` and `max_offer_count`, and every corporation entry, in groups and in
      errors, its own `offer_count`; CSV's `offer_count` column is now `min_offer_count` and
      `max_offer_count`, so it has 21 columns. The help for `--depth` says how groups are formed,
      and `--max-inflight` says "across the whole run" instead of "across all groups".
- [x] Observed and left as it is: stores with no offers join a store whose three offers all
      failed to price, since neither lists anything, and the group reads `Loyalty offers: 0–3
      found` over no offers.
- [x] Measured fully cached, on the same warm copy, at depths 5, 10, and 20: 71 groups before at
      every depth; 40, 42, and 51 after, of which 14, 13, and 16 show a range. Checked against an
      oracle built from the old binary's output by joining the groups whose printed top offers
      match as sets: the same number of groups at each depth, and the matching groups in the same
      order. The rest, 2, 2, and 5, each have the same corporations and ISK per LP sequence as one
      new group, and differ only in which offers without a value are listed at the cut, which the
      tie-break by offer ID now decides.
- [x] Tests: 7 added, for stores listing the same top offers joining with a range, a new depth
      joining and splitting groups, a store with fewer offers than the depth, stores listing
      nothing, a joined group in the run's order, the view keeping the selected corporation through
      a join and a split, and the export's fewest and most offers. One test changed: its two empty
      stores are now one group, so each holds one offer without a value instead. Keying the join
      on the whole ranked list fails 4 of them.

#### Required-item prices kept for the run

- [x] Observed: a reward lookup and a required-item lookup read the same market orders URL, but a
      required item is the same lookup for every offer that needs it, and many offers share a few
      items. Each was a cache file read after the first.
- [x] `ItemPrices` keeps each required item type's best sell price for one run, in the run's
      region and stations, which it holds so it cannot be asked for another market. An answer is
      kept, including that nothing sells the type; a failed request is not, so the next offer
      that needs the type asks again. Offers asking at the same moment share one lookup, through
      a `OnceCell` per type. It holds one number per type, so it costs next to no memory; reward
      lookups keep both sides of the book and are not kept, as offered.
- [x] Measured fully cached, three runs each on one warm copy: 5,823 cache hits and 0.38 to 0.39 s
      of pricing before, 3,755 and 0.17 to 0.18 s after. The 1,629 printed lines are the same
      apart from timings and hit counts.
- [x] Tests: 3 added, for one lookup per type including an answer that nothing sells it, offers
      asking at once sharing a lookup, and a failed lookup not being kept. With the memo bypassed
      the first two fail.

#### Publishing on GitHub

- [x] The location is `colsen665/eve_lpstore`, in `Cargo.toml` line 7 and `README.md` lines 23 and
      35; the User-Agent reads the URL from `Cargo.toml`.
- [x] Created the public repository on GitHub and pushed with `git push -u origin master`.
      `origin` is GitHub, the only remote.
- [x] The first CI run failed `paths::tests::neither_root_is_the_working_directory` on Windows,
      where the roots do end in `cache` and `data`. The test no longer checks their names. cargo
      stopped at that failure, so the binary's own tests had not yet run on Windows.
- [x] The second run failed two tests that treated `/no/such/directory` as unwritable. On Windows
      `save_to` could create it, after which the export could write there. Both now write under a
      regular file instead, which no platform or user can do.
- [x] The third CI run passed on Linux, Windows, and macOS, at `4a0e424`.
- [x] Tagged `v0.1.0` and the release workflow passed. The checksums of both downloaded archives
      verify. The Linux binary prints `eve_lpstore 0.1.0` and is static. The Windows binary has not
      been run, since CI builds it and only the tests run on Windows.
- [x] History rewritten before the first push: every commit is by
      `colsen665 <colsen665@proton.me>`, and the early commits are squashed into the initial one.
- [x] `cargo fmt` run once over the tree, alone in its commit: 11 hunks in 6 files that predated
      it. Stable rustfmt 1.10.0 and the nightly one agree, so CI can check `cargo fmt --check`.
- [x] The User-Agent was a hard-coded `eve_lpstore/0.1.0` with no contact, which ESI asks for, and
      the static data download sent none at all. Both now send `DEFAULT_USER_AGENT`,
      `eve_lpstore/<version> (+<repository>)`, built from `Cargo.toml`. A test reads the header
      off a request to a local server. The download is not covered by a test, since it fetches
      CCP's whole archive.
- [x] `--version` prints `eve_lpstore <version>` and exits, after `--help` if both are given.
- [x] Licensed `MIT OR Apache-2.0`, as chosen. `LICENSE-MIT` names colsen665, the git
      author, for 2026; `LICENSE-APACHE` is the text from apache.org, 202 lines. `Cargo.toml` has
      a description, the license, the README, keywords, and categories.
- [x] `README.md` says what the program does, how to install it from a release or with
      `cargo install`, how to run it, which files it keeps where, how it uses ESI, how to test and
      release it, and the license. It ends with the notice section 7.1 of CCP's Developer License
      Agreement asks for, quoted from developers.eveonline.com/license-agreement. The paths are the
      Linux ones `paths.rs` documents; the Windows and macOS ones are left to `--verbose` rather
      than written down unchecked.
- [x] CI, `.github/workflows/ci.yml`, on every branch push and pull request: one job runs
      `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`, and
      `cargo test --locked` runs on Linux, Windows, and macOS, as chosen, all on stable. Locally,
      stable 1.99 passes all three.
- [x] Releases, `.github/workflows/release.yml`, on a `v*` tag push: the tag is checked against
      the version in `Cargo.toml`; `x86_64-unknown-linux-musl` and `x86_64-pc-windows-msvc` are
      built, as chosen, and packaged with the README and both licenses as
      `eve_lpstore-<tag>-<target>.tar.gz` and `.zip`; and `gh release create` publishes them with
      `SHA256SUMS` and generated notes. The actions are the majors current on 2026-10-12:
      checkout v7, upload-artifact v7, download-artifact v8, rust-cache v2.
- [x] Checked locally: both workflow files parse; the tag check accepts `v0.1.0` and refuses
      `v0.2.0` with the two versions named; and the packaging and checksum steps, run as written
      against a native `x86_64-unknown-linux-gnu` build, give an archive of the four files whose
      checksum verifies and whose binary prints `eve_lpstore 0.1.0`.
- [x] Not checkable locally, and since checked by the first push and tag: the musl build, which
      needs a musl C compiler this machine lacks, the Windows build, the tests on Windows and macOS,
      and every GitHub-side step.

#### A single-thread runtime

- [x] `--single-thread`, the name chosen over `--green`: tokio tasks are green threads on either
      runtime, and what the flag changes is that the current-thread runtime runs all of them on
      the thread that calls `block_on`, instead of on a worker per CPU core. `main` is a plain
      function now, since the runtime has to be chosen before it exists: it answers `--help` and
      `--version` without one, then builds the runtime, multi-thread with `enable_all` as
      `#[tokio::main]` did, or current-thread with the flag. The flag is in `FLAGS`, the help, and
      the usage line.
- [x] Nothing else had to change, by reading the code: no `block_in_place`, which panics on a
      current-thread runtime; no nested runtime and no read of the worker count; no lock held
      across an `.await`, which clippy's `await_holding_lock` would report; and `#[tokio::test]`
      already runs the async tests on a current-thread runtime. The view's loop runs in
      `spawn_blocking` and reads finished tasks with `Handle::block_on`, which tokio documents as
      unable to drive I/O or timers on a current-thread runtime unless another thread is inside
      `Runtime::block_on`; the main thread is, and a finished task needs neither.
- [x] Measured fully cached, three runs each way on one warm copy, on this four-core machine:
      pricing took 0.17 to 0.19 s by default and 0.37 to 0.38 s with `--single-thread`, and the
      whole run 0.43 to 0.49 s against 0.72 to 0.74 s. The 1,631 printed lines are the same. The
      most threads seen, sampling every 5 ms, were 49 and 31: by inference mostly tokio's
      blocking pool, which serves the cache file reads and grows with how many run at once. A cold
      run is bound by ESI's four requests in flight and was not measured. The view had no live
      terminal check.
- [x] Tests: 2 added, for the flag picking the runtime and being documented, and for a finished
      task being read on a current-thread runtime the way the view reads it. Running that loop on
      the runtime's own thread instead of a blocking one makes the second fail: the task never
      runs.

#### Popups over the footer

- [x] Observed while adding figures to the help: the footer was drawn after every popup, so a
      popup reaching the bottom three rows lost them. At 80x24 the donation popup's QR code does,
      and its address, the line saying how to close it, and its bottom border were drawn over.
      The footer is drawn before the popups now. A test of the whole view at 80x24 with the
      donation popup open, which fails with the old order, checks both lines show.

#### Figures in the help

- [x] The help, `?`, lists under "Figures" what each column of the offers table is and how it is
      worked out, in the wording chosen: the formula and what its colour means, two lines at
      most. ISK/LP is (reward × units − ISK cost − items) ÷ LP cost per redemption, with the
      reward at the best buy order and the items at the best sell orders. Spread is (best ask −
      best bid) ÷ best bid, green to 10%, yellow to 50%, red past it, magenta when inverted.
      Imbal is (bid − ask volume) ÷ their sum within 10% of each best price, green from +25% and
      red from −25%. Vol ±10% is the units buy orders take at 90% or more of the best bid, and
      Profit ±10% is (price − unit cost) × units over them, with unit cost (ISK cost + items) ÷
      units. Two more lines say what n/a means and that prices come from the Market region, or
      only the Market system's stations. Each was checked against `lpstore` and the colour
      bands in `offers.rs`.
- [x] The column titles stay as they are, as chosen, although Vol ±10% and Profit ±10% count only
      buy orders from 90% to 100% of the best bid; the help says so.
- [x] The help scrolls, as chosen. It is 80 columns wide, so a formula fits on one 78-column
      line, and 24 lines long, so at 80x24 it shows 22 and the title says Up/Down scrolls. The
      arrows and the page keys scroll it within its length, as they do the details pane, and it
      opens at the top. The Up/Down key line says it scrolls the help too.
- [x] Tests: 3 added, for every figure line showing whole on a large screen, the window at 80x24
      at the top and at the end, with whole rows and no footer showing through, and the keys
      scrolling it within bounds and reopening it at the top. Drawing the footer last again
      fails the 80x24 one.

## Done

- [x] Shared errors keep their structure (`SharedError`), so `RateLimit`, `Throttle`, and `Esi` fields survive request sharing.
- [x] Run `--lpstores` across all corporations end to end and check the output and runtime.
- [x] Make `check_cache` use `tokio::fs` (or `spawn_blocking`) instead of blocking `std::fs` calls inside an async fn.
- [x] Separate parallelization from the number of in-flight ESI requests:
  - [x] Parallelization (groups priced at once, `--concurrency`) defaults to 2x the number of tokio runtime worker threads. Superseded on 2026-10-12: the flag is `--tasks`, with a fixed default of 64; see *Tasks instead of concurrency*.
  - [x] In-flight ESI requests default to 4, set by a separate command line argument (`--max-inflight`).
  - [x] Throttle in-flight requests with an awaitable tokio primitive, such as a `tokio::sync::Semaphore`, if it fits.
- [x] Corporation and type name lookups from static data.
- [x] `--lpstore` accepts a comma-separated list of IDs or names.
- [x] `--region` for `--example` and `--lpstores`.
- [x] ISK per LP, profit, and volume within 10% of best bid.
- [x] Groups with identical offers are priced once.
- [x] `--concurrency` with `JoinSet`.
- [x] Atomic cache writes and request deduplication.
- [x] Unknown arguments are rejected.
- [x] `--help` with examples.
- [x] Keep looking for page 1 when a sibling cache file is listed first.
- [x] Compute ISK per LP for the whole redemption. The reward is per unit, so it is multiplied by the offer quantity against the redemption's total ISK; multi-unit offers were undervalued before.
- [x] `d` donation overlay in `--tui`, showing the address as a QR code from `src/tui/qr.rs`. Esc or `d` closes it, `q` still quits from it, and it falls back to the address alone when the code does not fit.
- [x] Widen the offer table columns. The volume and within-10% figures show only when there is room, and offers without a reward show `n/a`.
- [x] Sort groups by the average ISK per LP over the top `--depth` offers, instead of by the single best offer. Offers without a value are left out of the average, and ties break on the group's first corp ID.
- [x] Remove the LP column from the Offers pane. LP cost stays in the detail pane, and the freed width made room for the spread column.
- [x] Bid/ask spread on the reward, as a percentage of the best bid, in a coloured Offers pane column and in the CLI's `spread=`. Green up to 10%, yellow to 50%, red past that, magenta when the market is inverted, and dim when nothing sells the reward. Capped at `>999%` to hold the column width.
- [x] Bid/ask imbalance within 10% of the touch, in a coloured wide-layout column and in the CLI's `imbalance=`. Green at +25% and above, red at -25% and below, yellow closer to even, and dim when either side is empty. `WIDE_OFFERS` rose from 66 to 73 to fit it.
- [x] Both read the ask side of a response the reward lookup already fetched and previously discarded, so neither adds an ESI request: a full run priced 690 offers from 24,233 cache hits with no misses and no writes. Spread and imbalance come back `n/a` on exactly the same 30 offers, since the bid depth reuses the within-10% volume.
- [x] Fix the offers table cutting off its last column, found by the first results-view render test. Both layouts truncated `Profit ±10%` at their own minimum widths. Narrow was measurably broken at the documented 80-column minimum, and that predates the imbalance work, which is wide-only and never touched the narrow columns. Wide was broken at its threshold too, and `WIDE_OFFERS` had only ever been set by arithmetic. Fixed by dropping narrow's `Item` floor from `Min(12)` to `Min(10)`, the only column that can shrink, and by measuring `WIDE_OFFERS` at 73: a 72-cell pane cuts the header and a 73-cell pane does not.
- [x] Drop the `columns_needed` width arithmetic and its two tests. It budgeted four cells for the borders and the `"> "` highlight, but ratatui's table needs about nine, so both fit tests passed while the real layout truncated. A render sweep over every width from 80 to 160 now checks the last column instead, which is what caught the bug.

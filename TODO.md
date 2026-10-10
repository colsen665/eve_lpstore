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
- [x] Show a "terminal too small" message below 80x20.

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
- [x] One behaviour that is not a bug: a file containing `[]` loads as every field absent rather than being rejected, because serde's derived deserializer accepts the sequence form. That is the same run the file being absent would give, so there is nothing to report.

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

Burners, which are deferred, are said to reach the current or a neighbouring constellation.

The same weights apply for a second grouping by **distance from the agent system in jumps**: 0 jumps is
the agent's own system, 1 jump the systems one gate away, and so on. Both groupings are marginals of one
uniform distribution over the constellation's systems, so a system contributes its `1/n` to whichever
band and whichever distance it belongs to. The distance in the details pane is unambiguous per row,
because each listed system is itself an agent system and the counting starts there.

Worked through the four-system example, which shows why the two groupings behave differently. The
marginal **by the systems' own security** is `0.75 / 0.25` in every case, because it counts systems per
band and ignores the graph. The grouping `bands` reports is by the security flown on the route, and
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
- [ ] Whether a destination is a solar system at all is unresolved, and the earlier claim about space
      systems stays withdrawn. The gate data now argues against the separate-system reading. Of 8,490 systems, 5,268 have at
      least one stargate and 3,222 have none: 2,604 in the 310xxxxx range, which is wormhole space and
      expected to have no gates, 400 in the 320xxxxx and 340xxxxx ranges, and 217 in the 300xxxxx
      range. No constellation mixes gated and gate-less systems, the 418 all-gate-less constellations
      being wholly separate, so there is no pocket system sitting in Kimotoro beside Jita, which is the
      shape a `Jita Space` destination would need. Two sources agree no such system exists: the cached
      `mapSolarSystems` rows, and ESI's `POST /universe/ids`, which resolves the name only as a
      character. The claim that the data "excludes space systems" stays withdrawn, since 14 of the 15
      probed names may not be real.

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
- [ ] Where it appears. Options are an overlay on its own key, lines under each system in the details
      pane, a compact suffix on the corporation line, or a column in the groups pane. The details pane
      is already the tallest region of the view and holds the offer breakdown, which argues against
      adding rows there.
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
      security, and `destinations` carries both bands per destination so that a display can show the
      difference rather than only the collapsed one.
      Sazdih from Bayuka is that case: `0.4/100%` as the grouping and `own 0.5` on Moh.
- [ ] What the model assumes without evidence: that destinations stay inside the constellation, that
      every system in it is equally likely, and that the agent's own system carries the same weight as
      a neighbour. That is a stated model rather than a measurement of how missions are routed, and
      whatever displays it will have to say so; the wording waits on the research below rather than
      being guessed at now.
Deferred to a later phase:

- [ ] Burner and anomic variants. These are acquired from a level 4 basic security agent, the same
      agent that offers the ordinary mission, so excluding them is a rule about the mission variant and
      not about the agent: 502 such agents across 125 corporations sit in 457 systems. A system
      hosting one has two destination sets, and a single probability per destination would mix them.
- [ ] Whether burner destinations can be named now hinges on the question above rather than on the
      withdrawn probe. What is measured is that the non-known-space systems are in the set, and the
      2,604 wormhole systems among them carry no stargate at all, so under a gate-only model they
      cannot be a destination however they are named. If burner destinations turn out to be sites
      rather than systems, this item goes away with the model instead of needing data.
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
- [ ] No live terminal check is claimed for any of it, per the standing note on such checks, and none
      is possible yet: the grouping and the routing are pinned by tests against the four-system
      example and against Sazdih measured from ESI, but there is no display until the option above is
      taken.
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

## Done

- [x] Shared errors keep their structure (`SharedError`), so `RateLimit`, `Throttle`, and `Esi` fields survive request sharing.
- [x] Run `--lpstores` across all corporations end to end and check the output and runtime.
- [x] Make `check_cache` use `tokio::fs` (or `spawn_blocking`) instead of blocking `std::fs` calls inside an async fn.
- [x] Separate parallelization from the number of in-flight ESI requests:
  - [x] Parallelization (groups priced at once, `--concurrency`) defaults to 2x the number of tokio runtime worker threads.
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

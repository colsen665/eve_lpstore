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

- [x] Add `ratatui = "0.30"` and `crossterm = "0.29"`. ratatui 0.30 needs Rust 1.86 or newer. Pin crossterm to 0.29 so only one copy is built. Cargo needs network access to fetch them.
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
- [ ] Check the focus-dependent note in a real terminal. The first attempt proved nothing: the keys did not move focus, so what looked like the syntax line was the always-visible inline hint of some other row. The `TestBackend` tests set `focus` directly and pass, so the behaviour is covered; only the live check is outstanding.

##### Remembering and validating the pair

- [ ] On apply, clear `Market system` when `Market region` changed. Compare resolved region ids rather than typed text, so `Domain` and `10000043` are the same region. Clearing has to happen before `resolve`, or the run keeps using the old system's stations, and the log has to say the field was cleared rather than dropping what the user typed in silence.
- [ ] Leave `Market system` alone when the region did not change.
- [ ] Add the system to region mapping to the static data. `AgentCache.systems` is `(id, name, security)` and `stations` is `(station, system)`, so nothing today reaches a region. Confirm the SDE file name and the field inside it first: extraction matches names loosely, `contains("region")` and `contains("corporation")`, so a fifth file needs a check that it picked up the right one.
- [ ] Count the new file in `is_cached`, so an existing install reads as uncached and the Phase 7 dialog asks before re-downloading instead of failing on a mapping that is not there.
- [ ] Validate a typed `Market system` against the resolved `Market region`, and report a mismatch through `set_errors` on that field, which shows it inline and stops the run.
- [ ] Keep the rule to the dialog. `--region` and `--system` together on the command line behave as they do today, mismatch included.

#### Phase 8: Cache and data in platform directories

Store the ESI response cache and the static data in the platform's standard locations instead of `cache/` and `data/` in the working directory. Both are relative today, so running the binary from anywhere else silently re-downloads the static data and re-fetches every ESI response.

- [ ] Add `directories = "6"`. Rust std cannot do this: `std::env::home_dir` has been deprecated since 1.29 with "consider using a crate from crates.io instead", std has no XDG or Known Folder support, and on Windows the correct source is the Known Folder API rather than `%APPDATA%`, which misses OneDrive redirection and UWP. Version 6.0.0 is MIT or Apache-2.0 with one runtime dependency, `dirs-sys`. Cargo needs network access to fetch it.
- [ ] Resolve both roots once from `ProjectDirs::from("", "", "eve_lpstore")`. An empty qualifier and organization give `~/.cache/eve_lpstore` on Linux, `%LOCALAPPDATA%\eve_lpstore\cache` on Windows, and `~/Library/Caches/eve_lpstore` on macOS. Changing these arguments later orphans the data a second time, so they are fixed here.
- [ ] Put the ESI response cache in `cache_dir()` and the static data in `data_local_dir()`. Both are Local rather than Roaming on Windows, which keeps regenerable data out of domain roaming. The size is worth stating because it is not small: a full pricing run across every corporation was measured at 4,584 response files and 122 MB on 2026-10-09, and the four static data files at 6.1 MB. The two roots are the same directory on Linux and macOS, so this choice only affects Windows.
- [ ] Refuse to start when `ProjectDirs::from` returns `None`, which it does when the operating system reports no valid home directory, and say why. Do not fall back to the working directory: that would keep the relative-path defect this phase removes.
- [ ] Replace the five hardcoded sites: `CACHE_DIR` in `src/static_data/mod.rs` and the four path accessors built from it, the literal in `clear_cache_dir`, the two `.cache_dir("cache")` calls in `src/main.rs` and `src/report/options.rs`, and `FILE_NAME` in `src/saved_filters.rs`. The last one belongs in `config_dir()` rather than in either of the other two roots: it is a user file, and Phase 7.1 put it at the top level until a platform directory existed to move it into.
- [ ] Change `EsiConfig.cache_dir` from `Option<String>` to a path type. Windows locations hold spaces and backslashes, and the builder's `impl Into<String>`, the three `Path::new(cache_dir)` uses, and the `create_dir_all` call in `src/esi/mod.rs` would take a `PathBuf` directly.
- [ ] Add `--cache-dir <path>` to `VALUE_FLAGS`, `HELP_TEXT`, and the usage line, overriding the ESI cache root only; static data keeps its platform location. Carry it through `LpStoreOptions` beside `cache_minutes`, `concurrency`, and `max_inflight`, which are command-line only and absent from the filter form by the same rule.
- [ ] Change `--clear-cache` to delete files rather than the tree. Today it is `remove_dir_all("cache")`. It should remove `*.json` and the `.*.tmp` orphans in the ESI cache root, which is the `--cache-dir` value when given, plus the SDE archive in the static data root if one is there. The four extracted static data files stay, so the next run does not re-download the archive.
- [ ] Match `.*.tmp` as well as `*.json`. The atomic write names its temporary file `.{entry}_{id}_{expiry}.json.{pid}.tmp`, so a json-only pattern leaves it behind after a crash mid-write. The cache is flat today, so a recursive walk is not required.
- [ ] Keep the archive clause defensive. `download_and_cache` reads the response with one `bytes().await` and hands it to `zip::ZipArchive` in memory, so nothing writes an archive to disk and the check never matches today. It costs one `exists()` and stays correct if the archive is ever cached.
- [ ] Report what was removed and from where, with absolute paths. The current message names the directory and has a branch for it not existing; with files deleted instead of the tree the directory survives, so the message should count files rather than claim one was cleared.
- [ ] Leave the existing `cache/` and `data/` where they are and re-download on the next run. No migration: whatever is in them is abandoned, and the first run fetches the SDE archive again and refetches ESI against the rate limit. Say so where the change is announced. How much that costs depends on how warm the directory is, so check before announcing a number: this checkout is a clone from 2026-10-10 that started with neither directory, and its contents are only what its own runs accumulated. A predecessor checkout that had one full pricing run behind it measured 4,581 response files and 122 MB before it was deleted the same day.
- [x] Drop the seven files in `data/` that nothing read: `corporation_agent_details.json`, `corporation_agent_factions.json`, `corporation_agents.json`, `corporation_agent_systems.json`, `corporation_agent_true_sec.json`, `corporation_security.json`, and `hisec_corporations.json`, 6.4 MB together. `load_from_disk` reads four files and only those four have filename constants. Already satisfied, so there is nothing to remove: this checkout is a clone from 2026-10-10 whose `data/` was created by the first run and holds only the four files the code writes. The seven existed only in the predecessor checkout, which this code never wrote them into either, and that checkout was deleted the same day. Kept here because the reason matters — a reader wondering where the other seven came from should know they did not come from this code's download path.
- [ ] Reconcile `.gitignore` with the move: `/data` and `/cache` become unnecessary once nothing writes there, and `/filters.json` becomes unnecessary once `saved_filters` reads its path from `config_dir()`. All three entries are there because the files are per-user state rather than source, which does not change; only their locations do.
- [ ] Check the minimum supported Rust version of `directories` 6.0.0 against this project's floor, which ratatui 0.30 sets at 1.86. `Cargo.toml` declares no `rust-version` of its own.
- [ ] Print the resolved cache and data directories under `--verbose`, and into the TUI log, so the move is visible rather than silent. Printing to stdout unconditionally would scroll away before the TUI takes the terminal.
- [ ] Tests: the resolved roots are absolute and independent of the working directory, cache and data are not the same directory, `--cache-dir` overrides the cache root and not the data root, `--clear-cache` removes `*.json` and `.*.tmp` while leaving the directory and the static data, and the existing `esi` cache test still passes, since it already sets its own directory.

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

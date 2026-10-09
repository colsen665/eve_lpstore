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

- [ ] Add `ratatui = "0.30"` and `crossterm = "0.29"`. ratatui 0.30 needs Rust 1.86 or newer. Pin crossterm to 0.29 so only one copy is built. Cargo needs network access to fetch them.
- [ ] Add `--tui` to `FLAGS`, `HELP_TEXT`, and the usage line. It implies `--lpstores` and is rejected with `--example`. It exits with an error if stdout is not a terminal. Inside the TUI, the status bar replaces `--status`, and `--verbose` output goes to the log panel.
- [ ] Put the TUI in `src/tui/`, declared from `main.rs`. Start it after static data loads and the arguments validate, so existing errors still print to a normal terminal. The CLI filter flags pre-fill the filter form.
- [ ] Run the UI on a blocking thread with `spawn_blocking` around `ratatui::run`. It polls keys with `crossterm::event::poll`, drains progress with `try_recv`, and redraws every 250 ms. The pipeline stays on the async runtime. This fits `ratatui::run`, which takes a sync closure, and it avoids `EventStream` and `futures`.
- [ ] Restore the terminal on quit, on Ctrl-C (raw mode delivers it as a key event), and on panic. Check whether ratatui's init installs a panic hook. If not, add one.

#### Phase 3: Results view

- [ ] Keep app state in `src/tui/app.rs` as plain data with update functions (event in, state out), with no terminal I/O. Rendering goes in `src/tui/ui.rs`.
- [ ] Progress view, shown until the first run finishes: gauges for fetch and pricing, plus recent log lines.
- [ ] Results layout, using the CLI's grouping:
  - [ ] Groups pane on the left: each group's top ISK per LP, offer count, and corp names, best first.
  - [ ] Offers pane on the right: the selected group's top `--depth` offers, with the CLI's columns.
  - [ ] Detail pane: the group's corps with their agent systems, as the CLI prints them, plus the selected offer's breakdown, within-10% volume, and profit.
- [ ] Status bar: in flight, waiting, rate limit, errors, and cache hits, misses, and writes.
- [ ] Log panel (`l`): cache lines under `--verbose`, plus fetch and pricing errors.
- [ ] Keys: arrows or j/k to move, Tab to switch panes, PgUp/PgDn to scroll, `?` for help, and `q` to quit. Ctrl-C also quits. Quitting aborts the current run.
- [ ] Show a "terminal too small" message below a minimum size.

#### Phase 4: Filter editing

- [ ] Filter form (`f`): text fields for corporations, region, security, agent level, division, type, faction, system, and depth, plus an `--expire` toggle. Each field shows its valid values. Tab and Shift+Tab move between fields, Enter applies, and Esc cancels.
- [ ] While the form is open, keys go to the form, so typing `q` enters a letter.
- [ ] Apply validates every field first. If any field fails, its error shows inline and nothing runs.
- [ ] Apply on success cancels the running pipeline and starts a new run with a new run id. Results from the previous run stay visible, marked stale, until the new run finishes.
- [ ] Each run builds its own `EsiClient`, because the default cache offset depends on the number of corps. The status counters therefore reset for each run.
- [ ] Depth changes re-render only. The pipeline uses depth only when printing.
- [ ] Name suggestions for corporations, systems, and regions. Systems need a new `StaticData` accessor, since system names aren't exposed yet. This can be cut from v1 to keep the first version smaller.

#### Phase 5: Tests and docs

- [ ] Unit tests for state transitions: key sequences, progress events, stale run ids, and validation errors.
- [ ] Render tests with `ratatui::backend::TestBackend` at 80x24 and 120x40.
- [ ] Update `HELP_TEXT` with `--tui` examples.
- [ ] Manual check in a real terminal: resize, filter apply during a run, quit, Ctrl-C, and a forced panic.

#### Phase 6: Later

- [ ] Sort toggles for ISK per LP, profit, and volume.
- [ ] Export the current view to CSV or JSON.

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

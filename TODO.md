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

- [x] Unit tests for state transitions: key sequences, progress events, stale run ids, and validation errors. 55 tests live in `src/tui/`: `app.rs` covers keys, progress, stale run ids, and offer errors, `form.rs` covers per-field and general validation errors, `mod.rs` covers run identity and the filter summary, `qr.rs` covers the QR rendering, and `ui.rs` covers text fitting, the offer cells, the spread and imbalance formatting and colours, and full-view renders.
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

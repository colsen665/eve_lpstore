use super::*;
use crate::tui::form::FilterForm;
use crate::tui::test_support::{forge_run, press, priced_offer, report_of, results_app};
use eve_lpstore::report::{Field, LpStoreOptions, OptionError};

#[test]
fn wrapping_keeps_the_indent_on_every_row() {
    // Destination lines are indented under their corporation. Without this the continuation row
    // sat at column 0, which is where a new system would start.
    assert_eq!(
        wrap_text("  Bereye 0.5/12% 0.6/38%", 12),
        ["  Bereye", "  0.5/12%", "  0.6/38%"]
    );
    // A line with no indent is wrapped exactly as before.
    assert_eq!(wrap_text("one two three", 7), ["one two", "three"]);
    // An indent wider than the row still keeps its text rather than vanishing.
    assert_eq!(
        wrap_text("    deep", 3),
        ["    deep"],
        "clipped, not shredded"
    );
}

#[test]
fn wrap_breaks_at_spaces_and_cuts_long_words() {
    assert_eq!(
        wrap_text("Ministry of Assessment (1000081): Turba", 20),
        ["Ministry of", "Assessment", "(1000081): Turba"]
    );
    assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(wrap_text("", 10), [""]);
}

#[test]
fn a_line_break_in_the_source_starts_its_own_row() {
    // The market fields ask for two lines instead of leaving the split to the width.
    assert_eq!(wrap_text("one\ntwo", 20), ["one", "two"]);
    assert_eq!(wrap_text("one\ntwo", 3), ["one", "two"]);
    assert_eq!(wrap_text("a b\nc d", 3), ["a b", "c d"]);
    // A blank line stays a row of its own rather than vanishing.
    assert_eq!(wrap_text("a\n\nb", 10), ["a", "", "b"]);
}

#[test]
fn truncate_ends_a_cut_with_an_ellipsis() {
    assert_eq!(truncate("abcdef", 4), "abc…");
    assert_eq!(truncate("abc", 4), "abc");
}

#[test]
fn short_isk_uses_one_suffix_and_two_decimals() {
    assert_eq!(short_isk(3_980_000_000.0), "3.98B");
    assert_eq!(short_isk(1_234_000.0), "1.23M");
    assert_eq!(short_isk(5_000.0), "5.00K");
    assert_eq!(short_isk(512.0), "512.00");
    assert_eq!(short_isk(-2_500_000.0), "-2.50M");
}

#[test]
fn wide_offer_rows_show_the_volume_and_the_profit_within_10_percent() {
    let result = priced_offer(Some(12.5), Some(50.0), Some((2_474_600, 3_980_000_000.0)));
    let cells: Vec<String> =
        offer_cells(0, &result, "Sisters Combat Scanner Probe".to_string(), true)
            .into_iter()
            .map(|(text, _)| text)
            .collect();
    assert_eq!(cells.len(), 7);
    assert_eq!(cells[3], "12.50%");
    assert_eq!(cells[4], "+50%");
    assert_eq!(cells[5], "2,474,600");
    assert_eq!(cells[6], "3.98B");
}

#[test]
fn narrow_offer_rows_leave_out_the_imbalance_and_the_volume() {
    let result = priced_offer(Some(12.5), Some(50.0), Some((2_474_600, 3_980_000_000.0)));
    let cells: Vec<String> = offer_cells(0, &result, "Probe".to_string(), false)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(cells.len(), 5);
    assert_eq!(cells[3], "12.50%");
    // The profit, not the imbalance, is what follows the spread in a narrow pane.
    assert_eq!(cells[4], "3.98B");
}

#[test]
fn offers_without_a_reward_show_n_a() {
    let result = priced_offer(Some(12.5), Some(50.0), None);
    let cells: Vec<String> = offer_cells(0, &result, "Probe".to_string(), true)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(cells[3], "12.50%");
    assert_eq!(cells[4], "+50%");
    assert_eq!(cells[5], "n/a");
    assert_eq!(cells[6], "n/a");
}

#[test]
fn the_spread_column_shows_n_a_without_an_ask() {
    let result = priced_offer(None, None, Some((100, 1000.0)));
    let cells: Vec<String> = offer_cells(0, &result, "Probe".to_string(), true)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(cells[3], "n/a");
}

#[test]
fn the_imbalance_column_shows_n_a_without_both_sides() {
    let result = priced_offer(Some(12.5), None, Some((100, 1000.0)));
    let cells: Vec<String> = offer_cells(0, &result, "Probe".to_string(), true)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(cells[4], "n/a");
}

#[test]
fn the_imbalance_shows_its_sign_and_never_a_negative_zero() {
    assert_eq!(format_imbalance(Some(73.4)), "+73%");
    assert_eq!(format_imbalance(Some(-73.4)), "-73%");
    assert_eq!(format_imbalance(Some(100.0)), "+100%");
    assert_eq!(format_imbalance(Some(-100.0)), "-100%");
    assert_eq!(format_imbalance(Some(0.0)), "0%");
    assert_eq!(format_imbalance(Some(-0.4)), "0%");
    assert_eq!(format_imbalance(None), "n/a");
}

#[test]
fn the_imbalance_colour_follows_the_bands() {
    assert_eq!(imbalance_style(Some(100.0)), Style::new().green());
    assert_eq!(
        imbalance_style(Some(IMBALANCE_STRONG)),
        Style::new().green()
    );
    assert_eq!(imbalance_style(Some(10.0)), Style::new().yellow());
    assert_eq!(imbalance_style(Some(0.0)), Style::new().yellow());
    assert_eq!(imbalance_style(Some(-10.0)), Style::new().yellow());
    assert_eq!(imbalance_style(Some(-IMBALANCE_STRONG)), Style::new().red());
    assert_eq!(imbalance_style(Some(-100.0)), Style::new().red());
    assert_eq!(imbalance_style(None), Style::new().dark_gray());
}

#[test]
fn a_huge_spread_is_capped_to_hold_the_column_width() {
    assert_eq!(format_spread(Some(12.5)), "12.50%");
    assert_eq!(format_spread(Some(-25.0)), "-25.00%");
    assert_eq!(format_spread(Some(-100.0)), "-100.00%");
    assert_eq!(format_spread(Some(999.0)), "999.00%");
    assert_eq!(format_spread(Some(999.5)), ">999%");
    assert_eq!(format_spread(Some(1234.5)), ">999%");
    assert_eq!(format_spread(None), "n/a");
}

#[test]
fn the_spread_colour_follows_the_bands() {
    assert_eq!(spread_style(Some(0.0)), Style::new().green());
    assert_eq!(spread_style(Some(SPREAD_TIGHT)), Style::new().green());
    assert_eq!(spread_style(Some(10.1)), Style::new().yellow());
    assert_eq!(spread_style(Some(SPREAD_WIDE)), Style::new().yellow());
    assert_eq!(spread_style(Some(50.1)), Style::new().red());
    assert_eq!(spread_style(Some(-1.0)), Style::new().magenta());
    assert_eq!(spread_style(None), Style::new().dark_gray());
}

#[test]
fn each_layout_shows_its_own_columns_in_order() {
    let titles = |wide| -> Vec<&str> {
        offer_columns(wide)
            .iter()
            .map(|(column, _)| column.title)
            .collect()
    };
    assert_eq!(
        titles(true),
        [
            "#",
            "Item",
            "ISK/LP",
            "Spread",
            "Imbal",
            "Vol ±10%",
            "Profit ±10%"
        ]
    );
    assert_eq!(
        titles(false),
        ["#", "Item", "ISK/LP", "Spread", "Profit ±10%"]
    );
    let item_width = |wide| offer_columns(wide)[1].1;
    assert_eq!(item_width(true), Constraint::Min(12));
    assert_eq!(
        item_width(false),
        Constraint::Min(10),
        "the narrow item column can shrink further"
    );
}

#[test]
fn only_the_spread_and_imbalance_cells_are_coloured() {
    let result = priced_offer(Some(12.5), Some(50.0), Some((100, 1000.0)));
    let styles: Vec<Style> = offer_cells(0, &result, "Probe".to_string(), true)
        .into_iter()
        .map(|(_, style)| style)
        .collect();
    assert_eq!(styles[3], spread_style(Some(12.5)));
    assert_eq!(styles[4], imbalance_style(Some(50.0)));
    for column in [0, 1, 2, 5, 6] {
        assert_eq!(styles[column], Style::new(), "column {column}");
    }
    let narrow = offer_cells(0, &result, "Probe".to_string(), false);
    assert_eq!(
        narrow[4].1,
        Style::new(),
        "the narrow profit column is not the imbalance"
    );
}

/// The whole screen after drawing `f`, as every cell's symbol in reading order.
fn rendered(width: u16, height: u16, f: impl FnOnce(&mut Frame)) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(f).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// The whole `--tui` view rendered at `width` x `height`, as one string, on empty static data.
fn view_text(app: &App, width: u16, height: u16) -> String {
    let sdata = StaticData::default();
    let stats = ClientStats::default();
    rendered(width, height, |frame| {
        draw(frame, app, &sdata, &stats);
    })
}

/// One offer with a known spread, imbalance, volume, and profit, so a test can look for the
/// exact strings that have to survive the layout.
fn one_priced_offer() -> Vec<OfferResult> {
    vec![priced_offer(
        Some(12.5),
        Some(50.0),
        Some((2_474_600, 3_980_000_000.0)),
    )]
}

#[test]
fn the_wide_offers_table_shows_every_column_at_120x40() {
    let app = results_app(one_priced_offer(), 10);
    let text = view_text(&app, 120, 40);
    for title in [
        "Item",
        "ISK/LP",
        "Spread",
        "Imbal",
        "Vol ±10%",
        "Profit ±10%",
    ] {
        assert!(text.contains(title), "the column {title:?} did not render");
    }
    for value in ["12.50%", "+50%", "2,474,600", "3.98B"] {
        assert!(text.contains(value), "the value {value:?} did not render");
    }
}

#[test]
fn the_narrow_offers_table_fits_the_smallest_terminal() {
    // The smallest terminal the view accepts has to show the last column in full. It did not
    // until `Item`'s floor dropped, and nothing caught it because no test rendered the results.
    let app = results_app(one_priced_offer(), 10);
    let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT + 4);
    assert!(
        text.contains("Spread") && text.contains("12.50%"),
        "the spread column shows in both layouts"
    );
    assert!(
        text.contains("Profit ±10%") && text.contains("3.98B"),
        "the last column was cut at {}x{}",
        MIN_WIDTH,
        MIN_HEIGHT + 4
    );
    assert!(!text.contains("Imbal"), "the imbalance is wide only");
    assert!(!text.contains("Vol ±10%"), "the volume is wide only");
}

#[test]
fn the_wide_offers_table_survives_the_terminal_it_switches_on_at() {
    // The offers pane is 68% of the terminal, so this is about the narrowest terminal whose
    // pane reaches WIDE_OFFERS. Every column has to survive at the threshold, which is where an
    // over-wide one first cuts the last column's header off.
    let width = (f64::from(WIDE_OFFERS) / 0.68).ceil() as u16;
    let app = results_app(one_priced_offer(), 10);
    let text = view_text(&app, width, 24);
    assert!(
        text.contains("Imbal"),
        "the wide layout did not engage at {} columns",
        width
    );
    assert!(
        text.contains("Profit ±10%") && text.contains("3.98B"),
        "the last column was cut at {} columns",
        width
    );
}

#[test]
fn no_terminal_width_cuts_the_last_offers_column() {
    // Both layouts were truncating "Profit ±10%" at their own minimum widths, and the gap
    // between them was not obvious from the column arithmetic. Every width the view accepts has
    // to show the last column in full, whichever layout it picks.
    for width in MIN_WIDTH..=160 {
        let app = results_app(one_priced_offer(), 10);
        let text = view_text(&app, width, MIN_HEIGHT + 4);
        assert!(
            text.contains("Profit ±10%") && text.contains("3.98B"),
            "the last offers column was cut at {} columns",
            width
        );
    }
}

#[test]
fn the_export_prompt_shows_the_name_and_the_footer_hints_for_it() {
    let mut app = results_app(one_priced_offer(), 10);
    app.open_export();
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Export"), "the popup is titled");
    assert!(
        text.contains("lpstores-the-forge.csv"),
        "the suggested name shows: {}",
        text
    );
    assert!(
        text.contains("Enter write"),
        "the footer switches to the prompt's keys"
    );
    assert!(
        !text.contains("f filters"),
        "the view keys are hidden while the prompt owns the keyboard"
    );
}

#[test]
fn a_refused_export_shows_why_beside_the_name() {
    let mut app = results_app(one_priced_offer(), 10);
    app.open_export();
    app.refuse_export("Use a file name ending in .csv or .json");
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Use a file name ending in .csv or .json"));
    assert!(
        text.contains("lpstores-the-forge.csv"),
        "the name is still there to correct"
    );
}

#[test]
fn the_static_data_dialog_offers_the_download_and_says_where_it_goes() {
    let mut app = App::new(LpStoreOptions::default());
    app.open_static_data(None);
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Static data"), "the popup is titled");
    assert!(
        text.contains("Enter or d downloads it"),
        "the keys are on the dialog itself: {}",
        text
    );
    assert!(
        text.contains("It is stored in the data directory."),
        "it names a place even before the real root is known"
    );
    assert!(
        text.contains("Enter or d download"),
        "the footer switches to the dialog's keys"
    );
    assert!(
        !text.contains("f filters"),
        "the view keys are hidden while it is open"
    );
}

#[test]
fn the_static_data_dialog_names_the_root_the_view_was_given() {
    // The whole point of the platform roots is that they are not `data/` any more, so the
    // dialog has to repeat what it was told rather than a guessed relative name.
    let mut app = App::new(LpStoreOptions::default());
    app.set_data_dir("/home/user/.local/share/eve_lpstore");
    app.open_static_data(None);
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("It is stored in /home/user/.local/share/eve_lpstore."),
        "the given root is not shown"
    );
}

#[test]
fn the_static_data_dialog_shows_the_reason_when_the_files_cannot_be_read() {
    let mut app = App::new(LpStoreOptions::default());
    app.open_static_data(Some("data/regions.json is not valid JSON".to_string()));
    let text = view_text(&app, 120, 40);
    assert!(text.contains("data/regions.json is not valid JSON"));
    assert!(
        text.contains("Enter or d downloads it"),
        "retry is still offered"
    );
}

#[test]
fn the_static_data_dialog_changes_while_it_downloads() {
    let mut app = App::new(LpStoreOptions::default());
    app.open_static_data(None);
    app.start_download();
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("Downloading the EVE Online static data"),
        "the dialog says something is happening"
    );
    assert!(
        !text.contains("Enter or d download"),
        "the download keys are gone once it started"
    );
    assert!(text.contains("Ctrl+C quits"), "the way out is still shown");
}

#[test]
fn the_idle_view_says_nothing_has_started_behind_the_startup_dialog() {
    let mut app = App::new(LpStoreOptions::default());
    app.open_static_data(None);
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Not started"), "the header is honest");
    assert!(
        !text.contains("0 corporations"),
        "no corporation count is claimed before a run"
    );
    assert!(
        !text.contains("waiting"),
        "the gauges are left out rather than reading waiting before anything has started"
    );
    assert!(
        !text.contains("avg ISK/LP"),
        "no results are claimed before a run"
    );
}

#[test]
fn the_startup_filters_dialog_shows_the_prefilled_defaults() {
    let mut app = App::new(LpStoreOptions::default());
    let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    app.show_filters(form);
    let text = view_text(&app, 120, 40);
    assert!(text.contains("The Forge"), "the region default is visible");
    assert!(
        text.contains("Corporations"),
        "the filter fields are there and empty"
    );
    assert!(
        text.contains("Enter apply"),
        "the footer shows the form's keys"
    );
}

#[test]
fn a_note_about_the_saved_file_shows_on_the_startup_form() {
    // The form owns the keyboard and the log panel cannot be opened over it, so this is the only
    // place the reason could be seen before the first run.
    let mut app = App::new(LpStoreOptions::default());
    let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    form.set_note(Some(
        "filters.json is not valid JSON: expected value".to_string(),
    ));
    app.show_filters(form);
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("is not valid JSON"),
        "the reason is on screen: {}",
        text
    );
    assert!(
        text.contains("The Forge"),
        "the form still shows what it fell back to"
    );
}

#[test]
fn the_market_hints_split_between_the_row_and_the_note_below() {
    // The row's detail column is one line tall, so an inline hint must show only its purpose.
    // Before this split the two lines collided into one mangled row reading
    // "Prices use its NPC stations.Name or ID…".
    for (focus, purpose) in [
        (
            FormField::Text(Field::Region),
            "Prices come from its market orders.",
        ),
        (
            FormField::Text(Field::System),
            "Prices use its NPC stations.",
        ),
    ] {
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.focus = focus;
        app.show_filters(form);
        let text = view_text(&app, 120, 40);
        assert!(text.contains(purpose), "the row shows its purpose");
        assert!(
            !text.contains(&format!("{purpose}Name")),
            "the row glued two lines together: {focus:?}"
        );
    }
}

#[test]
fn the_focused_market_field_shows_both_hint_lines_when_there_is_room() {
    let mut app = App::new(LpStoreOptions::default());
    let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    form.focus = FormField::Text(Field::System);
    app.show_filters(form);
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("Name or ID. Blank for all systems."),
        "the syntax line shows under the focused field"
    );
}

#[test]
fn the_focused_place_field_shows_both_hint_lines() {
    // The market fields ask for two lines, and the place fields do the same. This is what the
    // raised minimum height pays for: at the old size the second line had nowhere to go.
    for (focus, syntax) in [
        (
            FormField::Text(Field::AgentRegion),
            "ID or name. Blank for any region.",
        ),
        (
            FormField::Text(Field::AgentSystem),
            "ID or name. Blank for any system.",
        ),
    ] {
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.focus = focus;
        app.show_filters(form);
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        assert!(
            text.contains(syntax),
            "{focus:?} lost its syntax line: {text}"
        );
    }
}

#[test]
fn the_cache_minutes_field_explains_the_blank_default_and_zero() {
    let mut app = App::new(LpStoreOptions::default());
    let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    form.focus = FormField::Text(Field::CacheMinutes);
    app.show_filters(form);
    let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
    assert!(text.contains("Minutes past ESI Expires."), "{text}");
    assert!(
        text.contains("Blank: filtered corp count; 0 disables."),
        "the two existing defaults are explained: {text}"
    );
}

#[test]
fn the_pane_words_a_collapsed_band_group() {
    // Sazdih from Bayuka: every route starts in the lowest system, so the route bands
    // collapse to one row while the destination lines still name Moh's own 0.5.
    let spread = eve_lpstore::static_data::DestinationSpread {
        constellation_id: 20000006,
        systems: 6,
        bands: vec![(4, 1.0)],
        own_bands: vec![(4, 5.0 / 6.0), (5, 1.0 / 6.0)],
        hops: vec![(0, 1.0 / 6.0), (1, 3.0 / 6.0), (2, 2.0 / 6.0)],
        destinations: vec![
            (30000039, 0, 4, 4),
            (30000038, 1, 4, 4),
            (30000040, 1, 4, 4),
            (30000041, 1, 4, 4),
            (30000042, 2, 4, 5),
            (30000043, 2, 4, 4),
        ],
    };
    let names = std::collections::HashMap::from([
        (30000039, "Bayuka"),
        (30000038, "Dooz"),
        (30000040, "Uzistoon"),
        (30000041, "Bairshir"),
        (30000042, "Moh"),
        (30000043, "Sari"),
    ]);
    let rows = system_lines(30000039, "Bayuka", &spread, &|id| {
        names.get(&id).map(|found| found.to_string())
    });
    assert_eq!(
        rows[0],
        "  Bayuka (30000039)  route 0.4/100%  0J/17% 1J/50% 2J/33%"
    );
    assert_eq!(
        rows[1], "    Dooz 1J route 0.4",
        "same band as its own, so no note"
    );
    let moh = rows.iter().find(|r| r.contains("Moh")).expect("Moh listed");
    assert_eq!(moh, "    Moh 2J route 0.4  own 0.5");
    assert_eq!(rows.len(), 6, "one summary plus five destinations");
    assert!(
        !rows.iter().any(|r| r.contains("Bayuka (30000039)  0J")),
        "the agent system is not repeated as a destination"
    );
}

#[test]
fn the_pane_splits_bands_when_the_routes_allow_it() {
    let spread = eve_lpstore::static_data::DestinationSpread {
        constellation_id: 100,
        systems: 4,
        bands: vec![(5, 0.75), (6, 0.25)],
        own_bands: vec![(5, 0.75), (6, 0.25)],
        hops: vec![(0, 0.25), (1, 0.25), (2, 0.25), (3, 0.25)],
        destinations: vec![(1, 0, 5, 5), (2, 1, 5, 5), (3, 2, 5, 5), (4, 3, 5, 6)],
    };
    let rows = system_lines(1, "A", &spread, &|id| Some(format!("S{id}")));
    assert_eq!(
        rows[0],
        "  A (1)  route 0.5/75% 0.6/25%  0J/25% 1J/25% 2J/25% 3J/25%"
    );
    assert_eq!(rows.len(), 4, "one summary plus the three other systems");
    assert_eq!(rows[3], "    S4 3J route 0.5  own 0.6");
}

#[test]
fn an_unknown_destination_id_still_gets_a_line() {
    let spread = eve_lpstore::static_data::DestinationSpread {
        constellation_id: 1,
        systems: 2,
        bands: vec![(4, 1.0)],
        own_bands: vec![(4, 1.0)],
        hops: vec![(0, 0.5), (1, 0.5)],
        destinations: vec![(9, 0, 4, 4), (99, 1, 4, 4)],
    };
    let rows = system_lines(9, "Here", &spread, &|_| None);
    assert_eq!(rows[1], "    system 99 1J route 0.4");
}

/// A spread with the numbers of the asked-for example, so the wording is pinned against it.
fn two_band_spread() -> eve_lpstore::static_data::DestinationSpread {
    eve_lpstore::static_data::DestinationSpread {
        constellation_id: 20000300,
        systems: 2,
        bands: vec![(1, 0.5), (2, 0.5)],
        own_bands: vec![(1, 0.5), (2, 0.5)],
        hops: vec![(0, 0.5), (1, 0.5)],
        destinations: vec![(30003601, 0, 1, 1), (30003602, 1, 2, 2)],
    }
}

#[test]
fn the_pane_opens_at_the_top_of_the_table() {
    // The bug this pins: the pane inherited the log's bottom-first window, so it opened showing
    // the tail of the list and Down moved it up. A corporation table reads top to bottom.
    assert_eq!(pane_window(60, 0, 10), (0, 10), "nothing scrolled");
    assert_eq!(pane_window(60, 5, 10), (5, 15), "scrolled down");
    assert_eq!(
        pane_window(60, 999, 10),
        (50, 60),
        "clamped to the last row"
    );
    assert_eq!(pane_window(4, 0, 10), (0, 4), "shorter than the pane");
    assert_eq!(
        pane_window(4, 2, 0),
        (0, 0),
        "a pane with no room shows nothing"
    );
}

#[test]
fn a_long_summary_wraps_in_the_pane_instead_of_being_cut() {
    // The longest real summary measured 126 characters against the 78 columns a border leaves at
    // the minimum width, so this is the case the pane cannot show in one row.
    let long = format!(
        "  {}",
        [
            "Evenlongname (0.12)",
            "0.1/12%, 0.2/12%, 0.3/12%, 0.4/12%, 0.5/12%",
            "0J/12%, 1J/12%, 2J/12%, 3J/12%, 4J/12%, 5J/12%, 6J/12%, 7J/12%",
        ]
        .join("  ")
    );
    assert!(long.chars().count() > 100, "fixture is long enough");
    let dest = "    Deep 3J route 0.4";
    let body = pane_body(&[long.clone(), dest.to_string()], 78);
    assert!(
        body.len() >= 3,
        "the summary took more than one row: {body:?}"
    );
    for (part, _) in &body {
        assert!(part.chars().count() <= 78, "{part:?} overran the pane");
    }
    // Wrapping must lose no word and reorder none.
    let wrapped: Vec<&str> = body
        .iter()
        .flat_map(|(part, _)| part.split_whitespace())
        .collect();
    let wanted: Vec<&str> = long
        .split_whitespace()
        .chain(dest.split_whitespace())
        .collect();
    assert_eq!(wrapped, wanted, "wrapping lost or moved a word");
    assert!(
        body[0].0.starts_with("  Evenlongname"),
        "the first row keeps the indent"
    );
    assert!(
        body.iter().skip(1).all(|(part, _)| part.starts_with("  ")),
        "continuation rows are indented too: {body:?}"
    );
    assert_eq!(
        body.last().unwrap().0,
        dest,
        "the destination row is intact"
    );
    assert_eq!(body.last().unwrap().1, Style::new().dark_gray());
}

#[test]
fn the_system_block_caps_at_what_the_cli_report_cuts_at() {
    // One constant governs both views, so a corporation cannot show ten systems in the report and
    // twelve in the pane. 53 is the largest count measured on the archive.
    let ids: Vec<i32> = (1..=53).collect();
    let (shown, hidden) = cap_systems(ids);
    assert_eq!(shown.len(), output::MAX_SYSTEMS_SHOWN);
    assert_eq!(hidden, 43);
    assert_eq!(
        shown,
        (1..=10).collect::<Vec<_>>(),
        "data order, the first ten"
    );
    let exactly = cap_systems((1..=10).collect::<Vec<i32>>());
    assert_eq!(
        (exactly.0.len(), exactly.1),
        (10, 0),
        "at the cap, nothing is withheld"
    );
    let few = cap_systems((1..=3).collect::<Vec<i32>>());
    assert_eq!((few.0.len(), few.1), (3, 0));
    assert_eq!(
        cap_systems(Vec::<i32>::new()).1,
        0,
        "and an empty list stays empty"
    );
}

#[test]
fn the_overflow_line_counts_what_it_hides_and_names_the_way_to_see_it() {
    assert_eq!(
        more_systems_row(43),
        "  +43 more systems, press m for all of them"
    );
    assert_eq!(
        more_systems_row(1),
        "  +1 more system, press m for all of them"
    );
    // The largest count this can reach is 43, the biggest corporation measured on the archive
    // holding 53 systems, so the line stays well inside the 52 columns the details pane has at
    // the minimum size and never wraps onto a second row of its own.
    assert_eq!(more_systems_row(43).chars().count(), 43);
}

#[test]
fn the_details_summary_is_one_indented_line_per_system() {
    assert_eq!(
        summary_text("Babirmoult", 0.1788, &two_band_spread()),
        "  Babirmoult (0.18) 0.1/50%, 0.2/50%  0J/50%, 1J/50%"
    );
    assert_eq!(
        summary_text("Annancale", 0.0941, &two_band_spread()),
        "  Annancale (0.09) 0.1/50%, 0.2/50%  0J/50%, 1J/50%"
    );
}

#[test]
fn burner_probability_is_a_separate_details_line() {
    assert_eq!(
        burner_summary_text(&two_band_spread()),
        "    Burners: 0.1/50%, 0.2/50%  0J/50%, 1J/50%"
    );
    assert!(
        !burner_summary_text(&two_band_spread()).starts_with("  Babirmoult"),
        "the burner marginal is not merged into the regular-agent summary"
    );
}

#[test]
fn bands_are_coloured_at_the_asked_boundaries() {
    assert_eq!(band_colour(0), Color::LightMagenta);
    assert_eq!(band_colour(-1), Color::LightMagenta, "nullsec and below");
    assert_eq!(band_colour(1), Color::Red);
    assert_eq!(band_colour(4), Color::Red, "lowsec ends at 0.4");
    assert_eq!(band_colour(5), Color::Yellow);
    assert_eq!(band_colour(6), Color::Green);
    assert_eq!(band_colour(8), Color::Green);
    assert_eq!(band_colour(9), Color::Cyan);
    assert_eq!(band_colour(10), Color::Cyan, "1.0 is the top of the scale");
}

#[test]
fn a_band_is_coloured_and_a_true_security_is_not() {
    // The system's own security is printed to two decimals and belongs to the agent, not to a
    // band, so it stays in the line's style while the band groups beside it are coloured.
    let spans = coloured("  Babirmoult (0.18) 0.1/50%, 0.2/50%", Style::new());
    let texts: Vec<String> = spans.iter().map(|s| s.content.to_string()).collect();
    assert_eq!(
        texts,
        ["  Babirmoult (0.18) ", "0.1", "/50%, ", "0.2", "/50%"]
    );
    assert_eq!(spans[1].style.fg, Some(Color::Red));
    assert_eq!(spans[3].style.fg, Some(Color::Red));
    assert_eq!(
        spans[0].style.fg, None,
        "the uncoloured run keeps the line style"
    );

    let route = coloured("    Moh 2J route 0.4  own 0.5", Style::new().dark_gray());
    let values: Vec<Option<Color>> = route.iter().map(|s| s.style.fg).collect();
    assert_eq!(
        values,
        vec![
            Some(Color::DarkGray),
            Some(Color::Red),
            Some(Color::DarkGray),
            Some(Color::Yellow),
        ]
    );
    assert_eq!(
        route.iter().map(|s| s.content.as_ref()).collect::<String>(),
        "    Moh 2J route 0.4  own 0.5",
        "colouring must not lose a character"
    );
}

#[test]
fn a_negative_band_is_read_as_a_band() {
    let spans = coloured("  9-ERCP (30001484)  route -0.1/100%", Style::new());
    assert_eq!(spans[1].content, "-0.1");
    assert_eq!(spans[1].style.fg, Some(Color::LightMagenta));
    // Hops and percents are not bands, and neither is a whole number written without a decimal.
    let plain = coloured("  0J/100% 1J/50% x 10 stations", Style::new());
    assert_eq!(plain.len(), 1, "nothing in {plain:?} should split");
}

#[test]
fn the_details_pane_says_why_no_systems_are_listed() {
    // Empty static data has no division table, so nothing can be put in scope; the pane states
    // that rather than printing corporation names with nothing under them.
    let app = results_app(one_priced_offer(), 10);
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Corporations"), "the section is there");
    assert!(
        text.contains("the static data names no Security division"),
        "and says why it is empty: {text}"
    );
}

#[test]
fn the_new_place_fields_still_leave_room_for_a_hint() {
    // The height this view accepts was raised when the form grew to twelve fields, so that the
    // notes under them — hints, valid values, and every validation error — keep a row. If the
    // form ever fills the dialog again, this fails rather than the error going unseen.
    let mut app = App::new(LpStoreOptions::default());
    let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    app.show_filters(form);
    for focus in [
        FormField::Text(Field::AgentRegion),
        FormField::Text(Field::AgentSystem),
    ] {
        app.form.focus = focus;
        let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
        let hint = hint_line(focus, &StaticData::default());
        assert!(
            text.contains(&hint),
            "{focus:?}'s hint was pushed out of a {}x{} dialog: {text}",
            MIN_WIDTH,
            MIN_HEIGHT
        );
    }
}

#[test]
fn a_field_error_on_a_place_filter_is_visible_at_the_minimum_size() {
    let mut app = App::new(LpStoreOptions::default());
    let form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
    app.show_filters(form);
    app.form.focus = FormField::Text(Field::AgentRegion);
    app.form.set_errors(vec![OptionError {
        field: Field::AgentRegion,
        message: "Region 'Black Riss' not found".to_string(),
    }]);
    let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
    assert!(
        text.contains("Region 'Black Riss' not found"),
        "the error is on screen, not cut by the field rows: {text}"
    );
}

#[test]
fn the_place_fields_offer_name_matches_like_the_market_ones() {
    // The suggestions come from the static data, so an empty cache says so rather than nothing.
    for (field, typed) in [
        (FormField::Text(Field::AgentRegion), "bla"),
        (FormField::Text(Field::AgentSystem), "jit"),
    ] {
        let mut app = App::new(LpStoreOptions::default());
        let mut form = FilterForm::from_options_with_defaults(&app.applied, "The Forge", 10);
        form.focus = field;
        if let FormField::Text(field) = field {
            form.set_value(field, typed);
        }
        app.show_filters(form);
        let shown = suggestions(&app, &StaticData::default()).expect("a name field suggests");
        assert_eq!(shown, "No matches", "{field:?} on empty data: {shown}");
    }
}

/// Results on screen, then a second run started over them: the state a re-applied filter leaves.
fn rerun_app(done: usize, total: usize) -> App {
    let mut app = results_app(one_priced_offer(), 10);
    app.start_run(forge_run(12), "--security 0.4..0.5".to_string(), 10);
    app.on_event(
        &eve_lpstore::report::Event::FetchProgress { done, total },
        None,
    );
    app
}

#[test]
fn a_re_run_shows_the_progress_popup_over_the_results() {
    let app = rerun_app(3, 12);
    let text = view_text(&app, 120, 40);
    assert!(text.contains(" Run "), "the popup is titled: {text}");
    assert!(
        text.contains("Fetching loyalty offers: 3 / 12 corporations"),
        "the stage line is in it: {text}"
    );
    assert!(
        text.contains("Pricing offers") && text.contains("waiting"),
        "both bars show, the idle one reading waiting: {text}"
    );
}

#[test]
fn the_first_run_keeps_its_bars_inline() {
    // No results behind it, so there is nothing to cover and the progress view is already the
    // whole screen. The popup is only for a run that superseded something.
    let mut app = App::new(LpStoreOptions::default());
    app.begin_run(forge_run(3), "none".to_string(), 10);
    app.on_event(
        &eve_lpstore::report::Event::FetchProgress { done: 1, total: 3 },
        None,
    );
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("Fetching loyalty offers: 1 / 3 corporations"),
        "the inline bar still reports progress: {text}"
    );
    assert!(!text.contains(" Run "), "and no popup covers anything");
}

#[test]
fn the_end_of_a_re_run_waits_for_a_key() {
    let mut app = rerun_app(12, 12);
    app.finish(report_of(one_priced_offer()));
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("Finished: 1 offers priced, 1 groups")
            && text.contains("any key closes this"),
        "the summary is shown, and says how it goes away: {text}"
    );
    // The same state at the smallest size the view accepts. The summary popup is ten rows, so a
    // screen this short is where it would be cut first, and the key line is its last row.
    let small = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
    assert!(
        small.contains("any key closes this"),
        "the summary fits {}x{}: {small}",
        MIN_WIDTH,
        MIN_HEIGHT
    );
    app.on_key(press(crossterm::event::KeyCode::Esc));
    let text = view_text(&app, 120, 40);
    assert!(
        !text.contains("any key closes this"),
        "one key was enough: {text}"
    );
    assert!(
        text.contains("Corporations"),
        "the fresh results are now the view"
    );
}

#[test]
fn an_open_overlay_wins_over_the_progress_popup() {
    let mut app = rerun_app(3, 12);
    app.overlay = Overlay::Log;
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains(" Log ("),
        "the log the user asked for is showing"
    );
    assert!(
        !text.contains(" Run "),
        "the popup does not stack over it: {text}"
    );
}

#[test]
fn a_stopped_re_run_reports_itself_in_the_popup() {
    let mut app = rerun_app(3, 12);
    app.fail();
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("The run stopped unexpectedly"),
        "the failure is on the popup: {text}"
    );
    assert!(text.contains("any key closes this"), "and it waits");
}

#[test]
fn the_missions_notes_fit_the_narrowest_view() {
    // The notes sit above the body and must not wrap, or they eat the rows they describe.
    for note in DESTINATION_NOTES {
        assert!(
            note.len() + 2 <= usize::from(MIN_WIDTH),
            "{} wraps at the minimum width",
            note
        );
    }
}

#[test]
fn the_missions_pane_states_that_it_is_a_model() {
    let app = missions_app();
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("model, not measurement"),
        "the pane says the numbers are a model: {text}"
    );
    assert!(
        text.contains("security missions stay in the constellation"),
        "and names the scope it assumes: {text}"
    );
    assert!(
        text.contains("safest gate route"),
        "and says what a band means"
    );
}

#[test]
fn scrolling_keeps_the_missions_notes_on_screen() {
    let mut app = missions_app();
    app.destinations_scroll = 50;
    app.destinations_rows = 60;
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("model, not measurement"),
        "the disclaimer is not part of the scrolled body"
    );
}

/// The results view with the destinations pane open.
fn missions_app() -> App {
    let mut app = results_app(one_priced_offer(), 10);
    app.overlay = Overlay::Destinations;
    app
}

#[test]
fn probabilities_are_whole_percent() {
    assert_eq!(percent(1.0 / 6.0), "17%");
    assert_eq!(percent(5.0 / 6.0), "83%");
    assert_eq!(percent(1.0), "100%");
    assert_eq!(percent(0.0), "0%");
}

#[test]
fn the_missions_pane_reports_what_it_cannot_show() {
    // An empty static data names no divisions at all, which is the state before the data has
    // been fetched, and the pane must say that rather than print nothing or guess an id.
    let mut app = results_app(one_priced_offer(), 10);
    app.overlay = Overlay::Destinations;
    let text = view_text(&app, 120, 40);
    assert!(text.contains("Mission destinations"), "the pane is titled");
    assert!(
        text.contains("the static data names no Security division"),
        "it says why there is nothing to list: {text}"
    );
}

#[test]
fn the_scope_follows_the_division_the_run_asked_for() {
    assert_eq!(destination_scope(None, None), Scope::Unknown, "no data");
    assert_eq!(
        destination_scope(None, Some(22)),
        Scope::Unknown,
        "a division filter cannot conjure a division table"
    );
    assert_eq!(destination_scope(Some(24), None), Scope::Included);
    assert_eq!(destination_scope(Some(24), Some(24)), Scope::Included);
    assert_eq!(
        destination_scope(Some(24), Some(22)),
        Scope::Excluded(22),
        "Distribution is courier work, outside the model"
    );
}

#[test]
fn an_excluded_run_is_told_which_division_it_asked_for() {
    assert_eq!(
        excluded_row("Distribution", 78),
        "Distribution agents are outside this model: security missions only"
    );
}

#[test]
fn the_longest_division_name_still_fits_the_pane() {
    // "Industrialist - Entrepreneur" is 28 characters, and with a fixed sentence it would have overrun
    // the 78 columns a border leaves at the minimum width, where the line is cut without an
    // ellipsis because the pane does not wrap. The name gives way instead of the message.
    let width = usize::from(MIN_WIDTH) - 2;
    for division in [
        "Security",
        "Industrialist - Entrepreneur",
        "Soldier of Fortune",
        "a name far longer than any division in the archive at all",
    ] {
        let row = excluded_row(division, width);
        assert!(
            row.chars().count() <= width,
            "{row} overruns {width} columns"
        );
        assert!(
            row.ends_with("security missions only"),
            "the reason is never the part that gets cut: {row}"
        );
    }
    assert_eq!(
        excluded_row("Industrialist - Entrepreneur", width),
        "Industrialist - Entrepr… agents are outside this model: security missions only"
    );
}

#[test]
fn the_missions_pane_names_a_group_only_after_a_selection() {
    let mut app = App::new(LpStoreOptions::default());
    app.overlay = Overlay::Destinations;
    let text = view_text(&app, 120, 40);
    assert!(
        text.contains("No group selected."),
        "empty view is explicit"
    );
}

#[test]
fn the_inline_hint_is_one_line_even_when_the_note_is_two() {
    assert_eq!(
        hint_line(FormField::Text(Field::Region), &StaticData::default()),
        "Prices come from its market orders."
    );
    assert_eq!(
        hint_line(FormField::Text(Field::System), &StaticData::default()),
        "Prices use its NPC stations."
    );
    // The fields these changes worded have to fit the detail column at the minimum terminal
    // width, so their extra wording goes on the second line. Other hints are longer and already
    // truncate there, which is existing behaviour this test does not cover.
    assert_eq!(
        hint_line(FormField::Text(Field::Security), &StaticData::default()),
        "0.5 exact, 0.5..0.7 for a range."
    );
    let detail = usize::from(MIN_WIDTH - 2) - (LABEL_WIDTH + VALUE_WIDTH + 2);
    for field in [
        FormField::Text(Field::Region),
        FormField::Text(Field::System),
        FormField::Text(Field::Security),
        FormField::Text(Field::AgentRegion),
        FormField::Text(Field::AgentSystem),
        FormField::Text(Field::CacheMinutes),
    ] {
        let line = hint_line(field, &StaticData::default());
        assert!(
            line.chars().count() <= detail,
            "{field:?} needs {} of {} columns: {:?}",
            line.chars().count(),
            detail,
            line
        );
    }
    // No inline hint may carry a break, whatever its length.
    for field in FormField::all() {
        assert!(
            !hint_line(field, &StaticData::default()).contains('\n'),
            "{field:?} kept the break in the row"
        );
    }
}

#[test]
fn the_grouped_dialog_renders_at_the_minimum_terminal_size() {
    let mut app = App::new(LpStoreOptions::default());
    app.show_filters(FilterForm::from_options_with_defaults(
        &app.applied,
        "The Forge",
        10,
    ));
    let text = view_text(&app, MIN_WIDTH, MIN_HEIGHT);
    for label in [
        "Corporations",
        "Security",
        "Agent region",
        "Agent system",
        "Agent level",
        "Division",
        "Agent type",
        "Faction",
        "Market region",
        "Market system",
        "Depth",
        "Cache minutes",
        "Expire cache",
    ] {
        assert!(text.contains(label), "{label:?} did not render");
    }
    assert!(
        !text.contains("stations.Name"),
        "a two-line hint was glued into one row"
    );
}

fn screen_text(width: u16, height: u16) -> String {
    rendered(width, height, |frame| draw_donate(frame, frame.area()))
}

#[test]
fn the_donation_popup_shows_the_code_and_the_address_when_it_fits() {
    let text = screen_text(80, 24);
    assert!(text.contains(DONATION_ADDRESS));
    assert!(text.contains('▀'));
    assert!(!text.contains("Enlarge"));
}

#[test]
fn the_donation_popup_falls_back_to_the_address_when_the_code_does_not_fit() {
    let text = screen_text(80, 20);
    assert!(text.contains(DONATION_ADDRESS));
    assert!(text.contains("Enlarge the terminal"));
}

#[test]
fn tail_keeps_the_end_of_long_text() {
    assert_eq!(tail("abcdef", 4), "…def");
    assert_eq!(tail("abc", 4), "abc");
}

#[test]
fn notes_past_the_room_are_cut_with_an_ellipsis() {
    let note = |text: &str| (text.to_string(), Style::new());
    let kept = fit(vec![note("one"), note("two"), note("three")], 2, 10);
    assert_eq!(kept, vec![note("one"), note("two…")]);
    assert_eq!(fit(vec![note("one")], 2, 10), vec![note("one")]);
    assert!(fit(vec![note("one")], 0, 10).is_empty());
}

//! The progress view before the first results, and the log.

use super::*;

pub(super) fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    // Before a run starts there is nothing to gauge, so the two bars that would read "waiting" are
    // left out and the log gets their space.
    let idle = app.stage == Stage::Idle;
    let (stage_area, log_area) = if idle {
        let [stage, log] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        (stage, log)
    } else {
        let [stage, fetch, pricing, log] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(area);
        let ((fetch_done, fetch_total), (price_done, price_total)) = app.gauges();
        gauge(
            frame,
            fetch,
            "Fetching loyalty offers",
            fetch_done,
            fetch_total,
        );
        gauge(frame, pricing, "Pricing offers", price_done, price_total);
        (stage, log)
    };
    let style = if app.stage == Stage::Failed {
        Style::new().red()
    } else {
        Style::new()
    };
    frame.render_widget(Paragraph::new(app.stage_text()).style(style), stage_area);

    let height = usize::from(log_area.height.saturating_sub(2));
    let lines = log_lines(&app.log, height, 0);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Recent log (l for all) ")),
        log_area,
    );
}

pub(super) fn gauge(frame: &mut Frame, area: Rect, title: &str, done: usize, total: usize) {
    let (ratio, label) = if total == 0 {
        (0.0, "waiting".to_string())
    } else {
        (
            (done as f64 / total as f64).min(1.0),
            format!("{} / {}", done, total),
        )
    };
    let widget = Gauge::default()
        .block(Block::bordered().title(format!(" {} ", title)))
        .gauge_style(Style::new().fg(Color::Cyan))
        .ratio(ratio)
        .label(label);
    frame.render_widget(widget, area);
}

pub(super) fn draw_log(frame: &mut Frame, area: Rect, app: &App) {
    let height = usize::from(area.height.saturating_sub(2));
    let lines = log_lines(&app.log, height, app.log_scroll);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Log (l or Esc to close) ")),
        area,
    );
}

/// The newest `height` lines, skipping the `scroll` newest. Errors are red.
fn log_lines(log: &VecDeque<LogLine>, height: usize, scroll: usize) -> Vec<Line<'static>> {
    let end = log.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    log.range(start..end)
        .map(|line| {
            let style = if line.is_error {
                Style::new().red()
            } else {
                Style::new()
            };
            Line::from(Span::styled(line.text.clone(), style))
        })
        .collect()
}

//! Fitting text to a width: cutting, keeping the end, and wrapping.

use super::*;

/// Keeps the first `room` notes. When some are cut, the last one kept ends with an ellipsis.
pub(super) fn fit(
    mut notes: Vec<(String, Style)>,
    room: usize,
    width: usize,
) -> Vec<(String, Style)> {
    if notes.len() > room {
        notes.truncate(room);
        if let Some((text, _)) = notes.last_mut() {
            *text = truncate(&format!("{}…", text), width);
        }
    }
    notes
}

/// Cuts `text` to `width` characters, ending a cut with an ellipsis.
pub(super) fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{}…", kept)
}

/// The last `width` characters of `text`, with an ellipsis when the start is cut.
pub(super) fn tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let kept: String = text.chars().skip(count - width.saturating_sub(1)).collect();
    format!("…{}", kept)
}

/// Splits `text` into rows of at most `width` characters, breaking at spaces where it can. A
/// word longer than a row is cut into pieces.
pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    // A break in the source text starts a row of its own, so a hint can ask for two lines instead
    // of leaving that to the width. Each of those lines then wraps as usual.
    if let Some((first, rest)) = text.split_once('\n') {
        let mut rows = wrap_text(first, width);
        rows.extend(wrap_text(rest, width));
        return rows;
    }
    // Leading spaces are indentation, not a word, and splitting on spaces would drop them: a
    // wrapped destination line would then start its second row at column 0 and read like a new
    // system rather than the continuation of one. The indent is taken off, the rest wrapped into
    // what is left of the width, and the indent put back on every row.
    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    if !indent.is_empty() {
        if width <= indent.chars().count() {
            // Nothing is left to wrap into. Hand back the line and let the pane clip it, which is
            // what it did before, rather than cutting one character per row. The view's own
            // minimum width makes this unreachable; the guard is so it stays that way.
            return vec![text.to_string()];
        }
        let room = width - indent.chars().count();
        return wrap_text(&text[indent.len()..], room)
            .into_iter()
            .map(|row| format!("{indent}{row}"))
            .collect();
    }
    let width = width.max(1);
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let len = word.chars().count();
        let joined = if current.is_empty() {
            len
        } else {
            current.chars().count() + 1 + len
        };
        if joined <= width {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
            continue;
        }
        if !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        let chars: Vec<char> = word.chars().collect();
        let mut pieces = chars.chunks(width).peekable();
        while let Some(piece) = pieces.next() {
            let piece: String = piece.iter().collect();
            if pieces.peek().is_some() {
                rows.push(piece);
            } else {
                current = piece;
            }
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

pub(super) fn optional(value: Option<f64>) -> String {
    format_optional(value, 2)
}

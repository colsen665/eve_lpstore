//! Displayed security and the `--security` filter that compares on it.

/// Displayed security is rounded as described at https://wiki.eveuniversity.org/System_security
///
/// Returns the displayed security in tenths, so 0.5 is `5` and -0.5 is `-5`.
pub(super) fn displayed_security(true_sec: f64) -> i32 {
    if true_sec > 0.0 && true_sec < 0.05 {
        return 1;
    }
    (true_sec * 10.0).round() as i32
}

/// A security range, compared on displayed security. Both bounds are inclusive, and either side
/// being `None` leaves that end open, so `0.5..0.7` is `low: 5, high: 7`, `0.5..` is
/// `low: 5, high: None`, and `0.5` alone is both bounds equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityFilter {
    low: Option<i32>,
    high: Option<i32>,
}

impl SecurityFilter {
    /// Parse a level or a range: `0.5` exactly, `0.5..0.7` inclusive on both bounds, `0.5..` at
    /// least, and `..0.7` at most.
    ///
    /// The old `<op><level>` forms are refused rather than reinterpreted. A bare value starting
    /// with `-` is the reason: `-0.5` used to mean *at most 0.5* and now reads as the level
    /// *minus a half*, so accepting it would quietly filter different corporations. Write
    /// `..0.5` for *at most*, or `-0.5..-0.5` for that level exactly.
    pub fn parse(value: &str) -> Result<Self, String> {
        // Surrounding space is noise on a value that came from a quoted argument or a form field,
        // and an all-blank side counts as an open one so ".. " reads as the bare ".." it means.
        let value = value.trim();
        if let Some((low, high)) = value.split_once("..") {
            let (low, high) = (low.trim(), high.trim());
            let low = if low.is_empty() {
                None
            } else {
                Some(level(low)?)
            };
            let high = if high.is_empty() {
                None
            } else {
                Some(level(high)?)
            };
            match (low, high) {
                (None, None) => {
                    return Err(format!(
                        "'{value}' bounds nothing. Use 0.5..0.7, 0.5.., or ..0.7"
                    ));
                }
                (Some(low), Some(high)) if low > high => {
                    return Err(format!(
                        "the lower bound {} is above the upper bound {}",
                        show(low),
                        show(high)
                    ));
                }
                _ => return Ok(Self { low, high }),
            }
        }

        // A level on its own, unless it carries an old operator.
        if let Some(rest) = value.strip_prefix('+') {
            return Err(format!(
                "'{value}' no longer uses +. Write '{rest}..' for at least {}",
                show_or_raw(rest)
            ));
        }
        if let Some(rest) = value.strip_prefix('=') {
            return Err(format!(
                "'{value}' no longer uses =. Write '{rest}' on its own for exactly {}",
                show_or_raw(rest)
            ));
        }
        if value.starts_with('-') {
            return Err(format!(
                "'{value}' is ambiguous: '-x' used to mean at most x. Write '..{}' for at most \
                 {}, or '{value}..{value}' for that level exactly",
                value.strip_prefix('-').unwrap_or(value),
                show_or_raw(value.strip_prefix('-').unwrap_or(value)),
            ));
        }
        let only = level(value)?;
        Ok(Self {
            low: Some(only),
            high: Some(only),
        })
    }

    pub(super) fn matches(self, tenths: i32) -> bool {
        self.low.is_none_or(|low| tenths >= low) && self.high.is_none_or(|high| tenths <= high)
    }
}

/// One bound, in the displayed tenths the filter compares on, so `0.45` lands on `0.5` the same
/// way a single level always has.
fn level(text: &str) -> Result<i32, String> {
    let parsed: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number"))?;
    if !(-1.0..=1.0).contains(&parsed) {
        return Err("the level must be between -1.0 and 1.0".to_string());
    }
    Ok(displayed_security(parsed))
}

/// A bound in tenths back to the spelling the user typed.
fn show(tenths: i32) -> String {
    format!("{:.1}", tenths as f64 / 10.0)
}

/// Echo a level as written when it does not convert, so an error about `+abc` does not claim
/// `at least 0`.
fn show_or_raw(text: &str) -> String {
    match text.parse::<f64>() {
        Ok(parsed) if (-1.0..=1.0).contains(&parsed) => show(displayed_security(parsed)),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(low: Option<i32>, high: Option<i32>) -> SecurityFilter {
        SecurityFilter { low, high }
    }

    #[test]
    fn a_single_level_means_exactly_that_level() {
        assert_eq!(SecurityFilter::parse("0.5"), Ok(bounds(Some(5), Some(5))));
        assert_eq!(SecurityFilter::parse("1.0"), Ok(bounds(Some(10), Some(10))));
    }

    #[test]
    fn a_closed_range_includes_both_bounds() {
        assert_eq!(
            SecurityFilter::parse("0.5..0.7"),
            Ok(bounds(Some(5), Some(7)))
        );
        let filter = SecurityFilter::parse("0.5..0.7").unwrap();
        assert!(filter.matches(5), "the lower bound is included");
        assert!(filter.matches(6));
        assert!(filter.matches(7), "the upper bound is included");
        assert!(!filter.matches(4));
        assert!(!filter.matches(8));
    }

    #[test]
    fn an_open_end_leaves_that_side_unbounded() {
        assert_eq!(SecurityFilter::parse("0.5.."), Ok(bounds(Some(5), None)));
        assert_eq!(SecurityFilter::parse("..0.5"), Ok(bounds(None, Some(5))));
        let at_least = SecurityFilter::parse("0.5..").unwrap();
        assert!(at_least.matches(5));
        assert!(at_least.matches(10), "the old +0.5 also matched highsec");
        assert!(!at_least.matches(4));
        let at_most = SecurityFilter::parse("..0.5").unwrap();
        assert!(at_most.matches(0), "the old -0.5 also matched nullsec");
        assert!(at_most.matches(-5));
        assert!(!at_most.matches(6));
    }

    #[test]
    fn the_double_dot_keeps_a_leading_dash_a_sign() {
        assert_eq!(
            SecurityFilter::parse("-0.9..-0.5"),
            Ok(bounds(Some(-9), Some(-5)))
        );
        assert_eq!(
            SecurityFilter::parse("-0.5..-0.5"),
            Ok(bounds(Some(-5), Some(-5)))
        );
        assert_eq!(SecurityFilter::parse("..-0.5"), Ok(bounds(None, Some(-5))));
    }

    #[test]
    fn bounds_are_rounded_to_displayed_tenths() {
        // A single level has always gone through displayed_security, and each bound of a range
        // does the same, so 0.45 sits on the 0.5 band rather than being rejected.
        assert_eq!(
            SecurityFilter::parse("0.45..0.65"),
            Ok(bounds(Some(5), Some(7)))
        );
        assert_eq!(SecurityFilter::parse("0.45"), Ok(bounds(Some(5), Some(5))));
    }

    #[test]
    fn a_reversed_range_names_both_bounds() {
        let error = SecurityFilter::parse("0.7..0.5").unwrap_err();
        assert!(error.contains("0.7"), "{}", error);
        assert!(error.contains("0.5"), "{}", error);
        assert!(error.contains("above"), "{}", error);
    }

    #[test]
    fn a_range_that_bounds_nothing_is_refused() {
        for value in ["..", ".. ", " .."] {
            let error = SecurityFilter::parse(value).unwrap_err();
            assert!(error.contains("0.5..0.7"), "{value:?} gave {error:?}");
        }
    }

    #[test]
    fn the_old_forms_are_refused_and_say_what_to_write() {
        let plus = SecurityFilter::parse("+0.5").unwrap_err();
        assert!(plus.contains("0.5.."), "{plus}");
        let equals = SecurityFilter::parse("=0.5").unwrap_err();
        assert!(equals.contains("'0.5'"), "{equals}");
        // The bare dash is the dangerous one: it used to mean at most, and a plain reading of
        // "-0.5" as a level would silently change which corporations match.
        let minus = SecurityFilter::parse("-0.5").unwrap_err();
        assert!(minus.contains("..0.5"), "{minus}");
        assert!(minus.contains("-0.5..-0.5"), "{minus}");
    }

    #[test]
    fn levels_outside_the_security_range_are_refused() {
        for value in ["1.5", "-1.5..-1.5", "0.5..1.5", "..-1.5", "11..12"] {
            let error = SecurityFilter::parse(value).unwrap_err();
            assert!(
                error.contains("between -1.0 and 1.0"),
                "{value:?} gave {error:?}"
            );
        }
        // A bare negative never reaches that check, because the ambiguity guard answers it first.
        assert!(
            SecurityFilter::parse("-1.5")
                .unwrap_err()
                .contains("ambiguous")
        );
    }

    #[test]
    fn a_bound_that_is_not_a_number_is_refused() {
        for value in ["abc", "0.5..abc", "abc..0.5", "0.5...0.7", "0.5.0.7"] {
            assert!(SecurityFilter::parse(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn the_displayed_tenths_of_a_level_are_unchanged() {
        assert_eq!(displayed_security(0.0), 0);
        assert_eq!(displayed_security(0.5), 5);
        assert_eq!(displayed_security(-0.5), -5);
        assert_eq!(displayed_security(1.0), 10);
    }
}

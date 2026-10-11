/// Insert thousands separators into a string of digits.
pub fn group_digits(digits: &str) -> String {
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    grouped
}

/// Format a number with thousands separators and the given number of decimals.
pub fn format_number(value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    let sign = if value < 0.0 { "-" } else { "" };
    match text.split_once('.') {
        Some((int_part, frac_part)) => format!("{}{}.{}", sign, group_digits(int_part), frac_part),
        None => format!("{}{}", sign, group_digits(&text)),
    }
}

/// [`format_number`] for a value that may be unknown, which reads `n/a`.
pub fn format_optional(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format_number(value, decimals))
}

/// Format an ISK amount, shown in millions or billions when large enough.
pub fn format_isk(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1e9 {
        format!("{:.2} billion ISK", value / 1e9)
    } else if magnitude >= 1e6 {
        format!("{:.2} million ISK", value / 1e6)
    } else {
        format!("{} ISK", format_number(value, 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_number_groups_thousands() {
        assert_eq!(format_number(1234567.891, 2), "1,234,567.89");
        assert_eq!(format_number(-1234.5, 1), "-1,234.5");
        assert_eq!(format_number(999.0, 0), "999");
    }

    #[test]
    fn an_unknown_value_reads_n_a() {
        assert_eq!(format_optional(None, 2), "n/a");
        assert_eq!(format_optional(Some(1234.5), 2), "1,234.50");
    }

    #[test]
    fn format_isk_uses_the_largest_fitting_unit() {
        assert_eq!(format_isk(2_500_000_000.0), "2.50 billion ISK");
        assert_eq!(format_isk(3_000_000.0), "3.00 million ISK");
        assert_eq!(format_isk(999.5), "999.50 ISK");
    }
}

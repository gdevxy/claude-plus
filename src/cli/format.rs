const CENTS: usize = 2;
const SUB_CENTS: usize = 4;

pub fn usd(value: f64) -> String {
    usd_with(value, column_decimals(&[value]))
}

pub fn column_decimals(values: &[f64]) -> usize {
    let reaches_a_dollar = values
        .iter()
        .any(|value| (value.abs() * 10_000.0).round() >= 10_000.0);
    if reaches_a_dollar { CENTS } else { SUB_CENTS }
}

pub fn usd_with(value: f64, decimals: usize) -> String {
    let rounded = format!("{:.decimals$}", value.abs());
    let (whole, fraction) = rounded.split_once('.').unwrap_or((&rounded, ""));
    let body = format!("{}.{fraction}", count(whole.parse().unwrap_or_default()));
    let is_zero = fraction.bytes().all(|b| b == b'0') && whole == "0";
    let sign = if value < 0.0 && !is_zero { "-" } else { "" };
    format!("{sign}${body}")
}

pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn plural(value: u64, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        count(value),
        if value == 1 { singular } else { plural }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_uses_four_decimals_below_a_dollar() {
        assert_eq!(usd(0.06404), "$0.0640");
        assert_eq!(usd(12.345), "$12.35");
    }

    #[test]
    fn large_money_has_thousands_separators() {
        assert_eq!(usd(6461.78), "$6,461.78");
        assert_eq!(usd(1_234_567.0), "$1,234,567.00");
        assert_eq!(usd(-2452.26), "-$2,452.26");
    }

    #[test]
    fn precision_is_chosen_after_rounding() {
        assert_eq!(usd(0.99996), "$1.00");
        assert_eq!(usd(0.99994), "$0.9999");
    }

    #[test]
    fn negative_money_always_has_the_sign_first() {
        assert_eq!(usd(-0.5), "-$0.5000");
        assert_eq!(usd(-0.00001), "$0.0000");
    }

    #[test]
    fn a_column_uses_two_decimals_once_any_value_reaches_a_dollar() {
        assert_eq!(column_decimals(&[0.5, -0.25, 0.0]), 4);
        assert_eq!(column_decimals(&[0.5, 1.0]), 2);
        assert_eq!(column_decimals(&[-1.5, 0.01]), 2);
        assert_eq!(column_decimals(&[0.99996]), 2);
        assert_eq!(column_decimals(&[0.99994]), 4);
        assert_eq!(column_decimals(&[]), 4);
        assert_eq!(usd_with(0.0011, 2), "$0.00");
        assert_eq!(usd_with(1234.5, 2), "$1,234.50");
        assert_eq!(usd_with(2.0, 4), "$2.0000");
        assert_eq!(usd_with(-0.001, 2), "$0.00");
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(1284), "1,284");
        assert_eq!(count(12), "12");
        assert_eq!(count(1_000_000), "1,000,000");
        assert_eq!(plural(1, "line", "lines"), "1 line");
        assert_eq!(plural(2, "line", "lines"), "2 lines");
    }
}

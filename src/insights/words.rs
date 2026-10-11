const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const FAMILIES: &[&str] = &["opus", "sonnet", "haiku", "fable", "mythos"];
const TOKEN_UNITS: [char; 4] = ['k', 'M', 'B', 'T'];

pub fn money(usd: f64) -> String {
    if usd == 0.0 {
        return "$0.00".into();
    }
    if usd < 0.01 {
        return "<$0.01".into();
    }
    if usd < 1000.0 {
        return format!("${usd:.2}");
    }
    format!("${}", grouped(usd.round() as u64))
}

pub fn tokens(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    scaled(count as f64 / 1000.0, 0)
}

pub fn duration(ms: i64) -> String {
    if ms < MINUTE_MS {
        return "<1m".into();
    }
    if ms < HOUR_MS {
        return format!("{}m", ms / MINUTE_MS);
    }
    if ms < DAY_MS {
        let minutes = ms % HOUR_MS / MINUTE_MS;
        return match minutes {
            0 => format!("{}h", ms / HOUR_MS),
            _ => format!("{}h {minutes}m", ms / HOUR_MS),
        };
    }
    match ms % DAY_MS / HOUR_MS {
        0 => format!("{}d", ms / DAY_MS),
        hours => format!("{}d {hours}h", ms / DAY_MS),
    }
}

pub fn model_name(id: &str) -> String {
    let base = crate::pricing::normalize_model(id);
    let parts: Vec<&str> = base.split('-').collect();
    let Some(at) = parts.iter().position(|part| FAMILIES.contains(part)) else {
        return id.to_string();
    };
    let family = parts[at];
    let version: Vec<&str> = parts[at + 1..]
        .iter()
        .copied()
        .filter(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    let name = format!("{}{}", family[..1].to_uppercase(), &family[1..]);
    match version.is_empty() {
        true => name,
        false => format!("{name} {}", version.join(".")),
    }
}

pub fn count(value: u64, one: &str, many: &str) -> String {
    format!("{} {}", grouped(value), if value == 1 { one } else { many })
}

pub fn times(value: u64) -> String {
    match value {
        1 => "once".into(),
        2 => "twice".into(),
        _ => format!("{} times", grouped(value)),
    }
}

fn scaled(value: f64, step: usize) -> String {
    let rounded = if value < 100.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    };
    if rounded >= 1000.0 && step < TOKEN_UNITS.len() - 1 {
        return scaled(value / 1000.0, step + 1);
    }
    let unit = TOKEN_UNITS[step];
    if rounded.fract() == 0.0 {
        format!("{}{unit}", rounded as u64)
    } else {
        format!("{rounded:.1}{unit}")
    }
}

pub fn grouped(value: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_like_the_pane() {
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(0.004), "<$0.01");
        assert_eq!(money(4.183), "$4.18");
        assert_eq!(money(1234.6), "$1,235");
    }

    #[test]
    fn tokens_read_like_the_pane() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(4_100), "4.1k");
        assert_eq!(tokens(46_000), "46k");
        assert_eq!(tokens(151_400), "151k");
        assert_eq!(tokens(999_700), "1M");
        assert_eq!(tokens(1_250_000), "1.3M");
    }

    #[test]
    fn tokens_past_a_million_scale_on_to_b_and_t() {
        let read: Vec<String> = [
            999_960_000,
            1_178_000_000,
            18_408_000_000,
            124_500_000_000,
            2_400_000_000_000,
            1_234_000_000_000_000,
        ]
        .into_iter()
        .map(tokens)
        .collect();
        assert_eq!(read, ["1B", "1.2B", "18.4B", "125B", "2.4T", "1234T"]);
    }

    #[test]
    fn durations_read_like_the_pane() {
        assert_eq!(duration(30_000), "<1m");
        assert_eq!(duration(52 * MINUTE_MS), "52m");
        assert_eq!(duration(HOUR_MS), "1h");
        assert_eq!(duration(70 * MINUTE_MS), "1h 10m");
        assert_eq!(duration(DAY_MS + 3 * HOUR_MS), "1d 3h");
    }

    #[test]
    fn model_names_read_like_the_pane() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(model_name("claude-sonnet-5-5[1m]"), "Sonnet 5.5");
        assert_eq!(model_name("gpt-x"), "gpt-x");
    }

    #[test]
    fn counts_say_once_twice_and_plurals() {
        assert_eq!(times(1), "once");
        assert_eq!(times(2), "twice");
        assert_eq!(times(1200), "1,200 times");
        assert_eq!(count(1, "session", "sessions"), "1 session");
        assert_eq!(count(3, "session", "sessions"), "3 sessions");
    }
}

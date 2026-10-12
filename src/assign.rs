use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Days, NaiveDate, TimeZone};

use crate::history::calendar::local_midnight;
use crate::store::Candidate;

#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    project: Option<String>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub sessions: Vec<String>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub spend_usd: f64,
}

impl Filter {
    pub fn new<Tz: TimeZone>(
        project: Option<String>,
        since: Option<&str>,
        until: Option<&str>,
        tz: &Tz,
    ) -> Result<Self> {
        if project.is_none() && since.is_none() && until.is_none() {
            bail!("pass --project, --since or --until to choose the sessions");
        }
        let since = since.map(|text| day("--since", text)).transpose()?;
        let until = until.map(|text| day("--until", text)).transpose()?;
        if let (Some(since), Some(until)) = (since, until)
            && until < since
        {
            bail!("--until {until} is before --since {since}");
        }
        let since_ts = since.map(|day| local_midnight(day, tz)).transpose()?;
        let until_ts = until
            .map(|day| {
                let next = day
                    .checked_add_days(Days::new(1))
                    .context("--until is the last supported day")?;
                local_midnight(next, tz)
            })
            .transpose()?;
        Ok(Self {
            project,
            since_ts,
            until_ts,
        })
    }

    pub fn keeps(&self, candidate: &Candidate) -> bool {
        let in_project = self.project.as_deref().is_none_or(|pattern| {
            candidate
                .cwd
                .as_deref()
                .is_some_and(|cwd| glob_matches(pattern, cwd))
        });
        let undated = self.since_ts.is_none() && self.until_ts.is_none();
        let in_days = undated
            || candidate.started_at.is_some_and(|started| {
                self.since_ts.is_none_or(|since| started >= since)
                    && self.until_ts.is_none_or(|until| started < until)
            });
        in_project && in_days
    }
}

impl Selection {
    pub fn days<Tz: TimeZone>(&self, tz: &Tz) -> Option<(NaiveDate, NaiveDate)> {
        let local =
            |ts: i64| DateTime::from_timestamp(ts, 0).map(|utc| utc.with_timezone(tz).date_naive());
        Some((local(self.first_ts?)?, local(self.last_ts?)?))
    }
}

pub fn select(candidates: Vec<Candidate>, filter: &Filter) -> Selection {
    let kept: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| filter.keeps(candidate))
        .collect();
    Selection {
        first_ts: kept.iter().filter_map(|c| c.started_at).min(),
        last_ts: kept.iter().filter_map(|c| c.started_at).max(),
        spend_usd: kept.iter().map(|c| c.spend_usd).sum(),
        sessions: kept.into_iter().map(|c| c.id).collect(),
    }
}

pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if pattern.get(p) == Some(&'*') {
            star = Some((p, t));
            p += 1;
        } else if pattern.get(p) == Some(&text[t]) {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|ch| *ch == '*')
}

pub fn expand_home(pattern: &str, home: &Path) -> String {
    match pattern.strip_prefix('~') {
        Some("") => home.display().to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.display()),
        _ => pattern.to_string(),
    }
}

fn day(flag: &str, text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| anyhow!("{flag} takes a day as YYYY-MM-DD, not {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    fn at(text: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp()
    }

    fn session(id: &str, cwd: Option<&str>, started: Option<&str>, spend_usd: f64) -> Candidate {
        Candidate {
            id: id.into(),
            cwd: cwd.map(String::from),
            started_at: started.map(at),
            spend_usd,
        }
    }

    fn project(pattern: &str) -> Filter {
        Filter::new(Some(pattern.into()), None, None, &Utc).unwrap()
    }

    #[test]
    fn a_star_matches_any_characters_slashes_included() {
        assert!(glob_matches("*/private/*", "/Users/me/private/notes/app"));
        assert!(glob_matches("/Users/me/*", "/Users/me/a/b/c"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("/w/*-api", "/w/x/billing-api"));
        assert!(glob_matches("/w/demo", "/w/demo"));
        assert!(!glob_matches("/w/demo", "/w/demo/sub"));
        assert!(!glob_matches("/w/*-api", "/w/billing-api/src"));
        assert!(!glob_matches("*/private/*", "/Users/me/privateer"));
        assert!(glob_matches("/w/a?b[c]", "/w/a?b[c]"));
    }

    #[test]
    fn a_session_without_a_working_directory_never_matches_a_project() {
        assert!(!project("*").keeps(&session("s", None, Some("2026-10-01T00:00:00Z"), 0.0)));
        assert!(project("*").keeps(&session("s", Some("/x"), None, 0.0)));
    }

    #[test]
    fn days_are_local_and_both_bounds_are_inclusive() {
        let paris = FixedOffset::east_opt(2 * 3600).unwrap();
        let filter = Filter::new(None, Some("2026-10-02"), Some("2026-10-03"), &paris).unwrap();
        let keeps = |started: &str| filter.keeps(&session("s", Some("/x"), Some(started), 0.0));
        assert!(!keeps("2026-10-01T21:59:59Z"));
        assert!(keeps("2026-10-01T22:30:00Z"));
        assert!(keeps("2026-10-03T21:59:59Z"));
        assert!(!keeps("2026-10-03T22:00:00Z"));
        assert!(!filter.keeps(&session("s", Some("/x"), None, 0.0)));
        let one_day = Filter::new(None, Some("2026-10-02"), Some("2026-10-02"), &paris).unwrap();
        assert!(one_day.keeps(&session("s", None, Some("2026-10-02T21:00:00Z"), 0.0)));
    }

    #[test]
    fn a_filter_needs_a_project_or_a_day_and_valid_ordered_dates() {
        let error = |project: Option<&str>, since: Option<&str>, until: Option<&str>| {
            Filter::new(project.map(String::from), since, until, &Utc)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            error(None, None, None),
            "pass --project, --since or --until to choose the sessions"
        );
        assert_eq!(
            error(None, Some("2026-13-01"), None),
            "--since takes a day as YYYY-MM-DD, not 2026-13-01"
        );
        assert_eq!(
            error(None, None, Some("yesterday")),
            "--until takes a day as YYYY-MM-DD, not yesterday"
        );
        assert_eq!(
            error(None, Some("2026-10-05"), Some("2026-10-04")),
            "--until 2026-10-04 is before --since 2026-10-05"
        );
        assert!(Filter::new(None, None, Some("2026-10-04"), &Utc).is_ok());
    }

    #[test]
    fn a_selection_counts_its_sessions_days_and_spend() {
        let candidates = vec![
            session("a", Some("/w/one"), Some("2026-06-03T10:00:00Z"), 1.25),
            session("b", Some("/elsewhere"), Some("2026-07-01T10:00:00Z"), 9.0),
            session("c", Some("/w/two"), None, 0.5),
            session("d", Some("/w/three"), Some("2026-10-11T23:30:00Z"), 2.0),
        ];
        let selection = select(candidates, &project("/w/*"));
        assert_eq!(selection.sessions, vec!["a", "c", "d"]);
        assert_eq!(selection.spend_usd, 3.75);
        let day = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap();
        assert_eq!(
            selection.days(&Utc),
            Some((day("2026-06-03"), day("2026-10-11")))
        );
        let paris = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(
            selection.days(&paris),
            Some((day("2026-06-03"), day("2026-10-12")))
        );
        let undated = select(
            vec![session("c", Some("/w/two"), None, 0.5)],
            &project("/w/*"),
        );
        assert_eq!(undated.days(&Utc), None);
    }

    #[test]
    fn a_leading_tilde_is_the_home_folder() {
        let home = Path::new("/Users/me");
        assert_eq!(expand_home("~/private/*", home), "/Users/me/private/*");
        assert_eq!(expand_home("~", home), "/Users/me");
        assert_eq!(expand_home("/w/~/x", home), "/w/~/x");
        assert_eq!(expand_home("~other/x", home), "~other/x");
    }
}

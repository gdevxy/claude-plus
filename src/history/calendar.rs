use anyhow::{Context, Result};
use chrono::{DateTime, Days, NaiveDate, NaiveTime, TimeDelta, TimeZone, Timelike};

use super::Range;

const MINUTES_PER_DAY: i64 = 24 * 60;
const HOUR_S: i64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: i64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spans {
    pub spans: Vec<Span>,
    pub until: i64,
}

impl Spans {
    pub fn since(&self) -> i64 {
        self.spans.first().map_or(self.until, |span| span.start)
    }

    pub fn index_of(&self, ts: i64) -> Option<usize> {
        if ts < self.since() || ts >= self.until {
            return None;
        }
        Some(self.spans.partition_point(|span| span.start <= ts) - 1)
    }
}

pub fn spans<Tz: TimeZone>(range: Range, now_s: i64, tz: &Tz) -> Result<Spans>
where
    Tz::Offset: std::fmt::Display,
{
    let now = DateTime::from_timestamp(now_s, 0)
        .context("the clock is out of range")?
        .with_timezone(tz);
    match range {
        Range::Day => {
            let hour = now_s - i64::from(now.minute() * 60 + now.second());
            let spans = (0..24)
                .rev()
                .map(|back| {
                    let start = hour - back * HOUR_S;
                    Ok(Span {
                        start,
                        label: label(start, tz, "%H:%M")?,
                    })
                })
                .collect::<Result<_>>()?;
            Ok(Spans {
                spans,
                until: hour + HOUR_S,
            })
        }
        _ => {
            let days = range.days();
            let today = now.date_naive();
            let first = today
                .checked_sub_days(Days::new(days - 1))
                .context("the range reaches before the calendar start")?;
            let spans = (0..days)
                .map(|offset| {
                    let day = first + Days::new(offset);
                    let start = local_midnight(day, tz)?;
                    Ok(Span {
                        start,
                        label: day.format("%b %-d").to_string(),
                    })
                })
                .collect::<Result<_>>()?;
            let tomorrow = today
                .checked_add_days(Days::new(1))
                .context("today is the last supported day")?;
            Ok(Spans {
                spans,
                until: local_midnight(tomorrow, tz)?,
            })
        }
    }
}

fn label<Tz: TimeZone>(ts: i64, tz: &Tz, format: &str) -> Result<String>
where
    Tz::Offset: std::fmt::Display,
{
    Ok(DateTime::from_timestamp(ts, 0)
        .context("the clock is out of range")?
        .with_timezone(tz)
        .format(format)
        .to_string())
}

pub fn local_midnight<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> Result<i64> {
    let midnight = day.and_time(NaiveTime::MIN);
    (0..MINUTES_PER_DAY)
        .find_map(|minute| {
            tz.from_local_datetime(&(midnight + TimeDelta::minutes(minute)))
                .earliest()
        })
        .map(|start| start.timestamp())
        .with_context(|| format!("{day} has no valid local time"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, MappedLocalTime, NaiveDateTime, Offset, Utc};

    #[derive(Debug, Clone, Copy)]
    struct GapAtMidnight;

    const GAP_DAY: (i32, u32, u32) = (2026, 9, 6);

    fn summer() -> FixedOffset {
        FixedOffset::west_opt(3 * 3600).unwrap()
    }

    fn winter() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }

    fn gap_start() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(GAP_DAY.0, GAP_DAY.1, GAP_DAY.2)
            .unwrap()
            .and_time(NaiveTime::MIN)
    }

    impl TimeZone for GapAtMidnight {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            GapAtMidnight
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            self.offset_from_local_datetime(&local.and_time(NaiveTime::MIN))
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            let gap_end = gap_start() + chrono::TimeDelta::hours(1);
            if *local < gap_start() {
                MappedLocalTime::Single(winter())
            } else if *local < gap_end {
                MappedLocalTime::None
            } else {
                MappedLocalTime::Single(summer())
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_time(NaiveTime::MIN))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let transition = gap_start() - winter().fix();
            if *utc < transition {
                winter()
            } else {
                summer()
            }
        }
    }

    fn utc(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp()
    }

    #[test]
    fn local_midnight_skipped_by_a_gap_starts_the_day_when_the_gap_ends() {
        let day = NaiveDate::from_ymd_opt(GAP_DAY.0, GAP_DAY.1, GAP_DAY.2).unwrap();
        assert_eq!(
            local_midnight(day, &GapAtMidnight).unwrap(),
            utc("2026-09-06T04:00:00Z")
        );
        assert_eq!(
            local_midnight(day.pred_opt().unwrap(), &GapAtMidnight).unwrap(),
            utc("2026-09-05T04:00:00Z")
        );
        assert_eq!(
            local_midnight(day.succ_opt().unwrap(), &GapAtMidnight).unwrap(),
            utc("2026-09-07T03:00:00Z")
        );
    }

    fn zurich() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    #[test]
    fn a_day_is_the_last_24_local_hours_ending_with_this_one() {
        let now = utc("2026-10-11T09:20:15Z");
        let day = spans(Range::Day, now, &zurich()).unwrap();
        assert_eq!(day.spans.len(), 24);
        assert_eq!(
            day.spans[0],
            Span {
                start: utc("2026-10-10T10:00:00Z"),
                label: "12:00".into()
            }
        );
        assert_eq!(
            day.spans[23],
            Span {
                start: utc("2026-10-11T09:00:00Z"),
                label: "11:00".into()
            }
        );
        assert_eq!(day.until, utc("2026-10-11T10:00:00Z"));
        let india = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        assert_eq!(
            spans(Range::Day, now, &india).unwrap().until,
            utc("2026-10-11T09:30:00Z")
        );
    }

    #[test]
    fn longer_ranges_are_local_calendar_days_with_today_last() {
        let now = utc("2026-10-11T23:30:00Z");
        let week = spans(Range::Week, now, &zurich()).unwrap();
        assert_eq!(week.spans.len(), 7);
        assert_eq!(
            week.spans[0],
            Span {
                start: utc("2026-10-05T22:00:00Z"),
                label: "Oct 6".into()
            }
        );
        assert_eq!(week.spans[6].label, "Oct 12");
        assert_eq!(week.until, utc("2026-10-12T22:00:00Z"));
        assert_eq!(
            spans(Range::Month, now, &Utc).unwrap().spans[0].label,
            "Sep 12"
        );
        assert_eq!(spans(Range::Quarter, now, &Utc).unwrap().spans.len(), 90);
    }

    #[test]
    fn a_time_falls_in_the_span_that_starts_at_or_before_it() {
        let week = spans(Range::Week, utc("2026-10-11T12:00:00Z"), &Utc).unwrap();
        assert_eq!(week.index_of(utc("2026-10-05T00:00:00Z")), Some(0));
        assert_eq!(week.index_of(utc("2026-10-05T23:59:59Z")), Some(0));
        assert_eq!(week.index_of(utc("2026-10-11T23:59:59Z")), Some(6));
        assert_eq!(week.index_of(utc("2026-10-04T23:59:59Z")), None);
        assert_eq!(week.index_of(utc("2026-10-12T00:00:00Z")), None);
    }
}

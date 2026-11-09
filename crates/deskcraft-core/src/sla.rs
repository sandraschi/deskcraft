//! Service levels in business hours: Mon-Fri 08:00-17:00 local time, Austrian
//! public holidays excluded (craft-biz holidays). Targets per priority.
//!
//! Known simplification: `pending` (waiting on the customer) does not pause the
//! clock yet.

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use craft_biz::holidays::is_working_day;
use deskcraft_store::Priority;

pub const DAY_START_H: u32 = 8;
pub const DAY_END_H: u32 = 17;

/// (first response, resolution) in business minutes.
pub fn targets(p: Priority) -> (i64, i64) {
    match p {
        Priority::Urgent => (60, 8 * 60),
        Priority::High => (4 * 60, 16 * 60),
        Priority::Normal => (8 * 60, 40 * 60),
        Priority::Low => (16 * 60, 80 * 60),
    }
}

fn working(d: NaiveDate) -> bool {
    is_working_day(d).unwrap_or(false)
}

fn window(d: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let s = d.and_time(NaiveTime::from_hms_opt(DAY_START_H, 0, 0)?);
    let e = d.and_time(NaiveTime::from_hms_opt(DAY_END_H, 0, 0)?);
    Some((s, e))
}

/// Business minutes between two instants (0 if end <= start).
pub fn business_minutes(start: NaiveDateTime, end: NaiveDateTime) -> i64 {
    if end <= start {
        return 0;
    }
    let mut total = 0;
    let mut d = start.date();
    // Bounded: two years of calendar days at most.
    for _ in 0..800 {
        if d > end.date() {
            break;
        }
        if working(d) {
            if let Some((ws, we)) = window(d) {
                let a = ws.max(start);
                let b = we.min(end);
                if b > a {
                    total += (b - a).num_minutes();
                }
            }
        }
        d += Duration::days(1);
    }
    total
}

/// The instant `minutes` business minutes after `start`.
pub fn add_business(start: NaiveDateTime, minutes: i64) -> NaiveDateTime {
    let mut left = minutes.max(0);
    let mut d = start.date();
    for _ in 0..800 {
        if working(d) {
            if let Some((ws, we)) = window(d) {
                let a = ws.max(start);
                if we > a {
                    let avail = (we - a).num_minutes();
                    if left <= avail {
                        return a + Duration::minutes(left);
                    }
                    left -= avail;
                }
            }
        }
        d += Duration::days(1);
    }
    start + Duration::minutes(minutes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Met,
    Ok,
    AtRisk,
    Breached,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Met => "met",
            State::Ok => "ok",
            State::AtRisk => "at_risk",
            State::Breached => "breached",
        }
    }
}

/// Evaluate one clock. `done` is when it stopped (response sent, ticket solved).
pub fn clock(
    start: NaiveDateTime,
    done: Option<NaiveDateTime>,
    as_of: NaiveDateTime,
    target: i64,
) -> (State, i64, NaiveDateTime) {
    let due = add_business(start, target);
    match done {
        Some(t) => {
            let used = business_minutes(start, t);
            (
                if used <= target {
                    State::Met
                } else {
                    State::Breached
                },
                used,
                due,
            )
        }
        None => {
            let used = business_minutes(start, as_of);
            let state = if used > target {
                State::Breached
            } else if used * 4 >= target * 3 {
                State::AtRisk
            } else {
                State::Ok
            };
            (state, used, due)
        }
    }
}

pub fn round_to_minute(t: NaiveDateTime) -> NaiveDateTime {
    t.with_second(0)
        .and_then(|x| x.with_nanosecond(0))
        .unwrap_or(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn business_minutes_skip_nights_weekends_and_holidays() {
        // Friday 16:00 -> Monday 09:00 = 60 + 60.
        assert_eq!(business_minutes(at(11, 6, 16, 0), at(11, 9, 9, 0)), 120);
        // Before opening and after closing count nothing.
        assert_eq!(business_minutes(at(11, 9, 6, 0), at(11, 9, 7, 59)), 0);
        assert_eq!(business_minutes(at(11, 9, 17, 0), at(11, 9, 23, 0)), 0);
        // Monday 26.10.2026 is Nationalfeiertag.
        assert_eq!(business_minutes(at(10, 23, 16, 0), at(10, 27, 9, 0)), 120);
    }

    #[test]
    fn due_dates() {
        // Normal first response (8h) from Friday 15:00: 2h Friday + 6h Monday -> Monday 14:00.
        assert_eq!(add_business(at(11, 6, 15, 0), 480), at(11, 9, 14, 0));
        // Mail at 20:00 counts from next morning 08:00.
        assert_eq!(add_business(at(11, 9, 20, 0), 60), at(11, 10, 9, 0));
        // Over the holiday weekend.
        assert_eq!(add_business(at(10, 23, 16, 30), 60), at(10, 27, 8, 30));
    }

    #[test]
    fn clock_states() {
        let start = at(11, 9, 8, 0);
        assert_eq!(
            clock(start, Some(at(11, 9, 8, 50)), at(11, 9, 12, 0), 60).0,
            State::Met
        );
        assert_eq!(clock(start, None, at(11, 9, 8, 30), 60).0, State::Ok);
        assert_eq!(clock(start, None, at(11, 9, 8, 50), 60).0, State::AtRisk);
        assert_eq!(clock(start, None, at(11, 9, 9, 30), 60).0, State::Breached);
        assert_eq!(
            clock(start, Some(at(11, 9, 10, 0)), at(11, 9, 12, 0), 60).0,
            State::Breached
        );
    }
}

//! Dates and times in the user's zone, without `core::fmt`.
//!
//! "Local seconds" are Unix seconds shifted by the UTC offset, so that
//! `local / 86400` is the local day number and `local % 86400` the time of day.

use super::*;

pub const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The local zone's offset from UTC in seconds.
pub fn utc_offset() -> i64 {
    env().utc_offset as i64
}

/// Unix seconds → local seconds.
pub fn local(unix_secs: i64) -> i64 {
    unix_secs + utc_offset()
}

/// The current local time.
pub fn now() -> i64 {
    local(now_ms().div_euclid(1000))
}

/// The local day number (days since 1970-01-01).
pub fn day(local: i64) -> i64 {
    local.div_euclid(86_400)
}

/// 0 Monday … 6 Sunday.
pub fn weekday(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

/// Day number → (year, month 1–12, day 1–31). Howard Hinnant's algorithm.
pub fn civil(day: i64) -> (i64, u32, u32) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m as u32, d as u32)
}

/// (year, month, day) → day number.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn pad2(out: &mut String, v: i64) {
    if v < 10 {
        out.push('0');
    }
    out.push_str(&itoa(v));
}

/// "09:05" (24-hour).
pub fn hhmm(local: i64) -> String {
    let m = local.rem_euclid(86_400) / 60;
    let mut s = String::with_capacity(5);
    pad2(&mut s, m / 60);
    s.push(':');
    pad2(&mut s, m % 60);
    s
}

/// "2026-11-12".
pub fn ymd(day: i64) -> String {
    let (y, m, d) = civil(day);
    let mut s = String::from(&*itoa(y));
    s.push('-');
    pad2(&mut s, m as i64);
    s.push('-');
    pad2(&mut s, d as i64);
    s
}

/// "Nov 12", plus ", 2025" when the year differs from `today`'s.
pub fn short_date(day: i64, today: i64) -> String {
    let (y, m, d) = civil(day);
    let mut s = cat(&[MONTHS[(m - 1) as usize], " ", &itoa(d as i64)]);
    if y != civil(today).0 {
        s.push_str(", ");
        s.push_str(&itoa(y));
    }
    s
}

/// "Today", "Yesterday", or "Wednesday, Nov 12" (with the year if it isn't this year's).
pub fn day_label(day: i64, today: i64) -> String {
    match today - day {
        0 => String::from("Today"),
        1 => String::from("Yesterday"),
        _ => cat(&[DAYS[weekday(day)], ", ", &short_date(day, today)]),
    }
}

/// "just now", "5m", "3h", "2d", then `short_date`.
pub fn ago(local: i64, now: i64) -> String {
    let s = now - local;
    let n = |v: i64, u: &str| cat(&[&itoa(v), u]);
    match s {
        i64::MIN..=59 => String::from("just now"),
        60..=3599 => n(s / 60, "m"),
        3600..=86_399 => n(s / 3600, "h"),
        86_400..=604_799 => n(s / 86_400, "d"),
        _ => short_date(day(local), day(now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(weekday(0), 3); // Thursday
        let d = days_from_civil(2026, 11, 12);
        assert_eq!(civil(d), (2026, 11, 12));
        assert_eq!(ymd(d), "2026-11-12");
        assert_eq!(day_label(d - 2, d), "Tuesday, Nov 10");
        assert_eq!(day_label(days_from_civil(2025, 2, 28), d), "Friday, Feb 28, 2025");
        assert_eq!(hhmm(9 * 3600 + 5 * 60 + 59), "09:05");
        assert_eq!(hhmm(-60), "23:59");
    }
}

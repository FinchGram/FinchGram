//! Times of messages in local time, taken apart for the pages: which day it is relative to today,
//! the clock time, the date. The words ("Yesterday", "Friday") are in ui/format.slint, so that they
//! are translated with the rest of the UI.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Timelike};

use crate::{Day, Moment};

/// `unix` (seconds, as TDLib gives dates) in local time.
pub fn moment(unix: i32) -> Moment {
    moment_at(unix, Local::now())
}

fn moment_at(unix: i32, now: DateTime<Local>) -> Moment {
    let Some(time) = Local.timestamp_opt(i64::from(unix), 0).single() else {
        return Moment::default();
    };
    let days = (now.date_naive() - time.date_naive()).num_days();
    let day = match days {
        ..=0 => Day::Today, // also a clock that is a little ahead
        1 => Day::Yesterday,
        2..=6 => Day::ThisWeek,
        _ if time.year() == now.year() => Day::ThisYear,
        _ => Day::Earlier,
    };
    Moment {
        day,
        hour: time.hour() as i32,
        minute: time.minute() as i32,
        weekday: time.weekday().number_from_monday() as i32,
        month: time.month() as i32,
        date: time.day() as i32,
        year: time.year(),
    }
}

/// "14:20"
pub fn clock(unix: i32) -> String {
    match Local.timestamp_opt(i64::from(unix), 0).single() {
        Some(time) => format!("{:02}:{:02}", time.hour(), time.minute()),
        None => String::new(),
    }
}

/// The local day of `unix`, to know where a new day starts in a chat.
pub fn day(unix: i32) -> Option<NaiveDate> {
    Local.timestamp_opt(i64::from(unix), 0).single().map(|time| time.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, date: u32, hour: u32, minute: u32) -> (i32, DateTime<Local>) {
        let time = Local.with_ymd_and_hms(year, month, date, hour, minute, 0).single().expect("a local time");
        (time.timestamp() as i32, time)
    }

    #[test]
    fn days_are_named_relative_to_today() {
        let (_, now) = at(2026, 9, 25, 12, 0); // a Friday
        let (today, _) = at(2026, 9, 25, 9, 12);
        let (yesterday, _) = at(2026, 9, 24, 23, 59);
        let (monday, _) = at(2026, 9, 21, 8, 0);
        let (summer, _) = at(2026, 7, 3, 8, 0);
        let (last_year, _) = at(2025, 12, 31, 8, 0);

        let moment = moment_at(today, now);
        assert_eq!((moment.day, moment.hour, moment.minute), (Day::Today, 9, 12));
        assert_eq!(moment_at(yesterday, now).day, Day::Yesterday);
        let moment = moment_at(monday, now);
        assert_eq!((moment.day, moment.weekday), (Day::ThisWeek, 1));
        let moment = moment_at(summer, now);
        assert_eq!((moment.day, moment.month, moment.date), (Day::ThisYear, 7, 3));
        assert_eq!(moment_at(last_year, now).day, Day::Earlier);
    }
}

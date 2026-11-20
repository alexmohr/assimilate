// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use croner::Cron;
use jiff::{
    SignedDuration,
    civil::DateTime,
    tz::{AmbiguousOffset, TimeZone},
};

use crate::time::Timestamp;

/// Upper bound on how far a local time is rolled forward out of a DST gap.
/// Regular DST gaps are one hour; the largest real-world gap (Samoa skipping
/// 2011-12-30 entirely) is a full day.
const MAX_GAP_MINUTES: i64 = 25 * 60;

/// Calculate the next run time for a cron expression, evaluating in the given
/// timezone. The cron expression is interpreted in the target timezone (e.g.,
/// "0 2 * * *" means 02:00 in `tz`), and the result is returned as UTC.
///
/// Daylight-saving transitions are resolved so no occurrence is skipped:
/// * A local time that falls into a spring-forward gap (e.g. 02:30 in
///   `Europe/Berlin` on the last Sunday of March) is rolled forward to the
///   first valid local time after the gap (03:00).
/// * A local time that occurs twice during a fall-back transition resolves to
///   its earliest instant, so the job runs once, before the clocks go back.
///
/// # Errors
///
/// Returns an error if `cron_expression` fails to parse, if no next occurrence
/// exists, or if the computed local time cannot be mapped to an instant in
/// `tz`.
pub fn calculate_next_run(
    cron_expression: &str,
    from: Timestamp,
    tz: &TimeZone,
) -> Result<Timestamp, String> {
    let cron = cron_expression
        .parse::<Cron>()
        .map_err(|e| format!("invalid cron expression '{cron_expression}': {e}"))?;

    // The target timezone's wall-clock reading of `from`. The cron expression
    // is evaluated against plain civil times, so the search never sees a DST
    // transition; those are resolved afterwards.
    let local_now = from.to_zoned(tz.clone()).datetime();
    let next_local = cron
        .find_next_occurrence(&local_now, false)
        .map_err(|e| format!("no next occurrence for '{cron_expression}': {e}"))?;

    // Interpret the result as a local time in the target timezone, convert to UTC
    resolve_local_time(next_local, tz).ok_or_else(|| {
        format!(
            "local time {next_local} for '{cron_expression}' does not exist in timezone {}",
            tz.iana_name().unwrap_or("unknown")
        )
    })
}

/// Map a local wall-clock time to UTC, rolling forward to the first valid
/// minute when `local` lies in a DST gap.
fn resolve_local_time(local: DateTime, tz: &TimeZone) -> Option<Timestamp> {
    (0..=MAX_GAP_MINUTES).find_map(|minutes| {
        local
            .checked_add(SignedDuration::from_mins(minutes))
            .ok()
            .and_then(|candidate| {
                // A gap has no instant at all, which is what rolls the search forward;
                // a fold has two, of which the earlier one is the `before` offset.
                let offset = match tz.to_ambiguous_timestamp(candidate).offset() {
                    AmbiguousOffset::Unambiguous { offset }
                    | AmbiguousOffset::Fold { before: offset, .. } => offset,
                    AmbiguousOffset::Gap { .. } => return None,
                };
                offset.to_timestamp(candidate).ok()
            })
            .map(Timestamp::from)
    })
}

/// # Errors
///
/// Returns an error if `expression` is not a valid cron expression.
pub fn validate_cron(expression: &str) -> Result<(), String> {
    expression
        .parse::<Cron>()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    Ok(())
}

/// Parse a timezone string (IANA format like "Europe/Berlin") into a [`TimeZone`].
/// When the string is empty, detects the system's local timezone.
/// Returns UTC only if the system timezone cannot be determined.
///
/// # Errors
///
/// Returns an error if `tz_str` is not empty, not `"utc"`, and not a valid IANA
/// timezone name.
pub fn parse_timezone(tz_str: &str) -> Result<TimeZone, String> {
    if tz_str.eq_ignore_ascii_case("utc") {
        return Ok(TimeZone::UTC);
    }
    if tz_str.is_empty() {
        return Ok(detect_system_timezone());
    }
    TimeZone::get(tz_str).map_err(|e| format!("invalid timezone '{tz_str}': {e}"))
}

fn detect_system_timezone() -> TimeZone {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| TimeZone::get(&name).ok())
        .unwrap_or(TimeZone::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i16, mo: i8, d: i8, h: i8, m: i8) -> Timestamp {
        jiff::civil::datetime(y, mo, d, h, m, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .into()
    }

    fn tz(name: &str) -> TimeZone {
        TimeZone::get(name).unwrap()
    }

    #[test]
    fn hourly_cron() {
        let from = utc(2026, 1, 1, 10, 0);
        let next = calculate_next_run("0 */6 * * *", from, &TimeZone::UTC).unwrap();
        assert_eq!(next, utc(2026, 1, 1, 12, 0));
    }

    #[test]
    fn daily_cron_in_timezone_summer() {
        let from = utc(2026, 6, 30, 23, 0);
        let tz = tz("Europe/Berlin");
        let next = calculate_next_run("0 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 7, 1, 0, 0));
    }

    #[test]
    fn daily_cron_in_timezone() {
        let from = utc(2026, 1, 1, 0, 0);
        let tz = tz("Europe/Berlin");
        let next = calculate_next_run("0 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 1, 1, 1, 0));
    }

    #[test]
    fn weekly_cron() {
        let from = utc(2026, 1, 5, 8, 0);
        let next = calculate_next_run("0 10 * * 3", from, &TimeZone::UTC).unwrap();
        assert_eq!(next, utc(2026, 1, 7, 10, 0));
    }

    #[test]
    fn spring_forward_gap_rolls_forward_to_end_of_gap() {
        // 2026-03-29: Europe/Berlin jumps 02:00 CET -> 03:00 CEST, so 02:30 does
        // not exist. The job must run at 03:00 CEST (01:00 UTC), not be skipped.
        let tz = tz("Europe/Berlin");
        let from = utc(2026, 3, 28, 23, 0);
        let next = calculate_next_run("30 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 3, 29, 1, 0));
    }

    #[test]
    fn spring_forward_gap_does_not_affect_following_day() {
        let tz = tz("Europe/Berlin");
        let from = utc(2026, 3, 29, 1, 0);
        let next = calculate_next_run("30 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 3, 30, 0, 30));
    }

    #[test]
    fn spring_forward_gap_in_new_york() {
        // 2026-03-08: America/New_York jumps 02:00 EST -> 03:00 EDT.
        let tz = tz("America/New_York");
        let from = utc(2026, 3, 8, 5, 0);
        let next = calculate_next_run("15 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 3, 8, 7, 0));
    }

    #[test]
    fn fall_back_ambiguous_time_uses_earliest_instant() {
        // 2026-10-25: Europe/Berlin falls back 03:00 CEST -> 02:00 CET, so 02:30
        // occurs twice. The first occurrence (CEST, 00:30 UTC) is chosen.
        let tz = tz("Europe/Berlin");
        let from = utc(2026, 10, 24, 23, 0);
        let next = calculate_next_run("30 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 10, 25, 0, 30));
    }

    #[test]
    fn fall_back_runs_once_per_day() {
        // After the first 02:30 (CEST) the next run is the following day,
        // not the repeated 02:30 CET an hour later.
        let tz = tz("Europe/Berlin");
        let from = utc(2026, 10, 25, 0, 30);
        let next = calculate_next_run("30 2 * * *", from, &tz).unwrap();
        assert_eq!(next, utc(2026, 10, 26, 1, 30));
    }

    #[test]
    fn invalid_cron() {
        let result = calculate_next_run("invalid", utc(2026, 1, 1, 0, 0), &TimeZone::UTC);
        assert!(result.is_err());
    }

    #[test]
    fn a_cron_that_never_comes_due_has_no_next_run() {
        // February 31st does not exist, so the expression never matches.
        let result = calculate_next_run("0 0 31 2 *", utc(2026, 1, 1, 0, 0), &TimeZone::UTC);
        assert!(result.is_err());
    }

    #[test]
    fn validate_valid() {
        assert!(validate_cron("0 */6 * * *").is_ok());
        assert!(validate_cron("30 2 * * 1").is_ok());
    }

    #[test]
    fn validate_invalid() {
        assert!(validate_cron("not a cron").is_err());
    }

    #[test]
    fn parse_timezone_valid() {
        assert_eq!(
            parse_timezone("Europe/Berlin").unwrap(),
            tz("Europe/Berlin")
        );
        assert_eq!(parse_timezone("UTC").unwrap(), TimeZone::UTC);
        assert_eq!(
            parse_timezone("America/New_York").unwrap(),
            tz("America/New_York")
        );
    }

    #[test]
    fn parse_timezone_empty_detects_system() {
        let tz = parse_timezone("").unwrap();
        assert!(tz.iana_name().is_some_and(|name| !name.is_empty()));
    }

    #[test]
    fn parse_timezone_invalid() {
        assert!(parse_timezone("Not/A/Zone").is_err());
    }
}

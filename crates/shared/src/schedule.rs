// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

#[cfg(test)]
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
pub use domain::schedule::{calculate_next_run, validate_cron};

/// Parse a timezone string (IANA format like "Europe/Berlin") into a `Tz`.
/// When the string is empty, detects the system's local timezone.
/// Returns UTC only if the system timezone cannot be determined.
///
/// # Errors
///
/// Returns an error if `tz_str` is not empty, not `"utc"`, and not a valid IANA
/// timezone name.
pub fn parse_timezone(tz_str: &str) -> Result<Tz, String> {
    if tz_str.eq_ignore_ascii_case("utc") {
        return Ok(chrono_tz::UTC);
    }
    if tz_str.is_empty() {
        return Ok(detect_system_timezone());
    }
    tz_str
        .parse::<Tz>()
        .map_err(|e| format!("invalid timezone '{tz_str}': {e}"))
}

fn detect_system_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse::<Tz>().ok())
        .unwrap_or(chrono_tz::UTC)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, 0).unwrap()
    }

    #[test]
    fn hourly_cron() {
        let from = utc(2026, 1, 1, 10, 0);
        let next = calculate_next_run("0 */6 * * *", from, chrono_tz::UTC).unwrap();
        assert_eq!(next, utc(2026, 1, 1, 12, 0));
    }

    #[test]
    fn daily_cron_in_timezone_summer() {
        let from = utc(2026, 6, 30, 23, 0);
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let next = calculate_next_run("0 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 7, 1, 0, 0));
    }

    #[test]
    fn daily_cron_in_timezone() {
        let from = utc(2026, 1, 1, 0, 0);
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let next = calculate_next_run("0 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 1, 1, 1, 0));
    }

    #[test]
    fn weekly_cron() {
        let from = utc(2026, 1, 5, 8, 0);
        let next = calculate_next_run("0 10 * * 3", from, chrono_tz::UTC).unwrap();
        assert_eq!(next, utc(2026, 1, 7, 10, 0));
    }

    #[test]
    fn spring_forward_gap_rolls_forward_to_end_of_gap() {
        // 2026-03-29: Europe/Berlin jumps 02:00 CET -> 03:00 CEST, so 02:30 does
        // not exist. The job must run at 03:00 CEST (01:00 UTC), not be skipped.
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let from = utc(2026, 3, 28, 23, 0);
        let next = calculate_next_run("30 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 3, 29, 1, 0));
    }

    #[test]
    fn spring_forward_gap_does_not_affect_following_day() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let from = utc(2026, 3, 29, 1, 0);
        let next = calculate_next_run("30 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 3, 30, 0, 30));
    }

    #[test]
    fn spring_forward_gap_in_new_york() {
        // 2026-03-08: America/New_York jumps 02:00 EST -> 03:00 EDT.
        let tz: Tz = "America/New_York".parse().unwrap();
        let from = utc(2026, 3, 8, 5, 0);
        let next = calculate_next_run("15 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 3, 8, 7, 0));
    }

    #[test]
    fn fall_back_ambiguous_time_uses_earliest_instant() {
        // 2026-10-25: Europe/Berlin falls back 03:00 CEST -> 02:00 CET, so 02:30
        // occurs twice. The first occurrence (CEST, 00:30 UTC) is chosen.
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let from = utc(2026, 10, 24, 23, 0);
        let next = calculate_next_run("30 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 10, 25, 0, 30));
    }

    #[test]
    fn fall_back_runs_once_per_day() {
        // After the first 02:30 (CEST) the next run is the following day,
        // not the repeated 02:30 CET an hour later.
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let from = utc(2026, 10, 25, 0, 30);
        let next = calculate_next_run("30 2 * * *", from, tz).unwrap();
        assert_eq!(next, utc(2026, 10, 26, 1, 30));
    }

    #[test]
    fn invalid_cron() {
        let result = calculate_next_run("invalid", utc(2026, 1, 1, 0, 0), chrono_tz::UTC);
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
            chrono_tz::Europe::Berlin
        );
        assert_eq!(parse_timezone("UTC").unwrap(), chrono_tz::UTC);
        assert_eq!(
            parse_timezone("America/New_York").unwrap(),
            chrono_tz::America::New_York
        );
    }

    #[test]
    fn parse_timezone_empty_detects_system() {
        let tz = parse_timezone("").unwrap();
        let tz_name = tz.name();
        assert_ne!(tz_name, "");
    }

    #[test]
    fn parse_timezone_invalid() {
        assert!(parse_timezone("Not/A/Zone").is_err());
    }
}

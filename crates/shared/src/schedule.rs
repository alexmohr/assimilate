// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;

/// Calculate the next run time for a cron expression, evaluating in the given
/// timezone. The cron expression is interpreted in the target timezone (e.g.,
/// "0 2 * * *" means 02:00 in `tz`), and the result is returned as UTC.
///
/// # Errors
/// Returns a human-readable error string if the cron expression is invalid,
/// no next occurrence can be found, or the resulting local time is ambiguous.
pub fn calculate_next_run(
    cron_expression: &str,
    from: DateTime<Utc>,
    tz: Tz,
) -> Result<DateTime<Utc>, String> {
    let cron = Cron::new(cron_expression)
        .parse()
        .map_err(|e| format!("invalid cron expression '{cron_expression}': {e}"))?;

    // Convert current UTC time to the target timezone's local representation
    let local_now = from.with_timezone(&tz).naive_local();

    // Evaluate cron in "fake UTC" space using the local time values
    let fake_utc = DateTime::<Utc>::from_naive_utc_and_offset(local_now, Utc);
    let next_fake = cron
        .find_next_occurrence(&fake_utc, false)
        .map_err(|e| format!("no next occurrence for '{cron_expression}': {e}"))?;

    // Interpret the result as a local time in the target timezone, convert to UTC
    let next_naive: NaiveDateTime = next_fake.naive_utc();
    tz.from_local_datetime(&next_naive)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .ok_or_else(|| {
            format!("ambiguous or invalid local time for '{cron_expression}' in timezone {tz}")
        })
}

/// Validate a cron expression string.
///
/// # Errors
/// Returns a human-readable error string if the expression cannot be parsed.
pub fn validate_cron(expression: &str) -> Result<(), String> {
    Cron::new(expression)
        .parse()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    Ok(())
}

/// Parse a timezone string (IANA format like "Europe/Berlin") into a `Tz`.
/// When the string is empty, detects the system's local timezone.
/// Returns UTC only if the system timezone cannot be determined.
///
/// # Errors
/// Returns a human-readable error string if the timezone string is invalid.
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
        assert!(!tz_name.is_empty());
    }

    #[test]
    fn parse_timezone_invalid() {
        assert!(parse_timezone("Not/A/Zone").is_err());
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use domain::schedule::OffsetZone;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TIMEZONE_OFFSETS: &str = r"
export interface TimezoneOffsets {
  /** The zone's UTC offset in seconds at `epochMinutes` minutes after the Unix epoch. */
  offsetSecondsAt(epochMinutes: number): number
}
";

#[wasm_bindgen]
extern "C" {
    /// The browser's view of a timezone, read from `Intl`, so this module needs
    /// no timezone database of its own.
    pub type TimezoneOffsets;

    /// The zone's UTC offset in seconds at `epoch_minutes` after the Unix epoch.
    #[wasm_bindgen(method, catch, js_name = offsetSecondsAt)]
    fn offset_seconds_at(this: &TimezoneOffsets, epoch_minutes: i32) -> Result<i32, JsValue>;
}

/// Validates a cron expression exactly as the server does when a schedule is
/// saved; see [`domain::schedule::validate_cron`].
///
/// Returns the server's error message, or `undefined` when the expression is valid.
#[must_use]
#[wasm_bindgen(js_name = validateCron)]
pub fn validate_cron(expression: &str) -> Option<String> {
    domain::schedule::validate_cron(expression).err()
}

/// The next `count` runs of `expression` after `from` (RFC 3339) in the zone
/// named `timezone`, computed by the scheduler's own
/// [`domain::schedule::next_runs_in`] so DST gaps and repeats resolve exactly
/// as the schedule will fire. Each run is an RFC 3339 UTC timestamp.
///
/// # Errors
///
/// Fails if `from` is not RFC 3339, `offsets` cannot answer for the zone, or
/// the expression is invalid.
#[wasm_bindgen(js_name = nextCronRuns)]
pub fn next_cron_runs(
    expression: &str,
    from: &str,
    timezone: &str,
    #[wasm_bindgen(unchecked_param_type = "TimezoneOffsets")] offsets: &TimezoneOffsets,
    count: usize,
) -> Result<Vec<String>, JsError> {
    next_runs_rfc3339(
        expression,
        from,
        timezone,
        |epoch_minutes| offsets.offset_seconds_at(epoch_minutes).ok(),
        count,
    )
    .map_err(|e| JsError::new(&e))
}

fn next_runs_rfc3339(
    expression: &str,
    from: &str,
    timezone: &str,
    offset_seconds_at: impl Fn(i32) -> Option<i32>,
    count: usize,
) -> Result<Vec<String>, String> {
    let from = DateTime::parse_from_rfc3339(from)
        .map_err(|e| format!("invalid start time '{from}': {e}"))?
        .with_timezone(&Utc);
    let zone = OffsetZone::new(timezone, |utc: DateTime<Utc>| {
        let epoch_minutes = i32::try_from(utc.timestamp().checked_div_euclid(60)?).ok()?;
        FixedOffset::east_opt(offset_seconds_at(epoch_minutes)?)
    });
    let runs = domain::schedule::next_runs_in(expression, from, &zone, count)?;
    Ok(runs
        .iter()
        .map(|run| run.to_rfc3339_opts(SecondsFormat::Secs, true))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{next_runs_rfc3339, validate_cron};

    #[test]
    fn lists_runs_in_a_fixed_offset_zone_as_utc_timestamps() {
        let two_hours_east = |_| Some(2 * 3600);
        assert_eq!(
            next_runs_rfc3339(
                "0 2 * * *",
                "2025-12-31T12:00:00Z",
                "Test/Plus2",
                two_hours_east,
                2
            )
            .unwrap(),
            vec!["2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"]
        );
    }

    #[test]
    fn rejects_a_start_time_that_is_not_rfc3339() {
        let err = next_runs_rfc3339("0 2 * * *", "yesterday", "UTC", |_| Some(0), 1).unwrap_err();
        assert!(err.starts_with("invalid start time"), "{err}");
    }

    #[test]
    fn fails_when_the_browser_cannot_give_an_offset() {
        let err = next_runs_rfc3339("0 2 * * *", "2026-01-01T00:00:00Z", "Nowhere", |_| None, 1)
            .unwrap_err();
        assert!(err.contains("Nowhere"), "{err}");
    }

    #[test]
    fn validates_like_the_server() {
        assert_eq!(validate_cron("0 2 * * MON-FRI"), None);
        assert!(validate_cron("60 2 * * *").is_some());
    }
}

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
    let from = DateTime::parse_from_rfc3339(from)
        .map_err(|e| JsError::new(&format!("invalid start time '{from}': {e}")))?
        .with_timezone(&Utc);
    let zone = OffsetZone::new(timezone, |utc: DateTime<Utc>| {
        let epoch_minutes = i32::try_from(utc.timestamp().checked_div_euclid(60)?).ok()?;
        FixedOffset::east_opt(offsets.offset_seconds_at(epoch_minutes).ok()?)
    });
    let runs = domain::schedule::next_runs_in(expression, from, &zone, count)
        .map_err(|e| JsError::new(&e))?;
    Ok(runs
        .iter()
        .map(|run| run.to_rfc3339_opts(SecondsFormat::Secs, true))
        .collect())
}

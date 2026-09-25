// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use croner::Cron;

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
    resolve_local_time(next_naive, tz).ok_or_else(|| {
        format!("local time {next_naive} for '{cron_expression}' does not exist in timezone {tz}")
    })
}

/// Map a local wall-clock time to UTC, rolling forward to the first valid
/// minute when `local` lies in a DST gap.
fn resolve_local_time(local: NaiveDateTime, tz: Tz) -> Option<DateTime<Utc>> {
    (0..=MAX_GAP_MINUTES).find_map(|minutes| {
        local
            .checked_add_signed(TimeDelta::minutes(minutes))
            .and_then(|candidate| tz.from_local_datetime(&candidate).earliest())
            .map(|dt| dt.with_timezone(&Utc))
    })
}

/// The next `count` runs of `cron_expression` after `from`, as
/// [`calculate_next_run`] would schedule them one after another. Used for the
/// frontend's "Next:" preview so it can never disagree with the scheduler.
///
/// # Errors
///
/// Returns the first error [`calculate_next_run`] reports.
pub fn next_runs(
    cron_expression: &str,
    from: DateTime<Utc>,
    tz: Tz,
    count: usize,
) -> Result<Vec<DateTime<Utc>>, String> {
    std::iter::successors(
        Some(calculate_next_run(cron_expression, from, tz)),
        |prev| {
            prev.as_ref()
                .ok()
                .map(|&prev| calculate_next_run(cron_expression, prev, tz))
        },
    )
    .take(count)
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, 0).unwrap()
    }

    #[test]
    fn next_runs_follow_each_other() {
        let runs = next_runs("0 */6 * * *", utc(2026, 1, 1, 10, 0), chrono_tz::UTC, 3).unwrap();
        assert_eq!(
            runs,
            vec![
                utc(2026, 1, 1, 12, 0),
                utc(2026, 1, 1, 18, 0),
                utc(2026, 1, 2, 0, 0)
            ]
        );
    }

    #[test]
    fn next_runs_cross_a_dst_gap_like_the_scheduler() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let runs = next_runs("30 2 * * *", utc(2026, 3, 28, 0, 0), tz, 3).unwrap();
        assert_eq!(
            runs,
            vec![
                utc(2026, 3, 28, 1, 30),
                utc(2026, 3, 29, 1, 0),
                utc(2026, 3, 30, 0, 30)
            ]
        );
    }

    #[test]
    fn next_runs_zero_count_is_empty() {
        assert_eq!(
            next_runs("0 2 * * *", utc(2026, 1, 1, 0, 0), chrono_tz::UTC, 0).unwrap(),
            Vec::<DateTime<Utc>>::new()
        );
    }

    #[test]
    fn next_runs_rejects_an_invalid_expression() {
        assert!(next_runs("nope", utc(2026, 1, 1, 0, 0), chrono_tz::UTC, 3).is_err());
    }
}

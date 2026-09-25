// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! WebAssembly bindings for the timezone-aware parts of the `domain` crate.
//!
//! Kept apart from `domain-wasm` because the IANA timezone database makes this
//! module roughly ten times larger; the frontend only loads it when it needs a
//! next-run preview.

use chrono::{DateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use wasm_bindgen::prelude::*;

/// The next `count` runs of `expression` after `from` (RFC 3339), evaluated in
/// the IANA `timezone` exactly as the scheduler will fire them; see
/// [`domain::schedule::next_runs`]. Each run is an RFC 3339 UTC timestamp.
///
/// # Errors
///
/// Fails if `from` is not RFC 3339, `timezone` is not an IANA name, or the
/// expression is invalid.
#[wasm_bindgen(js_name = nextCronRuns)]
pub fn next_cron_runs(
    expression: &str,
    from: &str,
    timezone: &str,
    count: usize,
) -> Result<Vec<String>, JsError> {
    let from = DateTime::parse_from_rfc3339(from)
        .map_err(|e| JsError::new(&format!("invalid start time '{from}': {e}")))?
        .with_timezone(&Utc);
    let tz: Tz = timezone
        .parse()
        .map_err(|e| JsError::new(&format!("invalid timezone '{timezone}': {e}")))?;
    let runs =
        domain::schedule::next_runs(expression, from, tz, count).map_err(|e| JsError::new(&e))?;
    Ok(runs
        .iter()
        .map(|run| run.to_rfc3339_opts(SecondsFormat::Secs, true))
        .collect())
}

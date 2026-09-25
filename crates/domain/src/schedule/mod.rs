// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use croner::Cron;

/// Next-run calculation; needs the timezone database, so it is behind the
/// `timezones` feature to keep it out of the small WebAssembly module.
#[cfg(feature = "timezones")]
mod next_run;

#[cfg(feature = "timezones")]
pub use next_run::{calculate_next_run, next_runs};

/// Checks that `expression` parses in the scheduler's cron dialect.
///
/// # Errors
///
/// Returns an error if `expression` is not a valid cron expression.
pub fn validate_cron(expression: &str) -> Result<(), String> {
    Cron::new(expression)
        .parse()
        .map_err(|e| format!("invalid cron expression: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_cron;

    #[test]
    fn validate_accepts_names_the_old_frontend_check_rejected() {
        assert!(validate_cron("0 2 * * MON-FRI").is_ok());
        assert!(validate_cron("0 2 * JAN *").is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_fields() {
        assert!(validate_cron("60 2 * * *").is_err());
        assert!(validate_cron("0 24 * * *").is_err());
        assert!(validate_cron("0 2 * *").is_err());
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The bounds of a machine's "when the host is offline" settings: how often a
//! machine a run waits on is asked whether it is back, and how long the run
//! waits. Set on the machine (see [`super::machines`]), separately from its
//! wake settings: those carry their own validation (a MAC address to wake
//! with, an SSH user to shut down with), and marking a laptop as "not always
//! online" should not fail because its wake settings are incomplete.

use crate::error::ApiError;

/// Upper bound on the re-check interval: a week. Past that the probe is slower
/// than any schedule it could serve, so the catch-up would only ever be found
/// after a regular run had already covered the gap.
pub(crate) const MAX_CATCH_UP_RECHECK_MINUTES: i32 = 10_080;

/// Upper bound on the give-up window: thirty days. A backup nobody has managed
/// to take in a month is not waiting on a transient outage.
pub(crate) const MAX_CATCH_UP_GIVE_UP_MINUTES: i32 = 43_200;

pub(crate) fn validate_recheck_minutes(minutes: i32) -> Result<i32, ApiError> {
    if !(1..=MAX_CATCH_UP_RECHECK_MINUTES).contains(&minutes) {
        return Err(ApiError::BadRequest(format!(
            "catch_up_recheck_minutes must be between 1 and {MAX_CATCH_UP_RECHECK_MINUTES}"
        )));
    }
    Ok(minutes)
}

/// Zero means "wait indefinitely" and is always allowed. Any other value is
/// bounded, and - for a repository, which has to be asked - has to leave room
/// for at least one probe: a window shorter than the interval that fills it
/// would abandon every catch-up without ever having asked whether the host was
/// back, which from the outside looks like the feature silently not working.
pub(crate) fn validate_give_up_minutes(
    minutes: i32,
    recheck_minutes: Option<i32>,
) -> Result<i32, ApiError> {
    if minutes == 0 {
        return Ok(0);
    }
    if !(1..=MAX_CATCH_UP_GIVE_UP_MINUTES).contains(&minutes) {
        return Err(ApiError::BadRequest(format!(
            "catch_up_give_up_minutes must be 0 (wait indefinitely) or between 1 and \
             {MAX_CATCH_UP_GIVE_UP_MINUTES}"
        )));
    }
    if let Some(recheck) = recheck_minutes
        && minutes < recheck
    {
        return Err(ApiError::BadRequest(format!(
            "catch_up_give_up_minutes ({minutes}) must leave room for at least one re-check \
             ({recheck} minutes)"
        )));
    }
    Ok(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recheck_interval_is_bounded_on_both_sides() {
        assert!(validate_recheck_minutes(0).is_err());
        assert_eq!(validate_recheck_minutes(1).unwrap(), 1);
        assert_eq!(
            validate_recheck_minutes(MAX_CATCH_UP_RECHECK_MINUTES).unwrap(),
            MAX_CATCH_UP_RECHECK_MINUTES
        );
        assert!(validate_recheck_minutes(MAX_CATCH_UP_RECHECK_MINUTES + 1).is_err());
    }

    /// Zero is how every host behaved before the window existed, so it has to
    /// stay valid whatever the interval is.
    #[test]
    fn zero_is_always_a_valid_give_up_window() {
        assert_eq!(validate_give_up_minutes(0, Some(60)).unwrap(), 0);
        assert_eq!(validate_give_up_minutes(0, None).unwrap(), 0);
    }

    #[test]
    fn a_repository_host_window_must_hold_at_least_one_recheck() {
        assert!(validate_give_up_minutes(10, Some(15)).is_err());
        assert_eq!(validate_give_up_minutes(15, Some(15)).unwrap(), 15);
    }

    /// An agent is never asked, so there is no interval to leave room for.
    #[test]
    fn an_agent_window_only_has_to_be_in_range() {
        assert_eq!(validate_give_up_minutes(1, None).unwrap(), 1);
        assert!(validate_give_up_minutes(-1, None).is_err());
        assert!(validate_give_up_minutes(MAX_CATCH_UP_GIVE_UP_MINUTES + 1, None).is_err());
    }
}

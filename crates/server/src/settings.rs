// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Typed accessors for runtime-tunable `system_settings` values.
//!
//! Each value is read from the database on every use, so a change saved on the
//! System page takes effect on the next operation without a server restart.

use std::time::Duration;

use sqlx::PgPool;

use crate::{db, error::ApiError};

/// Upper bound on a single `borg list`/`borg info` invocation.
///
/// The database is the only source: the `ASSIMILATE_BORG_QUERY_TIMEOUT_SECS`
/// environment variable that used to back it is no longer read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorgQueryTimeout(u64);

/// Why a requested borg query timeout was rejected.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum InvalidBorgQueryTimeout {
    /// Zero would kill every borg query immediately.
    #[error("{} must be greater than zero", BorgQueryTimeout::KEY)]
    Zero,
    /// Longer than [`BorgQueryTimeout::MAX_SECS`].
    #[error(
        "{} must be at most {} seconds",
        BorgQueryTimeout::KEY,
        BorgQueryTimeout::MAX_SECS
    )]
    TooLong,
}

impl From<InvalidBorgQueryTimeout> for ApiError {
    fn from(e: InvalidBorgQueryTimeout) -> Self {
        Self::BadRequest(e.to_string())
    }
}

impl TryFrom<u64> for BorgQueryTimeout {
    type Error = InvalidBorgQueryTimeout;

    fn try_from(secs: u64) -> Result<Self, Self::Error> {
        match secs {
            0 => Err(InvalidBorgQueryTimeout::Zero),
            s if s > Self::MAX_SECS => Err(InvalidBorgQueryTimeout::TooLong),
            s => Ok(Self(s)),
        }
    }
}

impl Default for BorgQueryTimeout {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl BorgQueryTimeout {
    /// The `system_settings` key holding the timeout, in seconds.
    pub const KEY: &str = "borg_query_timeout_secs";
    /// Applied when the setting is unset or holds an unusable value.
    pub const DEFAULT: Self = Self(300);
    /// A week: long enough for any listing over a slow link, short enough that
    /// a hung connection is still noticed.
    pub const MAX_SECS: u64 = 7 * 24 * 60 * 60;

    /// The timeout in whole seconds.
    #[must_use]
    pub const fn as_secs(self) -> u64 {
        self.0
    }

    /// The timeout as a [`Duration`].
    #[must_use]
    pub const fn duration(self) -> Duration {
        Duration::from_secs(self.0)
    }

    /// Interprets a stored value. Zero or an unparsable value falls back to
    /// [`Self::DEFAULT`]; a value above [`Self::MAX_SECS`] (which the API
    /// never writes, but an older release could have) is capped rather than
    /// discarded, so it keeps meaning "a long time".
    fn from_stored(secs: Option<u64>) -> Self {
        secs.filter(|&s| s > 0)
            .map_or(Self::DEFAULT, |s| Self(s.min(Self::MAX_SECS)))
    }

    /// Reads the configured timeout.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn load(pool: &PgPool) -> Result<Self, ApiError> {
        Ok(Self::from_stored(
            db::get_parsed_setting::<u64>(pool, Self::KEY).await?,
        ))
    }

    /// Persists this timeout.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn store(self, pool: &PgPool) -> Result<(), ApiError> {
        db::set_setting(pool, Self::KEY, &self.0.to_string()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero() {
        assert_eq!(
            BorgQueryTimeout::try_from(0),
            Err(InvalidBorgQueryTimeout::Zero)
        );
    }

    #[test]
    fn rejects_more_than_the_maximum() {
        assert_eq!(
            BorgQueryTimeout::try_from(BorgQueryTimeout::MAX_SECS + 1),
            Err(InvalidBorgQueryTimeout::TooLong)
        );
        assert_eq!(
            BorgQueryTimeout::try_from(u64::MAX),
            Err(InvalidBorgQueryTimeout::TooLong)
        );
    }

    #[test]
    fn accepts_the_whole_valid_range() {
        assert_eq!(BorgQueryTimeout::try_from(1).unwrap().as_secs(), 1);
        assert_eq!(
            BorgQueryTimeout::try_from(BorgQueryTimeout::MAX_SECS)
                .unwrap()
                .duration(),
            Duration::from_secs(BorgQueryTimeout::MAX_SECS)
        );
    }

    #[test]
    fn errors_name_the_field_for_the_400_body() {
        let message = ApiError::from(InvalidBorgQueryTimeout::TooLong).to_string();
        assert!(message.contains("borg_query_timeout_secs"), "{message}");
        assert!(message.contains("604800"), "{message}");
    }

    #[test]
    fn unset_or_zero_stored_values_use_the_default() {
        assert_eq!(
            BorgQueryTimeout::from_stored(None),
            BorgQueryTimeout::DEFAULT
        );
        assert_eq!(
            BorgQueryTimeout::from_stored(Some(0)),
            BorgQueryTimeout::DEFAULT
        );
        assert_eq!(BorgQueryTimeout::default().as_secs(), 300);
    }

    #[test]
    fn oversized_stored_values_are_capped() {
        assert_eq!(
            BorgQueryTimeout::from_stored(Some(u64::MAX)).as_secs(),
            BorgQueryTimeout::MAX_SECS
        );
        assert_eq!(BorgQueryTimeout::from_stored(Some(42)).as_secs(), 42);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn load_defaults_when_unset_and_round_trips_a_stored_value(pool: PgPool) {
        assert_eq!(
            BorgQueryTimeout::load(&pool).await.unwrap(),
            BorgQueryTimeout::DEFAULT
        );

        BorgQueryTimeout::try_from(42)
            .unwrap()
            .store(&pool)
            .await
            .unwrap();
        assert_eq!(BorgQueryTimeout::load(&pool).await.unwrap().as_secs(), 42);
        assert_eq!(
            db::get_setting(&pool, BorgQueryTimeout::KEY).await.unwrap(),
            Some("42".to_owned())
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn load_ignores_an_unparsable_stored_value(pool: PgPool) {
        db::set_setting(&pool, BorgQueryTimeout::KEY, "soon")
            .await
            .unwrap();
        assert_eq!(
            BorgQueryTimeout::load(&pool).await.unwrap(),
            BorgQueryTimeout::DEFAULT
        );
    }
}

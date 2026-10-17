// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The VAPID key pair web push notifications are signed with.
//!
//! The database is the only place the pair is read from at runtime. The
//! `VAPID_PUBLIC_KEY`/`VAPID_PRIVATE_KEY` environment variables are a one-time
//! bootstrap seed: at startup, if the database holds no pair yet, a complete
//! pair from the environment is copied into it; otherwise a fresh pair is
//! generated. Once the database holds a pair, the environment is ignored, and
//! the pair is replaced only through `PUT /api/notifications/push/vapid-key`.

use std::fmt;

use super::{NotificationError, NotificationService};

/// The `system_settings` key holding the URL-safe base64 private key.
pub const PRIVATE_KEY_SETTING: &str = "vapid_private_key";
/// The `system_settings` key holding the URL-safe base64 public key.
pub const PUBLIC_KEY_SETTING: &str = "vapid_public_key";

const PUBLIC_KEY_ENV: &str = "VAPID_PUBLIC_KEY";
const PRIVATE_KEY_ENV: &str = "VAPID_PRIVATE_KEY";

/// What the environment offers to seed an empty database with.
#[derive(PartialEq, Eq)]
pub enum VapidEnvSeed {
    /// Neither variable is set.
    Absent,
    /// Only one of the two variables is set; a lone key cannot sign anything
    /// a browser would accept, so it is ignored.
    Incomplete,
    /// Both keys, trimmed.
    Pair {
        /// URL-safe base64 public key.
        public_key: String,
        /// URL-safe base64 private key.
        private_key: String,
    },
}

impl fmt::Debug for VapidEnvSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => f.write_str("Absent"),
            Self::Incomplete => f.write_str("Incomplete"),
            Self::Pair { .. } => f.write_str("Pair { [REDACTED] }"),
        }
    }
}

impl From<(Option<String>, Option<String>)> for VapidEnvSeed {
    /// Builds the seed from `(public, private)`; a blank value counts as unset.
    fn from((public_key, private_key): (Option<String>, Option<String>)) -> Self {
        let present = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        match (present(public_key), present(private_key)) {
            (Some(public_key), Some(private_key)) => Self::Pair {
                public_key,
                private_key,
            },
            (None, None) => Self::Absent,
            (Some(_), None) | (None, Some(_)) => Self::Incomplete,
        }
    }
}

impl VapidEnvSeed {
    /// Reads `VAPID_PUBLIC_KEY` and `VAPID_PRIVATE_KEY` from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from((
            std::env::var(PUBLIC_KEY_ENV).ok(),
            std::env::var(PRIVATE_KEY_ENV).ok(),
        ))
    }
}

/// A freshly generated pair, URL-safe base64 encoded as browsers expect.
fn generate_pair() -> (String, String) {
    use base64::Engine;
    use p256::ecdsa::SigningKey;

    let signing_key = SigningKey::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let public_bytes = signing_key.verifying_key().to_encoded_point(false);

    let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    (
        encoder.encode(public_bytes.as_bytes()),
        encoder.encode(signing_key.to_bytes()),
    )
}

impl NotificationService {
    /// Makes sure the database holds a VAPID key pair, seeding it from `seed`
    /// when it holds none yet and generating one when `seed` has no complete pair.
    ///
    /// # Errors
    ///
    /// Returns [`NotificationError::Config`] if the database cannot be read or written.
    pub async fn ensure_vapid_keys(&self, seed: VapidEnvSeed) -> Result<(), NotificationError> {
        let existing = crate::db::get_setting(&self.pool, PRIVATE_KEY_SETTING)
            .await
            .map_err(|e| NotificationError::Config(format!("DB error: {e}")))?;

        if existing.is_some() {
            if matches!(seed, VapidEnvSeed::Pair { .. } | VapidEnvSeed::Incomplete) {
                tracing::info!(
                    "ignoring {PUBLIC_KEY_ENV}/{PRIVATE_KEY_ENV}: the database already holds a \
                     VAPID key pair"
                );
            }
            return Ok(());
        }

        let (public_key, private_key) = match seed {
            VapidEnvSeed::Pair {
                public_key,
                private_key,
            } => {
                tracing::info!(
                    "seeding the VAPID key pair from {PUBLIC_KEY_ENV}/{PRIVATE_KEY_ENV}"
                );
                (public_key, private_key)
            }
            VapidEnvSeed::Incomplete => {
                tracing::warn!(
                    "only one of {PUBLIC_KEY_ENV}/{PRIVATE_KEY_ENV} is set; generating a new \
                     VAPID key pair instead"
                );
                generate_pair()
            }
            VapidEnvSeed::Absent => {
                tracing::info!("generating VAPID key pair for web push notifications");
                generate_pair()
            }
        };

        // The private key goes in last: it is what marks the pair as present,
        // so an interrupted write is retried on the next start.
        crate::db::set_setting(&self.pool, PUBLIC_KEY_SETTING, &public_key)
            .await
            .map_err(|e| NotificationError::Config(format!("DB error saving public key: {e}")))?;
        crate::db::set_setting(&self.pool, PRIVATE_KEY_SETTING, &private_key)
            .await
            .map_err(|e| NotificationError::Config(format!("DB error saving private key: {e}")))?;

        tracing::info!("VAPID keys stored in database");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::db;

    fn pair(public_key: &str, private_key: &str) -> VapidEnvSeed {
        VapidEnvSeed::Pair {
            public_key: public_key.to_owned(),
            private_key: private_key.to_owned(),
        }
    }

    async fn stored(pool: &PgPool) -> (Option<String>, Option<String>) {
        (
            db::get_setting(pool, PUBLIC_KEY_SETTING).await.unwrap(),
            db::get_setting(pool, PRIVATE_KEY_SETTING).await.unwrap(),
        )
    }

    #[test]
    fn seed_needs_both_keys() {
        assert_eq!(VapidEnvSeed::from((None, None)), VapidEnvSeed::Absent);
        assert_eq!(
            VapidEnvSeed::from((Some("pub".to_owned()), None)),
            VapidEnvSeed::Incomplete
        );
        assert_eq!(
            VapidEnvSeed::from((None, Some("priv".to_owned()))),
            VapidEnvSeed::Incomplete
        );
        assert_eq!(
            VapidEnvSeed::from((Some(" pub ".to_owned()), Some("priv\n".to_owned()))),
            pair("pub", "priv")
        );
    }

    #[test]
    fn blank_values_count_as_unset() {
        assert_eq!(
            VapidEnvSeed::from((Some("  ".to_owned()), Some(String::new()))),
            VapidEnvSeed::Absent
        );
        assert_eq!(
            VapidEnvSeed::from((Some("pub".to_owned()), Some(" ".to_owned()))),
            VapidEnvSeed::Incomplete
        );
    }

    #[test]
    fn debug_never_shows_a_key() {
        let shown = format!("{:?}", pair("public-material", "private-material"));
        assert!(!shown.contains("material"), "{shown}");
        assert!(shown.contains("[REDACTED]"), "{shown}");
    }

    #[test]
    fn generated_pairs_are_fresh_and_well_formed() {
        use base64::Engine;

        let (public_key, private_key) = generate_pair();
        let decode = |v: &str| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(v)
                .unwrap()
        };
        assert_eq!(decode(&public_key).len(), 65);
        assert_eq!(decode(&private_key).len(), 32);
        assert_ne!(generate_pair().1, private_key);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn an_empty_database_is_seeded_from_a_complete_env_pair(pool: PgPool) {
        let service = crate::test_support::test_notification_service(pool.clone());
        service
            .ensure_vapid_keys(pair("env-public", "env-private"))
            .await
            .unwrap();
        assert_eq!(
            stored(&pool).await,
            (
                Some("env-public".to_owned()),
                Some("env-private".to_owned())
            )
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_stored_pair_wins_over_the_environment(pool: PgPool) {
        db::set_setting(&pool, PUBLIC_KEY_SETTING, "db-public")
            .await
            .unwrap();
        db::set_setting(&pool, PRIVATE_KEY_SETTING, "db-private")
            .await
            .unwrap();
        let service = crate::test_support::test_notification_service(pool.clone());
        service
            .ensure_vapid_keys(pair("env-public", "env-private"))
            .await
            .unwrap();
        assert_eq!(
            stored(&pool).await,
            (Some("db-public".to_owned()), Some("db-private".to_owned()))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn without_a_complete_env_pair_a_new_pair_is_generated(pool: PgPool) {
        let service = crate::test_support::test_notification_service(pool.clone());
        service
            .ensure_vapid_keys(VapidEnvSeed::Incomplete)
            .await
            .unwrap();
        let (Some(public_key), Some(private_key)) = stored(&pool).await else {
            panic!("a generated pair must be stored");
        };
        assert_eq!(public_key.len(), 87);
        assert_eq!(private_key.len(), 43);

        // A second start keeps the pair it generated the first time.
        service
            .ensure_vapid_keys(VapidEnvSeed::Absent)
            .await
            .unwrap();
        assert_eq!(stored(&pool).await, (Some(public_key), Some(private_key)));
    }
}

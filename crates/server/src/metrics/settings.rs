// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Whether `/metrics` is served, and which networks may scrape it without a
//! token. Both live in `system_settings` and are off and empty by default.

use std::net::IpAddr;

use ipnetwork::IpNetwork;
use sqlx::PgPool;

use crate::{db, error::ApiError};

const ENABLED_KEY: &str = "metrics_enabled";
const ALLOWED_NETWORKS_KEY: &str = "metrics_allowed_networks";

/// How many networks the allow-list may hold. A list longer than this is a
/// mistake, not a network plan, and every entry is checked on every scrape.
pub const MAX_ALLOWED_NETWORKS: usize = 64;

/// The `/metrics` settings as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsSettings {
    /// Whether `/metrics` is served at all. Off by default.
    pub enabled: bool,
    /// Networks whose clients may scrape without a monitoring token.
    pub allowed_networks: Vec<IpNetwork>,
}

/// Why an allow-list entry was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetworkListError {
    /// An entry is neither an IP address nor a CIDR range.
    #[error("invalid network {entry:?}: expected an IP address or CIDR range such as 10.0.0.0/8")]
    Invalid {
        /// The entry as given.
        entry: String,
    },
    /// The list has more entries than [`MAX_ALLOWED_NETWORKS`].
    #[error("at most {max} networks may be listed", max = MAX_ALLOWED_NETWORKS)]
    TooMany,
}

impl From<NetworkListError> for ApiError {
    fn from(error: NetworkListError) -> Self {
        Self::BadRequest(error.to_string())
    }
}

/// Parses an allow-list given as entries separated by commas, spaces or line
/// breaks. A bare address is a single-host network. Duplicates are dropped and
/// the order kept, so what is stored is what the admin wrote, tidied.
///
/// # Errors
///
/// Returns [`NetworkListError`] for an entry that does not parse or a list
/// that is too long.
pub fn parse_networks<'a>(
    entries: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<IpNetwork>, NetworkListError> {
    let networks = entries
        .into_iter()
        .flat_map(|entry| entry.split([',', ' ', '\n', '\r', '\t']))
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .parse::<IpNetwork>()
                .map_err(|_| NetworkListError::Invalid {
                    entry: entry.chars().take(64).collect(),
                })
        })
        .try_fold(Vec::new(), |mut networks, network| {
            let network = network?;
            if !networks.contains(&network) {
                networks.push(network);
            }
            Ok::<_, NetworkListError>(networks)
        })?;
    if networks.len() > MAX_ALLOWED_NETWORKS {
        return Err(NetworkListError::TooMany);
    }
    Ok(networks)
}

impl MetricsSettings {
    /// Reads the settings, treating a missing or unreadable value as the
    /// default: off, and no network allowed in without a token.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn load(pool: &PgPool) -> Result<Self, ApiError> {
        let enabled = db::get_parsed_setting::<bool>(pool, ENABLED_KEY)
            .await?
            .unwrap_or(false);
        let allowed_networks = db::get_setting(pool, ALLOWED_NETWORKS_KEY)
            .await?
            .map_or_else(Vec::new, |stored| {
                parse_networks([stored.as_str()])
                    .inspect_err(|e| {
                        tracing::warn!(error = %e, "ignoring unreadable metrics allow-list");
                    })
                    .unwrap_or_default()
            });
        Ok(Self {
            enabled,
            allowed_networks,
        })
    }

    /// Writes both settings.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn save(&self, pool: &PgPool) -> Result<(), ApiError> {
        db::set_setting(pool, ENABLED_KEY, &self.enabled.to_string()).await?;
        let networks = self
            .allowed_networks
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        db::set_setting(pool, ALLOWED_NETWORKS_KEY, &networks).await
    }

    /// Whether a client at `ip` may scrape without a token.
    #[must_use]
    pub fn allows_without_token(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        self.allowed_networks
            .iter()
            .any(|network| network.contains(ip))
    }
}

/// An IPv4 client reaching a dual-stack listener shows up as `::ffff:a.b.c.d`;
/// it is still that IPv4 address as far as an IPv4 allow-list goes.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn settings(networks: &str) -> MetricsSettings {
        MetricsSettings {
            enabled: true,
            allowed_networks: parse_networks([networks]).unwrap(),
        }
    }

    #[test]
    fn parses_cidrs_and_bare_addresses_separated_any_way() {
        let networks = parse_networks(["10.0.0.0/8, 192.168.1.5\n2001:db8::/32  fd00::1"]).unwrap();

        assert_eq!(
            networks.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [
                "10.0.0.0/8",
                "192.168.1.5/32",
                "2001:db8::/32",
                "fd00::1/128"
            ]
        );
    }

    #[test]
    fn drops_duplicates_and_keeps_order() {
        let networks = parse_networks(["10.0.0.0/8,127.0.0.1,10.0.0.0/8"]).unwrap();

        assert_eq!(networks.len(), 2);
        assert_eq!(
            networks.first().map(ToString::to_string).as_deref(),
            Some("10.0.0.0/8")
        );
    }

    #[test]
    fn empty_input_allows_nothing() {
        assert_eq!(parse_networks(["", " , \n"]), Ok(Vec::new()));
    }

    #[test]
    fn rejects_an_entry_that_is_not_a_network() {
        assert_eq!(
            parse_networks(["10.0.0.0/8,not-a-network"]),
            Err(NetworkListError::Invalid {
                entry: "not-a-network".to_owned()
            })
        );
        assert!(parse_networks(["10.0.0.0/33"]).is_err());
    }

    #[test]
    fn truncates_a_long_invalid_entry_in_the_error() {
        let long = "x".repeat(500);

        assert_eq!(
            parse_networks([long.as_str()]),
            Err(NetworkListError::Invalid {
                entry: "x".repeat(64)
            })
        );
    }

    #[test]
    fn rejects_too_many_networks() {
        let list = |count: usize| {
            (0..count)
                .map(|i| format!("10.{i}.0.0/16"))
                .collect::<Vec<_>>()
                .join(",")
        };

        assert!(
            parse_networks([list(MAX_ALLOWED_NETWORKS).as_str()])
                .is_ok_and(|n| n.len() == MAX_ALLOWED_NETWORKS)
        );
        assert_eq!(
            parse_networks([list(MAX_ALLOWED_NETWORKS.checked_add(1).unwrap()).as_str()]),
            Err(NetworkListError::TooMany)
        );
    }

    #[test]
    fn matches_addresses_inside_an_allowed_range_only() {
        let settings = settings("10.0.0.0/8,192.168.1.5");

        assert!(settings.allows_without_token(IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40))));
        assert!(settings.allows_without_token(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))));
        assert!(!settings.allows_without_token(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 6))));
        assert!(!settings.allows_without_token(IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))));
    }

    #[test]
    fn matches_ipv6_ranges() {
        let settings = settings("2001:db8::/32");

        assert!(settings.allows_without_token("2001:db8::42".parse().unwrap()));
        assert!(!settings.allows_without_token("2001:db9::42".parse().unwrap()));
        assert!(!settings.allows_without_token(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn treats_an_ipv4_mapped_ipv6_client_as_its_ipv4_address() {
        let settings = settings("127.0.0.0/8");

        assert!(settings.allows_without_token("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn allows_nobody_without_a_token_by_default() {
        let settings = MetricsSettings::default();

        assert!(!settings.enabled);
        assert!(!settings.allows_without_token(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unreadable_stored_allow_list_is_ignored_with_a_warning(pool: PgPool) {
        settings("10.0.0.0/8").save(&pool).await.unwrap();
        db::set_setting(&pool, ALLOWED_NETWORKS_KEY, "10.0.0.0/8,not-a-network")
            .await
            .unwrap();

        let mut loaded = None;
        let warnings = crate::test_support::warnings_while(async {
            loaded = Some(MetricsSettings::load(&pool).await.unwrap());
        })
        .await;

        assert_eq!(
            loaded,
            Some(MetricsSettings {
                enabled: true,
                allowed_networks: Vec::new(),
            })
        );
        assert!(crate::test_support::logged(
            &warnings,
            "ignoring unreadable metrics allow-list"
        ));
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Provider presets for repository hosts.
//!
//! A repository host is entered by hand today: hostname, port, user, path and
//! the borg binary the remote side runs. A [`HostProvider`] names the hosting
//! service so the right defaults can be filled in and locked, instead of the
//! user having to know that a Hetzner Storage Box listens on port 23 or that
//! rsync.net runs borg 1.x as `borg1`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The hosting service a repository host belongs to.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::AsRefStr,
    strum_macros::EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum HostProvider {
    /// Any SSH server running borg: every setting is entered by hand.
    #[default]
    Generic,
    /// A Hetzner Storage Box.
    Hetzner,
    /// An rsync.net account.
    RsyncNet,
    /// A `BorgBase` repository.
    Borgbase,
}

/// The borg binary a remote side runs, passed to borg as `--remote-path`
/// (exported as `BORG_REMOTE_PATH`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::AsRefStr,
    strum_macros::EnumIter,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum RemoteBorg {
    /// Borg 1.4 on rsync.net.
    #[strum(serialize = "borg14")]
    #[serde(rename = "borg14")]
    RsyncNetBorg14,
    /// Borg 1.4 on a Hetzner Storage Box.
    #[strum(serialize = "borg-1.4")]
    #[serde(rename = "borg-1.4")]
    HetznerBorg14,
}

impl RemoteBorg {
    /// The executable name borg is told to run on the remote side.
    #[must_use]
    pub const fn executable(self) -> &'static str {
        match self {
            Self::RsyncNetBorg14 => "borg14",
            Self::HetznerBorg14 => "borg-1.4",
        }
    }
}

/// How the server installs its SSH key on a host.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
    strum_macros::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum KeyDeployMethod {
    /// Append the key to `authorized_keys` over an SSH shell.
    Shell,
    /// Upload the key over SFTP, for hosts that offer no shell.
    Sftp,
}

/// The defaults a provider fills in, and which of them the form locks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ProviderDefaults {
    /// The SSH port, or `None` when the user chooses it.
    pub ssh_port: Option<u16>,
    /// The suffix a bare account name is completed with to form the
    /// hostname, or `None` when the hostname is typed in full.
    pub hostname_suffix: Option<&'static str>,
    /// The directory repositories live in, or `None` when the user chooses
    /// it.
    pub base_path: Option<&'static str>,
    /// The borg binary on the remote side, or `None` to run plain `borg`.
    pub remote_borg: Option<RemoteBorg>,
    /// How the server's key is installed.
    pub key_deploy: KeyDeployMethod,
}

/// A provider with the defaults it fills in: one row of the preset picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ProviderPreset {
    /// The provider the defaults belong to.
    pub provider: HostProvider,
    /// What it fills in, and which of it the form locks.
    pub defaults: ProviderDefaults,
}

impl HostProvider {
    /// Every provider with its defaults, in the order the picker lists them.
    #[must_use]
    pub fn presets() -> Vec<ProviderPreset> {
        <Self as strum::IntoEnumIterator>::iter()
            .map(|provider| ProviderPreset {
                provider,
                defaults: provider.defaults(),
            })
            .collect()
    }

    /// The defaults this provider fills in.
    ///
    /// Hetzner Storage Boxes serve SSH on port 23, give no shell (so the key
    /// goes over SFTP) and keep repositories under `./backups`. rsync.net
    /// serves the default port and selects the borg version by binary name.
    /// `BorgBase` gives every repository its own hostname and path, so only the
    /// port is fixed.
    #[must_use]
    pub const fn defaults(self) -> ProviderDefaults {
        match self {
            Self::Generic => ProviderDefaults {
                ssh_port: None,
                hostname_suffix: None,
                base_path: None,
                remote_borg: None,
                key_deploy: KeyDeployMethod::Sftp,
            },
            Self::Hetzner => ProviderDefaults {
                ssh_port: Some(23),
                hostname_suffix: Some(".your-storagebox.de"),
                base_path: Some("./backups"),
                remote_borg: Some(RemoteBorg::HetznerBorg14),
                key_deploy: KeyDeployMethod::Sftp,
            },
            Self::RsyncNet => ProviderDefaults {
                ssh_port: Some(22),
                hostname_suffix: None,
                base_path: None,
                remote_borg: Some(RemoteBorg::RsyncNetBorg14),
                key_deploy: KeyDeployMethod::Sftp,
            },
            Self::Borgbase => ProviderDefaults {
                ssh_port: Some(22),
                hostname_suffix: None,
                base_path: None,
                remote_borg: None,
                key_deploy: KeyDeployMethod::Shell,
            },
        }
    }

    /// Checks that `ssh_port` agrees with a port the provider fixes.
    ///
    /// # Errors
    ///
    /// Returns a message naming the required port when it differs.
    pub fn check_port(self, ssh_port: u16) -> Result<(), String> {
        match self.defaults().ssh_port {
            Some(required) if required != ssh_port => Err(format!(
                "{self} hosts use SSH port {required}, not {ssh_port}"
            )),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn generic_fixes_nothing() {
        let d = HostProvider::Generic.defaults();
        assert_eq!(d.ssh_port, None);
        assert_eq!(d.base_path, None);
        assert_eq!(d.remote_borg, None);
        assert_eq!(d.key_deploy, KeyDeployMethod::Sftp);
    }

    #[test]
    fn presets_list_every_provider_once_with_generic_first() {
        let presets = HostProvider::presets();
        assert_eq!(presets.len(), HostProvider::iter().count());
        assert_eq!(
            presets.first().map(|p| p.provider),
            Some(HostProvider::Generic)
        );
        assert!(presets.iter().all(|p| p.defaults == p.provider.defaults()));
    }

    #[test]
    fn default_provider_is_generic() {
        assert_eq!(HostProvider::default(), HostProvider::Generic);
    }

    #[test]
    fn hetzner_uses_port_23_sftp_and_backups_dir() {
        let d = HostProvider::Hetzner.defaults();
        assert_eq!(d.ssh_port, Some(23));
        assert_eq!(d.base_path, Some("./backups"));
        assert_eq!(d.key_deploy, KeyDeployMethod::Sftp);
        assert_eq!(d.remote_borg, Some(RemoteBorg::HetznerBorg14));
    }

    #[test]
    fn rsync_net_selects_borg_by_remote_path_and_deploys_key() {
        let d = HostProvider::RsyncNet.defaults();
        assert_eq!(d.remote_borg, Some(RemoteBorg::RsyncNetBorg14));
        assert_eq!(d.key_deploy, KeyDeployMethod::Sftp);
        assert_eq!(d.ssh_port, Some(22));
    }

    #[test]
    fn borgbase_runs_plain_borg_over_port_22() {
        let d = HostProvider::Borgbase.defaults();
        assert_eq!(d.remote_borg, None);
        assert_eq!(d.ssh_port, Some(22));
    }

    #[test]
    fn remote_borg_executables() {
        assert_eq!(RemoteBorg::RsyncNetBorg14.executable(), "borg14");
        assert_eq!(RemoteBorg::HetznerBorg14.executable(), "borg-1.4");
    }

    #[test]
    fn check_port_enforces_fixed_ports_only() {
        assert!(HostProvider::Hetzner.check_port(23).is_ok());
        assert!(HostProvider::Hetzner.check_port(22).is_err());
        assert!(HostProvider::Generic.check_port(2222).is_ok());
    }

    #[test]
    fn every_provider_round_trips_through_its_string_form() {
        HostProvider::iter().for_each(|p| {
            assert_eq!(HostProvider::from_str(p.as_ref()), Ok(p));
        });
        assert_eq!(HostProvider::RsyncNet.as_ref(), "rsync_net");
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert!(HostProvider::from_str("dropbox").is_err());
    }
}

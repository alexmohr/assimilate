// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Types describing the libvirt/QEMU domains an agent stages before a backup,
//! the per-host settings that govern the staging, and what one run did to each
//! domain.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Directory a host stages its domains into when the operator leaves the
/// staging directory empty.
pub const DEFAULT_STAGING_DIR: &str = "/home/virt/backups";

/// Number of increments written before a fresh full image is taken.
pub const DEFAULT_FULL_INTERVAL: u32 = 7;

/// Seconds one domain's snapshot may take before it is aborted.
pub const DEFAULT_SNAPSHOT_TIMEOUT_SECONDS: u32 = 1800;

/// The run state of a domain as libvirt reports it.
///
/// Parsing accepts libvirt's own wording alongside the `snake_case` names
/// used on the wire, so an agent can turn a `virsh domstate` line into this enum
/// without comparing strings itself: libvirt writes "shut off" with a space
/// and calls a guest-initiated suspend "pmsuspended".
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum VmState {
    /// The domain is running.
    Running,
    /// The domain is paused, but its memory is still resident.
    Paused,
    /// The domain is defined but not running.
    #[strum(to_string = "shut_off", serialize = "shut off", serialize = "shutoff")]
    ShutOff,
    /// The domain is suspended to RAM or disk by the guest.
    #[strum(to_string = "suspended", serialize = "pmsuspended")]
    Suspended,
    /// libvirt reported a state this build does not know, or the host has not
    /// been scanned yet.
    #[default]
    Unknown,
}

impl VmState {
    /// Whether a domain in this state has a live QEMU process, which decides
    /// between the snapshot paths and a plain copy of the disks.
    #[must_use]
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Suspended)
    }
}

/// How a domain is captured. Which one applies is decided by the agent from
/// the domain's state and disk formats, not configured by the operator.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum VmSnapshotMode {
    /// Live domain on qcow2 disks: libvirt checkpoints let every run after the
    /// first write only the clusters that changed.
    Incremental,
    /// Live domain that cannot do incremental backups (raw disks, or a libvirt
    /// without the backup API): a full copy taken from a temporary external
    /// snapshot.
    FullCopy,
    /// Shut off domain: the disks are copied directly, and skipped entirely
    /// while they are unchanged.
    OfflineCopy,
    /// The operator excluded this domain.
    Excluded,
    /// Not scanned yet.
    #[default]
    Unknown,
}

/// What a snapshot run actually did to one domain.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum VmRunAction {
    /// A new full image was written and the previous chain dropped.
    FullImage,
    /// Only the clusters changed since the last checkpoint were written.
    Increment,
    /// The whole disk was copied.
    Copy,
    /// Nothing was written because the disks did not change.
    Unchanged,
    /// The domain was left out: excluded, or its disks do not fit its limit.
    Skipped,
}

/// A domain as the agent last saw it on the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct DiscoveredVm {
    /// libvirt domain name, unique on its host.
    pub name: String,
    /// Run state at scan time.
    pub state: VmState,
    /// How this domain would be captured with its current settings.
    pub mode: VmSnapshotMode,
    /// Number of writable disks that would be staged.
    pub disk_count: u32,
    /// Space the domain's disks occupy on the host, as allocated rather than
    /// as apparent size. This is what a full image needs.
    pub disk_bytes: u64,
}

/// What one snapshot run did to one domain, reported after a backup so the
/// per-domain figures in the UI come from a real run rather than an estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct VmSnapshotOutcome {
    /// libvirt domain name.
    pub name: String,
    /// What the run did.
    pub action: VmRunAction,
    /// How the domain was captured.
    pub mode: VmSnapshotMode,
    /// Space the domain now occupies below the staging directory.
    pub staged_bytes: u64,
    /// Increments in the chain after this run. Zero for a copy or a full image.
    pub chain_length: u32,
    /// Why the domain failed, when it did.
    #[serde(default)]
    pub error: Option<String>,
}

/// What to do once a restored domain's images are in place.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum VmBuildAction {
    /// Merge and place the images, define nothing.
    FilesOnly,
    /// Define the domain and leave it shut off.
    #[default]
    Define,
    /// Define the domain and start it.
    DefineAndStart,
}

impl VmBuildAction {
    /// Whether the domain is defined in libvirt.
    #[must_use]
    pub const fn defines(self) -> bool {
        matches!(self, Self::Define | Self::DefineAndStart)
    }

    /// Whether the domain is started once defined.
    #[must_use]
    pub const fn starts(self) -> bool {
        matches!(self, Self::DefineAndStart)
    }
}

/// Building a domain out of files a restore put back on disk: the second stage
/// of a virtual-machine restore, which can also run on its own against any
/// directory holding a staged domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct VmBuildRequest {
    /// Directory holding the restored domain: its chain, its definition and,
    /// for a UEFI domain, its NVRAM.
    pub source_dir: String,
    /// Name to define the domain under. Restoring beside a domain that still
    /// exists needs a new one.
    pub name: String,
    /// Directory the merged images are moved to.
    pub image_dir: String,
    /// What to do once the images are in place.
    pub action: VmBuildAction,
}

/// What building a domain produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct VmBuildOutcome {
    /// The name the domain was built under.
    pub name: String,
    /// The images that now back it, in the order its disks are attached.
    pub images: Vec<String>,
    /// How many increments were merged into the full images.
    pub merged_increments: u32,
    /// Whether the domain was defined in libvirt.
    pub defined: bool,
    /// Whether the domain was started.
    pub started: bool,
}

/// How a host decides which of its domains get staged.
///
/// The two modes read the same per-domain flag from opposite directions, which
/// is what makes the choice worth storing rather than inferring: under
/// [`Self::All`] the flag marks the exceptions to stage nothing of, under
/// [`Self::Selected`] it marks the only domains to stage. A domain the
/// operator has never touched carries no flag at all, and the mode is what
/// decides where that domain lands.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    TS,
    ToSchema,
    strum_macros::Display,
    strum_macros::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum VmSelectionMode {
    /// Stage every domain on the host except the ones the operator turned
    /// off. A machine created after the last scan is backed up rather than
    /// silently missed, which is why this is the default.
    #[default]
    All,
    /// Stage only the domains the operator turned on. A machine created after
    /// the last scan is left alone until someone selects it.
    Selected,
}

impl VmSelectionMode {
    /// Whether a domain with no settings of its own is staged under this mode.
    ///
    /// This is also the flag value that carries no operator intent, so it is
    /// what tells a rescan which rows for vanished domains are safe to drop.
    #[must_use]
    pub const fn includes_untouched(self) -> bool {
        match self {
            Self::All => true,
            Self::Selected => false,
        }
    }
}

/// Per-domain settings the operator made, delivered to the agent alongside the
/// host's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmDomainConfig {
    /// libvirt domain name.
    pub name: String,
    /// Whether this domain is staged at all, or `None` when nobody has
    /// decided and the host's [`VmSnapshotConfig::selection`] answers instead.
    #[serde(default)]
    pub included: Option<bool>,
    /// Bytes this domain may occupy below the staging directory. `None`
    /// inherits the host's default.
    #[serde(default)]
    pub limit_bytes: Option<u64>,
}

/// A host's virtual-machine staging settings, delivered to the agent as part
/// of its configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmSnapshotConfig {
    /// Whether this host stages its domains at all. A schedule that opts in
    /// while this is off stages nothing.
    pub enabled: bool,
    /// Absolute directory that receives one subdirectory per domain.
    pub staging_dir: String,
    /// Write a new full image after this many increments.
    pub full_interval: u32,
    /// Seconds one domain's snapshot may take.
    pub timeout_seconds: u32,
    /// Bytes a domain may occupy below the staging directory unless it carries
    /// its own limit. Zero means no limit.
    pub default_limit_bytes: u64,
    /// Which domains the per-domain flags select.
    #[serde(default)]
    pub selection: VmSelectionMode,
    /// Per-domain settings, for the domains the operator has touched.
    #[serde(default)]
    pub domains: Vec<VmDomainConfig>,
}

impl Default for VmSnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            staging_dir: DEFAULT_STAGING_DIR.to_owned(),
            full_interval: DEFAULT_FULL_INTERVAL,
            timeout_seconds: DEFAULT_SNAPSHOT_TIMEOUT_SECONDS,
            default_limit_bytes: 0,
            selection: VmSelectionMode::default(),
            domains: Vec::new(),
        }
    }
}

impl VmSnapshotConfig {
    /// The limit that applies to one domain, in bytes, with zero meaning no
    /// limit. A domain without its own entry inherits the host's default.
    #[must_use]
    pub fn limit_for(&self, domain: &str) -> u64 {
        self.domains
            .iter()
            .find(|candidate| candidate.name == domain)
            .and_then(|candidate| candidate.limit_bytes)
            .unwrap_or(self.default_limit_bytes)
    }

    /// Whether a domain is staged, under whichever direction
    /// [`VmSnapshotConfig::selection`] reads the per-domain flags in. A domain
    /// the operator has never touched follows the mode's own default.
    #[must_use]
    pub fn includes(&self, domain: &str) -> bool {
        self.domains
            .iter()
            .find(|candidate| candidate.name == domain)
            .and_then(|candidate| candidate.included)
            .unwrap_or_else(|| self.selection.includes_untouched())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn config() -> VmSnapshotConfig {
        VmSnapshotConfig {
            enabled: true,
            default_limit_bytes: 200,
            domains: vec![
                VmDomainConfig {
                    name: "web01".to_owned(),
                    included: Some(true),
                    limit_bytes: Some(500),
                },
                VmDomainConfig {
                    name: "win-ci".to_owned(),
                    included: Some(false),
                    limit_bytes: None,
                },
                VmDomainConfig {
                    name: "db01".to_owned(),
                    included: None,
                    limit_bytes: Some(900),
                },
            ],
            ..VmSnapshotConfig::default()
        }
    }

    #[test]
    fn limit_falls_back_to_the_host_default() {
        assert_eq!(config().limit_for("web01"), 500);
        assert_eq!(config().limit_for("win-ci"), 200);
        assert_eq!(config().limit_for("never-seen"), 200);
        assert_eq!(
            config().limit_for("db01"),
            900,
            "a limit applies whether or not anyone decided about staging"
        );
    }

    #[test]
    fn a_limit_alone_is_not_a_decision_about_staging() {
        let all = config();
        let selected = VmSnapshotConfig {
            selection: VmSelectionMode::Selected,
            ..config()
        };

        assert!(
            all.includes("db01"),
            "under all, a domain with only a limit is still staged"
        );
        assert!(
            !selected.includes("db01"),
            "under selected, giving a domain a limit must not select it"
        );
    }

    #[test]
    fn unknown_domains_are_included() {
        assert!(config().includes("never-seen"));
        assert!(config().includes("web01"));
        assert!(!config().includes("win-ci"));
    }

    #[test]
    fn selected_mode_stages_only_the_domains_turned_on() {
        let config = VmSnapshotConfig {
            selection: VmSelectionMode::Selected,
            ..config()
        };

        assert!(config.includes("web01"), "an included domain is staged");
        assert!(!config.includes("win-ci"), "an excluded domain is not");
        assert!(
            !config.includes("never-seen"),
            "a domain nobody selected is left alone rather than picked up"
        );
    }

    #[test]
    fn the_two_modes_disagree_only_about_untouched_domains() {
        let all = config();
        let selected = VmSnapshotConfig {
            selection: VmSelectionMode::Selected,
            ..config()
        };

        for domain in ["web01", "win-ci"] {
            assert_eq!(
                all.includes(domain),
                selected.includes(domain),
                "{domain} carries its own flag, so the mode must not override it"
            );
        }
        for domain in ["never-seen", "db01"] {
            assert_ne!(
                all.includes(domain),
                selected.includes(domain),
                "{domain} carries no decision, so the mode is what answers"
            );
        }
    }

    #[test]
    fn a_mode_says_what_an_untouched_domain_does() {
        assert!(VmSelectionMode::All.includes_untouched());
        assert!(!VmSelectionMode::Selected.includes_untouched());
        assert_eq!(VmSelectionMode::default(), VmSelectionMode::All);
    }

    #[test]
    fn selection_modes_survive_a_round_trip_through_their_wire_names() {
        for mode in [VmSelectionMode::All, VmSelectionMode::Selected] {
            let rendered = mode.to_string();
            assert_eq!(
                VmSelectionMode::from_str(&rendered),
                Ok(mode),
                "{rendered} must parse back to the mode that wrote it"
            );
        }
        assert_eq!(VmSelectionMode::All.to_string(), "all");
        assert_eq!(VmSelectionMode::Selected.to_string(), "selected");
    }

    #[test]
    fn a_config_without_a_selection_defaults_to_staging_everything() {
        let stored = r#"{
            "enabled": true,
            "staging_dir": "/srv/vm",
            "full_interval": 7,
            "timeout_seconds": 1800,
            "default_limit_bytes": 0
        }"#;
        let config: VmSnapshotConfig = serde_json::from_str(stored)
            .expect("a config predating the selection mode still deserializes");

        assert_eq!(config.selection, VmSelectionMode::All);
        assert!(config.includes("never-seen"));
    }

    #[test]
    fn build_actions_say_what_they_do() {
        assert!(!VmBuildAction::FilesOnly.defines());
        assert!(!VmBuildAction::FilesOnly.starts());
        assert!(VmBuildAction::Define.defines());
        assert!(!VmBuildAction::Define.starts());
        assert!(VmBuildAction::DefineAndStart.defines());
        assert!(VmBuildAction::DefineAndStart.starts());
    }

    #[test]
    fn live_states_are_the_ones_with_a_qemu_process() {
        assert!(VmState::Running.is_live());
        assert!(VmState::Paused.is_live());
        assert!(VmState::Suspended.is_live());
        assert!(!VmState::ShutOff.is_live());
        assert!(!VmState::Unknown.is_live());
    }

    #[test]
    fn states_round_trip_through_their_database_representation() {
        for state in [
            VmState::Running,
            VmState::Paused,
            VmState::ShutOff,
            VmState::Suspended,
            VmState::Unknown,
        ] {
            let text = state.to_string();
            assert_eq!(text.parse::<VmState>().expect("state parses"), state);
        }
        for mode in [
            VmSnapshotMode::Incremental,
            VmSnapshotMode::FullCopy,
            VmSnapshotMode::OfflineCopy,
            VmSnapshotMode::Excluded,
            VmSnapshotMode::Unknown,
        ] {
            let text = mode.to_string();
            assert_eq!(text.parse::<VmSnapshotMode>().expect("mode parses"), mode);
        }
    }

    #[test]
    fn vm_state_accepts_libvirts_own_spellings() {
        assert_eq!("shut off".parse::<VmState>(), Ok(VmState::ShutOff));
        assert_eq!("shutoff".parse::<VmState>(), Ok(VmState::ShutOff));
        assert_eq!("shut_off".parse::<VmState>(), Ok(VmState::ShutOff));
        assert_eq!("pmsuspended".parse::<VmState>(), Ok(VmState::Suspended));
        assert_eq!("suspended".parse::<VmState>(), Ok(VmState::Suspended));
        assert!("in shutdown".parse::<VmState>().is_err());
    }

    #[test]
    fn vm_state_still_renders_the_wire_spellings() {
        assert_eq!(VmState::ShutOff.to_string(), "shut_off");
        assert_eq!(VmState::Suspended.to_string(), "suspended");
    }
}

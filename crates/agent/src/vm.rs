// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Staging of libvirt/QEMU domains into a directory before a backup, so the
//! host's virtual machines end up in the archive as ordinary files.
//!
//! A running domain whose disks are all qcow2 goes through libvirt's
//! incremental backup API: the first run writes a full image and creates a
//! checkpoint, every later run writes only the clusters that changed since the
//! previous checkpoint. A domain that cannot do that is copied in full from a
//! temporary external snapshot, and a shut off domain is copied directly and
//! skipped entirely while its disks are unchanged.
//!
//! Every domain gets a budget below the staging directory. A chain that would
//! outgrow it is replaced by a new full image, which reclaims the increments;
//! a full image that cannot fit is refused before anything is deleted, so the
//! previous chain stays restorable.

use std::{
    collections::BTreeSet,
    convert::Infallible,
    fmt::Write as _,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    time::Duration,
};

use chrono::Utc;
use shared::vm::{
    DiscoveredVm, VmBuildOutcome, VmBuildRequest, VmRunAction, VmSnapshotConfig, VmSnapshotMode,
    VmSnapshotOutcome, VmState,
};
use tokio::process::Command;
use tracing::{info, warn};

/// Prefix of the checkpoints and temporary snapshots this agent owns, so a
/// checkpoint made by another tool is never deleted.
const OWNED_PREFIX: &str = "assimilate";

/// How often the agent asks libvirt whether a backup job has finished.
const JOB_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Name of the file recording which staged files belong to the current chain,
/// in the order they must be merged to restore the domain.
const CHAIN_FILE: &str = "chain.txt";

/// Something went wrong while staging a host's domains.
#[derive(Debug, thiserror::Error)]
pub enum VmError {
    /// A libvirt or qemu-img command could not be started at all.
    #[error("could not run {command}: {source}")]
    Spawn {
        /// The command that could not be started.
        command: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A command ran and failed.
    #[error("{command} failed: {stderr}")]
    Command {
        /// The command that failed.
        command: String,
        /// What the command wrote to stderr, trimmed.
        stderr: String,
    },
    /// A filesystem operation failed.
    #[error("{action}: {source}")]
    Io {
        /// What the agent was doing.
        action: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A backup job did not finish in time, or finished unsuccessfully.
    #[error("{0}")]
    Job(String),
    /// Staging would exceed the domain's budget.
    #[error("{0}")]
    OverLimit(String),
}

/// One writable disk of a domain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Disk {
    /// The target device name libvirt uses, such as `vda`.
    target: String,
    /// Path of the backing image or block device on the host.
    source: PathBuf,
}

/// Maps a `virsh domstate` line onto [`VmState`]. The spellings libvirt uses
/// are accepted by [`VmState`]'s own parser; anything else is unknown to this
/// build and reported as such rather than guessed at.
fn parse_domain_state(text: &str) -> VmState {
    VmState::from_str(text.trim()).unwrap_or_default()
}

/// The kind of block device libvirt reports in the `Device` column of
/// `domblklist --details`. Only `disk` carries guest state worth staging;
/// CD-ROMs, floppies and passed-through LUNs are left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BlockDeviceKind {
    /// A writable disk backed by an image or block device.
    Disk,
    /// A CD-ROM, which holds no guest state.
    Cdrom,
    /// A floppy device, which holds no guest state.
    Floppy,
    /// A SCSI LUN passed through to the guest.
    Lun,
    /// A device kind this build does not know, which is left alone.
    #[default]
    Unknown,
}

impl FromStr for BlockDeviceKind {
    type Err = Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text.trim() {
            "disk" => Self::Disk,
            "cdrom" => Self::Cdrom,
            "floppy" => Self::Floppy,
            "lun" => Self::Lun,
            _ => Self::Unknown,
        })
    }
}

/// The `Source` column of `domblklist --details`, which libvirt writes as `-`
/// for a device with nothing attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockSource(Option<PathBuf>);

impl FromStr for BlockSource {
    type Err = Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Self(match text.trim() {
            "" | "-" => None,
            path => Some(PathBuf::from(path)),
        }))
    }
}

/// The image format `qemu-img info` reports for a disk. Only qcow2 can carry
/// the dirty bitmaps an incremental backup is built on, so every other format
/// collapses into one variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DiskFormat {
    /// qcow2, the one format that supports persistent dirty bitmaps.
    Qcow2,
    /// Any other format, including one qemu-img did not report at all.
    #[default]
    Other,
}

impl FromStr for DiskFormat {
    type Err = Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text.trim() {
            "qcow2" => Self::Qcow2,
            _ => Self::Other,
        })
    }
}

/// The `Job type:` libvirt reports for a domain in `virsh domjobinfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum JobType {
    /// libvirt printed no job type at all, so there is no job to wait on.
    #[default]
    Absent,
    /// No job is running.
    None,
    /// A job is running and its end point is known.
    Bounded,
    /// A job is running without a known end point.
    Unbounded,
    /// The job finished successfully.
    Completed,
    /// The job failed.
    Failed,
    /// The job was cancelled.
    Cancelled,
    /// libvirt reported a job type this build does not know.
    Unknown,
}

impl FromStr for JobType {
    type Err = Infallible;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(match text.trim() {
            "" => Self::Absent,
            "None" => Self::None,
            "Bounded" => Self::Bounded,
            "Unbounded" => Self::Unbounded,
            "Completed" => Self::Completed,
            "Failed" => Self::Failed,
            "Cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        })
    }
}

impl JobType {
    /// Whether a job is still in flight and worth polling again.
    const fn is_running(self) -> bool {
        matches!(self, Self::Bounded | Self::Unbounded | Self::Unknown)
    }
}

/// Space a file actually occupies, counted in allocated blocks rather than
/// apparent size, so a sparse image is not charged for the holes in it.
fn allocated_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

/// Space a path occupies: for a directory, everything below it. Walked with
/// an explicit worklist rather than recursion, which an async function cannot
/// do without boxing every level.
async fn path_usage(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut pending = vec![path.to_path_buf()];

    while let Some(current) = pending.pop() {
        let Ok(metadata) = tokio::fs::symlink_metadata(&current).await else {
            continue;
        };
        if !metadata.is_dir() {
            total = total.saturating_add(allocated_bytes(&metadata));
            continue;
        }
        let Ok(mut entries) = tokio::fs::read_dir(&current).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            pending.push(entry.path());
        }
    }
    total
}

/// Whether a path is a regular file, without blocking the executor.
async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
}

/// Renders a byte count the way the operator sees it in the UI.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("TiB", 1 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ];
    for (unit, size) in UNITS {
        if bytes >= size {
            let whole = bytes.checked_div(size).unwrap_or(0);
            let tenths = bytes
                .checked_rem(size)
                .unwrap_or(0)
                .saturating_mul(10)
                .checked_div(size)
                .unwrap_or(0);
            return format!("{whole}.{tenths} {unit}");
        }
    }
    format!("{bytes} B")
}

/// The domain definition of a restored domain, edited so it can be defined
/// beside the domain it came from.
///
/// The edits are deliberately narrow: the top-level name, the UUID (dropped so
/// libvirt issues a new one rather than colliding with the original), the
/// source path of each disk, and the NVRAM path. Everything else about the
/// machine is left exactly as it was backed up.
struct DomainXml {
    text: String,
}

impl DomainXml {
    fn new(text: String) -> Self {
        Self { text }
    }

    /// Replaces the content of the first `<tag>` element, if there is one.
    fn replace_element(&mut self, tag: &str, value: &str) -> bool {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let Some(start) = self.text.find(&open) else {
            return false;
        };
        let Some(end) = self.text[start..]
            .find(&close)
            .and_then(|at| start.checked_add(at))
        else {
            return false;
        };
        let Some(from) = start.checked_add(open.len()) else {
            return false;
        };
        self.text.replace_range(from..end, value);
        true
    }

    /// Removes the first `<tag>...</tag>` element entirely.
    fn remove_element(&mut self, tag: &str) {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let Some(start) = self.text.find(&open) else {
            return;
        };
        let Some(end) = self.text[start..]
            .find(&close)
            .and_then(|at| start.checked_add(at))
        else {
            return;
        };
        let Some(end) = end.checked_add(close.len()) else {
            return;
        };
        self.text.replace_range(start..end, "");
    }

    /// The source path libvirt has for one disk target, found by scanning the
    /// `<disk>` element that carries that target.
    fn disk_source(&self, target: &str) -> Option<String> {
        for block in self.text.split("<disk ").skip(1) {
            let block = block.split("</disk>").next().unwrap_or(block);
            if Self::attribute(block, "dev").is_none_or(|dev| dev != target) {
                continue;
            }
            return Self::attribute(block, "file").or_else(|| Self::attribute(block, "dev_path"));
        }
        None
    }

    /// Reads an attribute value written with either quote style. Only the
    /// first occurrence in `block` is considered, which is what the disk
    /// elements need: `dev` names the target, `file` the source.
    fn attribute(block: &str, name: &str) -> Option<String> {
        for quote in ['\'', '"'] {
            let needle = format!("{name}={quote}");
            if let Some(start) = block.find(&needle)
                && let Some(from) = start.checked_add(needle.len())
                && let Some(end) = block.get(from..).and_then(|rest| rest.find(quote))
                && let Some(to) = from.checked_add(end)
                && let Some(value) = block.get(from..to)
            {
                return Some(value.to_owned());
            }
        }
        None
    }

    /// Points a path that appears in the definition at its new location.
    fn repath(&mut self, from: &str, to: &str) {
        for quote in ['\'', '"'] {
            self.text = self.text.replace(
                &format!("{quote}{from}{quote}"),
                &format!("{quote}{to}{quote}"),
            );
        }
    }

    /// Points `<nvram>` at the restored copy of the variables file. Its path is
    /// element text rather than an attribute, so [`Self::repath`], which
    /// matches a quoted value, cannot reach it. Leaving it alone would send the
    /// restored machine at the original domain's variables, which is either
    /// missing on this host or still in use by the domain it was copied from.
    fn repath_nvram(&mut self, to: &str) {
        let Some((head, rest)) = self.text.split_once("<nvram") else {
            return;
        };
        let Some((attrs, rest)) = rest.split_once('>') else {
            return;
        };
        let Some((_, tail)) = rest.split_once("</nvram>") else {
            return;
        };
        self.text = format!("{head}<nvram{attrs}>{to}</nvram>{tail}");
    }

    fn into_text(self) -> String {
        self.text
    }
}

/// One disk of a restored domain: which staged files make it up, in the order
/// they must be merged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainEntry {
    target: String,
    files: Vec<String>,
}

/// Reads `chain.txt` into one entry per disk, preserving the order the files
/// were written in, which is the order they must be merged in.
fn parse_chain(text: &str) -> Vec<ChainEntry> {
    let mut entries: Vec<ChainEntry> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(target), Some(file)) = (fields.next(), fields.next()) else {
            continue;
        };
        if let Some(entry) = entries.iter_mut().find(|entry| entry.target == target) {
            entry.files.push(file.to_owned());
        } else {
            entries.push(ChainEntry {
                target: target.to_owned(),
                files: vec![file.to_owned()],
            });
        }
    }
    entries
}

/// Stages a host's domains according to the settings the server delivered.
pub struct VmStager {
    config: VmSnapshotConfig,
    virsh: PathBuf,
    qemu_img: PathBuf,
    job_poll_interval: Duration,
    /// Extra environment variables injected into every command, used by tests
    /// to point the libvirt doubles at their own state directory.
    extra_env: Vec<(String, String)>,
}

impl VmStager {
    /// Builds a stager for `config`, using the libvirt tools on `PATH`.
    #[must_use]
    pub fn new(config: VmSnapshotConfig) -> Self {
        Self {
            config,
            virsh: std::env::var("VIRSH_BINARY")
                .map_or_else(|_| PathBuf::from("virsh"), PathBuf::from),
            qemu_img: std::env::var("QEMU_IMG_BINARY")
                .map_or_else(|_| PathBuf::from("qemu-img"), PathBuf::from),
            job_poll_interval: JOB_POLL_INTERVAL,
            extra_env: Vec::new(),
        }
    }

    /// Builds a stager against explicit binaries, so a test can drive the
    /// libvirt doubles without touching the process environment.
    #[cfg(test)]
    fn with_binaries(
        config: VmSnapshotConfig,
        virsh: PathBuf,
        qemu_img: PathBuf,
        extra_env: Vec<(String, String)>,
    ) -> Self {
        Self {
            config,
            virsh,
            qemu_img,
            job_poll_interval: Duration::from_millis(10),
            extra_env,
        }
    }

    /// Runs `virsh` with `args` and returns its standard output.
    async fn virsh(&self, args: &[&str]) -> Result<String, VmError> {
        self.run(&self.virsh, args).await
    }

    /// Runs `virsh` with `args`, reporting only whether it succeeded. Used
    /// where a failure is a fact about the host rather than an error, such as
    /// asking whether this libvirt knows a command at all.
    async fn virsh_ok(&self, args: &[&str]) -> bool {
        self.run(&self.virsh, args).await.is_ok()
    }

    /// Runs `binary` with `args` and returns its standard output.
    async fn run(&self, binary: &Path, args: &[&str]) -> Result<String, VmError> {
        let rendered = format!("{} {}", binary.display(), args.join(" "));
        let output = Command::new(binary)
            .args(args)
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|source| VmError::Spawn {
                command: rendered.clone(),
                source,
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(VmError::Command {
                command: rendered,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// The domains defined on this host, in the order libvirt lists them.
    async fn list_domains(&self) -> Result<Vec<String>, VmError> {
        Ok(self
            .virsh(&["list", "--all", "--name"])
            .await?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    /// The run state of one domain.
    async fn domain_state(&self, domain: &str) -> VmState {
        self.virsh(&["domstate", domain])
            .await
            .map_or(VmState::Unknown, |output| {
                parse_domain_state(output.lines().next().unwrap_or_default())
            })
    }

    /// The writable disks of one domain. CD-ROMs and empty devices are left
    /// out: they carry no guest state worth staging.
    async fn domain_disks(&self, domain: &str) -> Result<Vec<Disk>, VmError> {
        let output = self.virsh(&["domblklist", "--details", domain]).await?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let [_type, device, target, source] = fields.as_slice() else {
                    return None;
                };
                if BlockDeviceKind::from_str(device).unwrap_or_default() != BlockDeviceKind::Disk {
                    return None;
                }
                let BlockSource(Some(source)) =
                    BlockSource::from_str(source).unwrap_or(BlockSource(None))
                else {
                    return None;
                };
                Some(Disk {
                    target: (*target).to_owned(),
                    source,
                })
            })
            .collect())
    }

    /// Whether every disk is qcow2, which is what libvirt needs to keep the
    /// dirty bitmaps an incremental backup is built on.
    async fn all_disks_qcow2(&self, disks: &[Disk]) -> bool {
        for disk in disks {
            let Ok(info) = self
                .run(
                    &self.qemu_img,
                    &["info", "-U", &disk.source.to_string_lossy()],
                )
                .await
            else {
                return false;
            };
            let format = info
                .lines()
                .find_map(|line| line.trim().strip_prefix("file format: "))
                .map(DiskFormat::from_str)
                .and_then(Result::ok)
                .unwrap_or_default();
            if format != DiskFormat::Qcow2 {
                return false;
            }
        }
        !disks.is_empty()
    }

    /// How a domain would be captured, given its state and disks.
    async fn mode_for(&self, domain: &str, state: VmState, disks: &[Disk]) -> VmSnapshotMode {
        if !self.config.includes(domain) {
            return VmSnapshotMode::Excluded;
        }
        if !state.is_live() {
            return if state == VmState::Unknown {
                VmSnapshotMode::Unknown
            } else {
                VmSnapshotMode::OfflineCopy
            };
        }
        if self.virsh_ok(&["help", "backup-begin"]).await && self.all_disks_qcow2(disks).await {
            VmSnapshotMode::Incremental
        } else {
            VmSnapshotMode::FullCopy
        }
    }

    /// Builds a domain out of files a restore put back on disk: merges each
    /// disk's chain, moves the merged images into place, points a copy of the
    /// definition at them, and defines the domain under its new name.
    ///
    /// The merge rewrites the restored copy, which is why this works on a
    /// restore target and never on a host's staging directory.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::Io`] when the source directory is not a staged
    /// domain, or [`VmError::Command`] when qemu-img or libvirt refuses.
    pub async fn build(&self, request: &VmBuildRequest) -> Result<VmBuildOutcome, VmError> {
        let source = PathBuf::from(&request.source_dir);
        let image_dir = PathBuf::from(&request.image_dir);

        let chain_text = tokio::fs::read_to_string(source.join(CHAIN_FILE))
            .await
            .map_err(|e| VmError::Io {
                action: format!("read {}", source.join(CHAIN_FILE).display()),
                source: e,
            })?;
        let chain = parse_chain(&chain_text);
        if chain.is_empty() {
            return Err(VmError::Job(format!(
                "{} holds no chain.txt entries, so it is not a staged domain",
                source.display()
            )));
        }

        let definition = tokio::fs::read_to_string(source.join("domain.xml"))
            .await
            .map_err(|e| VmError::Io {
                action: format!("read {}", source.join("domain.xml").display()),
                source: e,
            })?;
        let mut xml = DomainXml::new(definition);

        tokio::fs::create_dir_all(&image_dir)
            .await
            .map_err(|e| VmError::Io {
                action: format!("create {}", image_dir.display()),
                source: e,
            })?;

        let mut images = Vec::with_capacity(chain.len());
        let mut merged_increments: u32 = 0;

        for entry in &chain {
            let (merged, increments) = self.merge_disk(&source, entry).await?;
            merged_increments = merged_increments.saturating_add(increments);

            let extension = merged.extension().map_or_else(
                || "img".to_owned(),
                |ext| ext.to_string_lossy().into_owned(),
            );
            let placed = image_dir.join(format!("{}-{}.{extension}", request.name, entry.target));
            tokio::fs::copy(&merged, &placed)
                .await
                .map_err(|e| VmError::Io {
                    action: format!("place {}", placed.display()),
                    source: e,
                })?;

            if let Some(old) = xml.disk_source(&entry.target) {
                xml.repath(&old, &placed.to_string_lossy());
            }
            images.push(placed.to_string_lossy().into_owned());
        }

        // A UEFI domain keeps its variables beside its images, so the restored
        // machine boots the way it did rather than falling back to defaults.
        let nvram = source.join("nvram.fd");
        if is_file(&nvram).await {
            let placed = image_dir.join(format!("{}-nvram.fd", request.name));
            tokio::fs::copy(&nvram, &placed)
                .await
                .map_err(|e| VmError::Io {
                    action: format!("place {}", placed.display()),
                    source: e,
                })?;
            xml.repath_nvram(&placed.to_string_lossy());
        }

        xml.replace_element("name", &request.name);
        // Dropped rather than rewritten: libvirt issues a fresh UUID, so the
        // restored domain never collides with the one it came from.
        xml.remove_element("uuid");

        let outcome = VmBuildOutcome {
            name: request.name.clone(),
            images,
            merged_increments,
            defined: false,
            started: false,
        };

        if !request.action.defines() {
            return Ok(outcome);
        }

        let definition_path = source.join(format!(".assimilate-{}.xml", request.name));
        tokio::fs::write(&definition_path, xml.into_text())
            .await
            .map_err(|e| VmError::Io {
                action: format!("write {}", definition_path.display()),
                source: e,
            })?;

        let defined = self
            .virsh(&["define", &definition_path.to_string_lossy()])
            .await;
        let _ = tokio::fs::remove_file(&definition_path).await;
        defined?;

        let started = if request.action.starts() {
            self.virsh(&["start", &request.name]).await?;
            true
        } else {
            false
        };

        info!(domain = request.name, started, "restored domain defined");
        Ok(VmBuildOutcome {
            defined: true,
            started,
            ..outcome
        })
    }

    /// Merges one disk's chain into its full image and returns the merged
    /// file, plus how many increments went into it. A chain of one file (a
    /// copy, or a full image on its own) is already the merged image.
    async fn merge_disk(
        &self,
        source: &Path,
        entry: &ChainEntry,
    ) -> Result<(PathBuf, u32), VmError> {
        let mut files = entry.files.iter();
        let Some(base_name) = files.next() else {
            return Err(VmError::Job(format!(
                "disk {} has no files in chain.txt",
                entry.target
            )));
        };
        let base = source.join(base_name);
        if !is_file(&base).await {
            return Err(VmError::Job(format!(
                "{} is missing, so the chain of {} cannot be merged",
                base.display(),
                entry.target
            )));
        }

        let mut merged: u32 = 0;
        for increment_name in files {
            let increment = source.join(increment_name);
            if !is_file(&increment).await {
                return Err(VmError::Job(format!(
                    "{} is missing, so the chain of {} is incomplete",
                    increment.display(),
                    entry.target
                )));
            }
            // Point the increment at the image it was taken against, then fold
            // it in. Both steps are what a hand restore does, in the order
            // chain.txt records.
            self.run(
                &self.qemu_img,
                &[
                    "rebase",
                    "-u",
                    "-F",
                    "qcow2",
                    "-b",
                    &base.to_string_lossy(),
                    &increment.to_string_lossy(),
                ],
            )
            .await?;
            self.run(&self.qemu_img, &["commit", &increment.to_string_lossy()])
                .await?;
            merged = merged.saturating_add(1);
        }

        Ok((base, merged))
    }

    /// The NVRAM path a definition carries, when it has one.
    fn nvram_path(xml: &str) -> Option<String> {
        xml.split_once("<nvram")
            .and_then(|(_, rest)| rest.split_once('>'))
            .and_then(|(_, rest)| rest.split_once("</nvram>"))
            .map(|(path, _)| path.trim().to_owned())
            .filter(|path| !path.is_empty())
    }

    /// Enumerates this host's domains without changing anything.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::Command`] or [`VmError::Spawn`] when libvirt cannot
    /// be reached at all.
    pub async fn scan(&self) -> Result<Vec<DiscoveredVm>, VmError> {
        let mut found = Vec::new();
        for domain in self.list_domains().await? {
            let state = self.domain_state(&domain).await;
            let disks = self.domain_disks(&domain).await.unwrap_or_default();
            let mode = self.mode_for(&domain, state, &disks).await;
            let disk_bytes = Self::full_size(&disks).await;

            found.push(DiscoveredVm {
                name: domain,
                state,
                mode,
                disk_count: u32::try_from(disks.len()).unwrap_or(u32::MAX),
                disk_bytes,
            });
        }
        Ok(found)
    }

    /// The directory one domain is staged into.
    fn dest_for(&self, domain: &str) -> PathBuf {
        Path::new(&self.config.staging_dir).join(domain)
    }

    /// Stages every included domain, one after another, and reports what it
    /// did to each. A domain that fails does not stop the others: the caller
    /// decides whether the backup goes ahead.
    pub async fn stage_all(&self) -> Vec<VmSnapshotOutcome> {
        let domains = match self.list_domains().await {
            Ok(domains) => domains,
            Err(e) => {
                warn!(error = %e, "could not list the domains of this host");
                return Vec::new();
            }
        };

        let mut outcomes = Vec::with_capacity(domains.len());
        for domain in domains {
            outcomes.push(self.stage_domain(&domain).await);
        }
        outcomes
    }

    /// Stages one domain, turning any failure into an outcome the server can
    /// show against that domain.
    async fn stage_domain(&self, domain: &str) -> VmSnapshotOutcome {
        let dest = self.dest_for(domain);
        let mut outcome = VmSnapshotOutcome {
            name: domain.to_owned(),
            action: VmRunAction::Skipped,
            mode: VmSnapshotMode::Excluded,
            staged_bytes: path_usage(&dest).await,
            chain_length: 0,
            error: None,
        };

        if !self.config.includes(domain) {
            info!(domain, "excluded from staging");
            return outcome;
        }

        match self.stage_included(domain, &dest).await {
            Ok(staged) => staged,
            Err(e) => {
                warn!(domain, error = %e, "could not stage domain");
                outcome.error = Some(e.to_string());
                outcome.staged_bytes = path_usage(&dest).await;
                outcome
            }
        }
    }

    /// Stages a domain the operator has not excluded.
    async fn stage_included(
        &self,
        domain: &str,
        dest: &Path,
    ) -> Result<VmSnapshotOutcome, VmError> {
        tokio::fs::create_dir_all(dest)
            .await
            .map_err(|source| VmError::Io {
                action: format!("create {}", dest.display()),
                source,
            })?;

        self.save_definition(domain, dest).await?;

        let disks = self.domain_disks(domain).await?;
        if disks.is_empty() {
            info!(domain, "no writable disks, only the definition was saved");
            return Ok(VmSnapshotOutcome {
                name: domain.to_owned(),
                action: VmRunAction::Unchanged,
                mode: VmSnapshotMode::OfflineCopy,
                staged_bytes: path_usage(dest).await,
                chain_length: 0,
                error: None,
            });
        }

        let state = self.domain_state(domain).await;
        let mode = self.mode_for(domain, state, &disks).await;
        let limit = self.config.limit_for(domain);

        let action = match mode {
            VmSnapshotMode::Incremental => {
                self.stage_with_checkpoints(domain, &disks, dest, limit)
                    .await?
            }
            VmSnapshotMode::FullCopy => {
                self.copy_running(domain, &disks, dest, limit).await?;
                VmRunAction::Copy
            }
            VmSnapshotMode::OfflineCopy => self.copy_offline(domain, &disks, dest, limit).await?,
            VmSnapshotMode::Excluded | VmSnapshotMode::Unknown => {
                return Err(VmError::Job(format!(
                    "domain {domain} is in a state this agent cannot stage"
                )));
            }
        };

        let staged_bytes = path_usage(dest).await;
        if limit > 0 && staged_bytes > limit {
            return Err(VmError::OverLimit(format!(
                "staged {} exceeds the limit of {}. Raise the limit for this domain or lower the \
                 full-image interval.",
                format_bytes(staged_bytes),
                format_bytes(limit)
            )));
        }

        Ok(VmSnapshotOutcome {
            name: domain.to_owned(),
            action,
            mode,
            staged_bytes,
            chain_length: self.chain_increments(dest).await,
            error: None,
        })
    }

    /// Saves the domain definition and, for a UEFI domain, its NVRAM: without
    /// them the images cannot be turned back into a machine.
    async fn save_definition(&self, domain: &str, dest: &Path) -> Result<(), VmError> {
        let xml = match self.virsh(&["dumpxml", "--inactive", domain]).await {
            Ok(xml) => xml,
            Err(_) => self.virsh(&["dumpxml", domain]).await?,
        };

        let definition = dest.join("domain.xml");
        tokio::fs::write(&definition, &xml)
            .await
            .map_err(|source| VmError::Io {
                action: format!("write {}", definition.display()),
                source,
            })?;

        if let Some(nvram) = Self::nvram_path(&xml) {
            let source = PathBuf::from(nvram);
            if is_file(&source).await {
                tokio::fs::copy(&source, dest.join("nvram.fd"))
                    .await
                    .map_err(|e| VmError::Io {
                        action: format!("copy {}", source.display()),
                        source: e,
                    })?;
            }
        }
        Ok(())
    }

    /// The checkpoints this agent owns for a domain, oldest first: their names
    /// carry a timestamp, so sorting them by name sorts them by age.
    async fn owned_checkpoints(&self, domain: &str) -> Vec<String> {
        let listed = self
            .virsh(&["checkpoint-list", domain, "--name"])
            .await
            .unwrap_or_default();
        let mut checkpoints: BTreeSet<String> = BTreeSet::new();
        for line in listed.lines().map(str::trim) {
            if line.starts_with(&format!("{OWNED_PREFIX}-")) {
                checkpoints.insert(line.to_owned());
            }
        }
        checkpoints.into_iter().collect()
    }

    /// Drops every checkpoint this agent owns for a domain, so the next run
    /// starts a fresh chain.
    async fn drop_checkpoints(&self, domain: &str) {
        for checkpoint in self.owned_checkpoints(domain).await {
            if self
                .virsh(&["checkpoint-delete", domain, &checkpoint])
                .await
                .is_err()
                && self
                    .virsh(&["checkpoint-delete", domain, &checkpoint, "--metadata"])
                    .await
                    .is_err()
            {
                warn!(domain, checkpoint, "could not delete checkpoint");
            }
        }
    }

    /// Removes the staged images of a domain, keeping its definition.
    async fn clear_images(&self, dest: &Path) {
        let Ok(mut entries) = tokio::fs::read_dir(dest).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let is_image = path
                .extension()
                .is_some_and(|ext| ext == "qcow2" || ext == "img");
            if is_image {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
        let _ = tokio::fs::remove_file(dest.join(CHAIN_FILE)).await;
    }

    /// Reads how many increments the current chain holds.
    async fn chain_increments(&self, dest: &Path) -> u32 {
        let chain = tokio::fs::read_to_string(dest.join(CHAIN_FILE))
            .await
            .unwrap_or_default();
        let increments = chain
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.contains(".full.qcow2"))
            .count();
        u32::try_from(increments).unwrap_or(u32::MAX)
    }

    /// Appends one line per disk to the chain file, recording the merge order
    /// a restore has to follow.
    async fn record_chain(&self, dest: &Path, disks: &[Disk], suffix: &str) -> Result<(), VmError> {
        let mut chain = tokio::fs::read_to_string(dest.join(CHAIN_FILE))
            .await
            .unwrap_or_default();
        for disk in disks {
            let _ = writeln!(chain, "{} {}.{suffix}", disk.target, disk.target);
        }
        tokio::fs::write(dest.join(CHAIN_FILE), chain)
            .await
            .map_err(|source| VmError::Io {
                action: format!("write {}", dest.join(CHAIN_FILE).display()),
                source,
            })
    }

    /// Space a fresh full copy of these disks would need.
    async fn full_size(disks: &[Disk]) -> u64 {
        let mut total: u64 = 0;
        for disk in disks {
            total = total.saturating_add(path_usage(&disk.source).await);
        }
        total
    }

    /// Refuses a full copy that cannot fit, before anything is written or
    /// deleted, so the previous chain stays restorable.
    async fn check_full_fits(domain: &str, disks: &[Disk], limit: u64) -> Result<(), VmError> {
        if limit == 0 {
            return Ok(());
        }
        let needed = Self::full_size(disks).await;
        if needed > limit {
            return Err(VmError::OverLimit(format!(
                "a full backup of {domain} needs about {}, which exceeds the limit of {}",
                format_bytes(needed),
                format_bytes(limit)
            )));
        }
        Ok(())
    }

    /// Stages a live domain through libvirt's incremental backup API.
    async fn stage_with_checkpoints(
        &self,
        domain: &str,
        disks: &[Disk],
        dest: &Path,
        limit: u64,
    ) -> Result<VmRunAction, VmError> {
        let checkpoints = self.owned_checkpoints(domain).await;
        let chain_exists = is_file(&dest.join(CHAIN_FILE)).await;
        let used = path_usage(dest).await;

        let mut from = (!checkpoints.is_empty()
            && u32::try_from(checkpoints.len()).unwrap_or(u32::MAX) < self.config.full_interval
            && chain_exists)
            .then(|| checkpoints.last().cloned().unwrap_or_default());

        // An increment that would push the chain past its limit is replaced by
        // a new full image, which drops the increments and reclaims the space.
        if from.is_some() && limit > 0 {
            let estimate = self
                .largest_increment(dest)
                .await
                .max(used.checked_div(10).unwrap_or(0).saturating_add(1));
            if used.saturating_add(estimate) > limit {
                info!(
                    domain,
                    "chain is within {} of the {} limit, writing a new full image",
                    format_bytes(estimate),
                    format_bytes(limit)
                );
                from = None;
            }
        }

        // Milliseconds, not seconds: two runs of a small domain can otherwise
        // land in the same second and collide on a checkpoint name.
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
        let checkpoint_name = format!("{OWNED_PREFIX}-{stamp}");

        if from.is_none() {
            Self::check_full_fits(domain, disks, limit).await?;
            self.drop_checkpoints(domain).await;
            self.clear_images(dest).await;
        }

        let suffix = if from.is_some() {
            format!("{stamp}.qcow2")
        } else {
            "full.qcow2".to_owned()
        };

        let backup_xml = Self::backup_xml(dest, disks, from.as_deref(), &suffix);
        let checkpoint_xml = Self::checkpoint_xml(disks, &checkpoint_name);
        let backup_file = Self::write_temp(dest, "backup.xml", &backup_xml).await?;
        let checkpoint_file = Self::write_temp(dest, "checkpoint.xml", &checkpoint_xml).await?;

        for disk in disks {
            let _ = tokio::fs::remove_file(dest.join(format!("{}.{suffix}", disk.target))).await;
        }

        // Stale statistics from an earlier job would otherwise be read as this
        // job's result.
        let _ = self.virsh(&["domjobinfo", domain, "--completed"]).await;

        let started = self
            .virsh(&[
                "backup-begin",
                domain,
                &backup_file.to_string_lossy(),
                &checkpoint_file.to_string_lossy(),
            ])
            .await;
        let _ = tokio::fs::remove_file(&backup_file).await;
        let _ = tokio::fs::remove_file(&checkpoint_file).await;
        started?;

        self.wait_for_job(domain).await?;
        self.record_chain(dest, disks, &suffix).await?;

        Ok(if from.is_some() {
            VmRunAction::Increment
        } else {
            VmRunAction::FullImage
        })
    }

    /// The largest increment of the current chain, used to guess how big the
    /// next one will be.
    async fn largest_increment(&self, dest: &Path) -> u64 {
        let Ok(mut entries) = tokio::fs::read_dir(dest).await else {
            return 0;
        };
        let mut largest = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.ends_with(".qcow2") && !name.ends_with(".full.qcow2") {
                largest = largest.max(path_usage(&path).await);
            }
        }
        largest
    }

    /// Writes one of the XML documents libvirt reads, next to the staged
    /// images so a leftover file is obvious.
    async fn write_temp(dest: &Path, name: &str, body: &str) -> Result<PathBuf, VmError> {
        let path = dest.join(format!(".assimilate-{name}"));
        tokio::fs::write(&path, body)
            .await
            .map_err(|source| VmError::Io {
                action: format!("write {}", path.display()),
                source,
            })?;
        Ok(path)
    }

    /// The backup document libvirt reads: push mode, one target file per disk,
    /// and an `<incremental>` element when this run is an increment.
    fn backup_xml(dest: &Path, disks: &[Disk], from: Option<&str>, suffix: &str) -> String {
        let mut xml = String::from("<domainbackup mode=\"push\">\n");
        if let Some(from) = from {
            let _ = writeln!(xml, "  <incremental>{from}</incremental>");
        }
        xml.push_str("  <disks>\n");
        for disk in disks {
            let target = dest.join(format!("{}.{suffix}", disk.target));
            let _ = writeln!(
                xml,
                "    <disk name=\"{}\" backup=\"yes\" type=\"file\">\n      <driver \
                 type=\"qcow2\"/>\n      <target file=\"{}\"/>\n    </disk>",
                disk.target,
                target.display()
            );
        }
        xml.push_str("  </disks>\n</domainbackup>\n");
        xml
    }

    /// The checkpoint document libvirt reads, which is what makes the next run
    /// able to write only what changed.
    fn checkpoint_xml(disks: &[Disk], name: &str) -> String {
        let mut xml = format!("<domaincheckpoint>\n  <name>{name}</name>\n  <disks>\n");
        for disk in disks {
            let _ = writeln!(
                xml,
                "    <disk name=\"{}\" checkpoint=\"bitmap\"/>",
                disk.target
            );
        }
        xml.push_str("  </disks>\n</domaincheckpoint>\n");
        xml
    }

    /// Waits for a domain's backup job, then reports whether it completed.
    async fn wait_for_job(&self, domain: &str) -> Result<(), VmError> {
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(self.config.timeout_seconds.into()))
            .unwrap_or_else(tokio::time::Instant::now);

        loop {
            let running = self
                .virsh(&["domjobinfo", domain])
                .await
                .unwrap_or_default()
                .lines()
                .find_map(|line| line.trim().strip_prefix("Job type:"))
                .map(JobType::from_str)
                .and_then(Result::ok)
                .unwrap_or_default();

            if !running.is_running() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = self.virsh(&["domjobabort", domain]).await;
                return Err(VmError::Job(format!(
                    "the backup job for {domain} timed out after {} seconds",
                    self.config.timeout_seconds
                )));
            }
            tokio::time::sleep(self.job_poll_interval).await;
        }

        let reported = self
            .virsh(&["domjobinfo", domain, "--completed"])
            .await
            .unwrap_or_default()
            .lines()
            .find_map(|line| line.trim().strip_prefix("Job type:"))
            .map(|state| state.trim().to_owned())
            .unwrap_or_default();
        let completed = JobType::from_str(&reported).unwrap_or_default();

        if completed == JobType::Completed {
            Ok(())
        } else {
            Err(VmError::Job(format!(
                "the backup job for {domain} did not complete ({})",
                if reported.is_empty() {
                    "no job information"
                } else {
                    &reported
                }
            )))
        }
    }

    /// Copies a live domain that cannot do incremental backups, taking the
    /// copy from a temporary external snapshot so the images are consistent.
    async fn copy_running(
        &self,
        domain: &str,
        disks: &[Disk],
        dest: &Path,
        limit: u64,
    ) -> Result<(), VmError> {
        Self::check_full_fits(domain, disks, limit).await?;
        self.drop_checkpoints(domain).await;
        self.clear_images(dest).await;

        let snapshot = format!(
            "{OWNED_PREFIX}-tmp-{}",
            Utc::now().format("%Y%m%dT%H%M%S%3fZ")
        );
        let quiesced = self
            .virsh(&[
                "snapshot-create-as",
                "--domain",
                domain,
                "--name",
                &snapshot,
                "--disk-only",
                "--atomic",
                "--no-metadata",
                "--quiesce",
            ])
            .await
            .is_ok();

        if !quiesced {
            // No guest agent to freeze the file systems: the copy is
            // crash-consistent, which is what a hard reset would leave behind.
            self.virsh(&[
                "snapshot-create-as",
                "--domain",
                domain,
                "--name",
                &snapshot,
                "--disk-only",
                "--atomic",
                "--no-metadata",
            ])
            .await?;
            info!(domain, "snapshot taken without guest agent quiesce");
        }

        let mut copy_error = None;
        for disk in disks {
            let target = dest.join(format!("{}.img", disk.target));
            if let Err(source) = tokio::fs::copy(&disk.source, &target).await {
                copy_error = Some(VmError::Io {
                    action: format!("copy {}", disk.source.display()),
                    source,
                });
                break;
            }
        }

        // The overlay has to be merged back whether or not the copy worked,
        // otherwise the domain keeps running on it.
        for disk in disks {
            match self
                .virsh(&[
                    "blockcommit",
                    domain,
                    &disk.target,
                    "--active",
                    "--pivot",
                    "--wait",
                ])
                .await
            {
                Ok(_) => {
                    let overlay =
                        PathBuf::from(format!("{}.{snapshot}", disk.source.to_string_lossy()));
                    let _ = tokio::fs::remove_file(overlay).await;
                }
                Err(e) => {
                    warn!(
                        domain,
                        target = disk.target,
                        error = %e,
                        "could not commit the snapshot overlay, the domain is still running on it"
                    );
                    copy_error.get_or_insert(e);
                }
            }
        }

        if let Some(error) = copy_error {
            return Err(error);
        }
        self.record_chain(dest, disks, "img").await
    }

    /// Copies a shut off domain, skipping disks that have not changed since
    /// the last run.
    async fn copy_offline(
        &self,
        domain: &str,
        disks: &[Disk],
        dest: &Path,
        limit: u64,
    ) -> Result<VmRunAction, VmError> {
        Self::check_full_fits(domain, disks, limit).await?;
        self.drop_checkpoints(domain).await;

        let mut copied = false;
        let mut chain = String::new();
        for disk in disks {
            let target = dest.join(format!("{}.img", disk.target));
            if Self::is_unchanged(&disk.source, &target).await {
                info!(
                    domain,
                    target = disk.target,
                    "unchanged, keeping the previous copy"
                );
            } else {
                tokio::fs::copy(&disk.source, &target)
                    .await
                    .map_err(|source| VmError::Io {
                        action: format!("copy {}", disk.source.display()),
                        source,
                    })?;
                copied = true;
            }
            let _ = writeln!(chain, "{} {}.img", disk.target, disk.target);
        }

        // A domain that was staged incrementally while it was running leaves a
        // qcow2 chain behind; the copies replace it.
        let Ok(mut entries) = tokio::fs::read_dir(dest).await else {
            return Ok(VmRunAction::Copy);
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path().extension().is_some_and(|ext| ext == "qcow2") {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }

        tokio::fs::write(dest.join(CHAIN_FILE), chain)
            .await
            .map_err(|source| VmError::Io {
                action: format!("write {}", dest.join(CHAIN_FILE).display()),
                source,
            })?;

        Ok(if copied {
            VmRunAction::Copy
        } else {
            VmRunAction::Unchanged
        })
    }

    /// Whether a previous copy is still current, judged by modification time.
    async fn is_unchanged(source: &Path, copy: &Path) -> bool {
        let (Ok(source_meta), Ok(copy_meta)) = (
            tokio::fs::metadata(source).await,
            tokio::fs::metadata(copy).await,
        ) else {
            return false;
        };
        let (Ok(source_time), Ok(copy_time)) = (source_meta.modified(), copy_meta.modified())
        else {
            return false;
        };
        source_time <= copy_time
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// The single outcome of a one-domain host.
    fn only(outcomes: &[VmSnapshotOutcome]) -> &VmSnapshotOutcome {
        outcomes.first().expect("one outcome")
    }

    /// A host built out of the libvirt doubles in `tests/mock-virt`, so the
    /// staging logic is exercised end to end without libvirt.
    struct FakeHost {
        root: TempDir,
    }

    impl FakeHost {
        async fn new() -> Self {
            let root = TempDir::new().expect("temp dir");
            for dir in ["state", "images", "stage"] {
                tokio::fs::create_dir_all(root.path().join(dir))
                    .await
                    .expect("create dir");
            }
            Self { root }
        }

        fn state(&self) -> PathBuf {
            self.root.path().join("state")
        }

        fn staging_dir(&self) -> String {
            self.root
                .path()
                .join("stage")
                .to_string_lossy()
                .into_owned()
        }

        /// Defines a domain with one disk of `size_kib` allocated bytes.
        async fn define(&self, name: &str, state: &str, image: &str, size_kib: usize) -> PathBuf {
            let source = self.root.path().join("images").join(image);
            tokio::fs::write(&source, vec![0u8; size_kib.saturating_mul(1024)])
                .await
                .expect("write image");

            let state_dir = self.state();
            append(&state_dir.join("domains"), &format!("{name}\n")).await;
            append(&state_dir.join("states"), &format!("{name} {state}\n")).await;
            append(
                &state_dir.join(format!("disks-{name}")),
                &format!("file disk vda {}\n", source.display()),
            )
            .await;
            source
        }

        fn stager(&self, config: VmSnapshotConfig) -> VmStager {
            self.stager_with_env(config, Vec::new())
        }

        fn stager_with_env(
            &self,
            config: VmSnapshotConfig,
            mut extra_env: Vec<(String, String)>,
        ) -> VmStager {
            let mocks = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("tests")
                .join("mock-virt");
            extra_env.push((
                "MOCK_VIRT_STATE".to_owned(),
                self.state().to_string_lossy().into_owned(),
            ));
            VmStager::with_binaries(
                config,
                mocks.join("virsh"),
                mocks.join("qemu-img"),
                extra_env,
            )
        }

        fn config(&self) -> VmSnapshotConfig {
            VmSnapshotConfig {
                enabled: true,
                staging_dir: self.staging_dir(),
                ..VmSnapshotConfig::default()
            }
        }

        fn staged(&self, domain: &str) -> PathBuf {
            self.root.path().join("stage").join(domain)
        }

        async fn chain(&self, domain: &str) -> String {
            tokio::fs::read_to_string(self.staged(domain).join(CHAIN_FILE))
                .await
                .unwrap_or_default()
        }

        /// The backup document the double recorded for the most recent run.
        async fn last_backup_xml(&self, domain: &str) -> String {
            tokio::fs::read_to_string(self.state().join(format!("last-backup-{domain}.xml")))
                .await
                .unwrap_or_default()
        }

        async fn checkpoints(&self, domain: &str) -> Vec<String> {
            tokio::fs::read_to_string(self.state().join(format!("checkpoints-{domain}")))
                .await
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect()
        }
    }

    async fn append(path: &Path, line: &str) {
        let mut existing = tokio::fs::read_to_string(path).await.unwrap_or_default();
        existing.push_str(line);
        tokio::fs::write(path, existing).await.expect("append");
    }

    #[tokio::test]
    async fn scan_reports_the_domains_of_the_host() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 8).await;
        host.define("mail01", "shut off", "mail01.raw", 8).await;

        let found = host.stager(host.config()).scan().await.expect("scan");

        assert_eq!(found.len(), 2);
        let web = found.iter().find(|vm| vm.name == "web01").expect("web01");
        assert_eq!(web.state, VmState::Running);
        assert_eq!(web.mode, VmSnapshotMode::Incremental);
        assert_eq!(web.disk_count, 1);
        assert!(web.disk_bytes > 0);

        let mail = found.iter().find(|vm| vm.name == "mail01").expect("mail01");
        assert_eq!(mail.state, VmState::ShutOff);
        assert_eq!(mail.mode, VmSnapshotMode::OfflineCopy);
    }

    #[tokio::test]
    async fn scan_reports_an_excluded_domain_as_excluded() {
        let host = FakeHost::new().await;
        host.define("win-ci", "running", "win-ci.qcow2", 8).await;
        let mut config = host.config();
        config.domains = vec![shared::vm::VmDomainConfig {
            name: "win-ci".to_owned(),
            included: false,
            limit_bytes: None,
        }];

        let found = host.stager(config).scan().await.expect("scan");
        let vm = found.first().expect("one domain");
        assert_eq!(vm.mode, VmSnapshotMode::Excluded);
    }

    #[tokio::test]
    async fn the_first_run_writes_a_full_image_and_the_next_one_an_increment() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 8).await;
        let stager = host.stager(host.config());

        let first = stager.stage_all().await;
        assert_eq!(only(&first).action, VmRunAction::FullImage);
        assert_eq!(only(&first).mode, VmSnapshotMode::Incremental);
        assert!(only(&first).error.is_none(), "{:?}", only(&first).error);
        assert!(is_file(&host.staged("web01").join("vda.full.qcow2")).await);
        assert!(is_file(&host.staged("web01").join("domain.xml")).await);
        assert_eq!(host.chain("web01").await.trim(), "vda vda.full.qcow2");
        assert!(
            !host
                .last_backup_xml("web01")
                .await
                .contains("<incremental>")
        );
        assert_eq!(host.checkpoints("web01").await.len(), 1);

        let second = stager.stage_all().await;
        assert_eq!(only(&second).action, VmRunAction::Increment);
        assert!(
            host.last_backup_xml("web01")
                .await
                .contains("<incremental>assimilate-")
        );
        assert_eq!(host.chain("web01").await.lines().count(), 2);
        assert_eq!(host.checkpoints("web01").await.len(), 2);
        assert_eq!(only(&second).chain_length, 1);
    }

    #[tokio::test]
    async fn the_full_interval_starts_a_new_chain() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 8).await;
        let mut config = host.config();
        config.full_interval = 2;
        let stager = host.stager(config);

        stager.stage_all().await;
        stager.stage_all().await;
        let third = stager.stage_all().await;

        assert_eq!(only(&third).action, VmRunAction::FullImage);
        assert!(
            !host
                .last_backup_xml("web01")
                .await
                .contains("<incremental>")
        );
        assert_eq!(host.chain("web01").await.trim(), "vda vda.full.qcow2");
        assert_eq!(host.checkpoints("web01").await.len(), 1);
    }

    #[tokio::test]
    async fn a_chain_near_its_limit_is_replaced_by_a_new_full_image() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 8).await;
        let mut config = host.config();
        // Every staged image the double writes is 256 KiB, so the chain
        // outgrows a 700 KiB budget on the third run.
        config.default_limit_bytes = 700 * 1024;
        let stager = host.stager_with_env(
            config,
            vec![("MOCK_VIRT_BACKUP_KIB".to_owned(), "256".to_owned())],
        );

        stager.stage_all().await;
        let second = stager.stage_all().await;
        assert_eq!(only(&second).action, VmRunAction::Increment);

        let third = stager.stage_all().await;
        assert_eq!(only(&third).action, VmRunAction::FullImage);
        assert!(only(&third).error.is_none(), "{:?}", only(&third).error);
        assert!(only(&third).staged_bytes <= 700 * 1024);
    }

    #[tokio::test]
    async fn a_full_image_that_cannot_fit_is_refused_and_the_chain_survives() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 512).await;
        let mut config = host.config();
        config.default_limit_bytes = 100 * 1024 * 1024;
        let stager = host.stager_with_env(
            config.clone(),
            vec![("MOCK_VIRT_BACKUP_KIB".to_owned(), "256".to_owned())],
        );
        stager.stage_all().await;
        assert!(is_file(&host.staged("web01").join("vda.full.qcow2")).await);

        // The budget now cannot hold even the domain's own disks.
        config.default_limit_bytes = 64 * 1024;
        let starved = host.stager_with_env(
            config,
            vec![("MOCK_VIRT_BACKUP_KIB".to_owned(), "256".to_owned())],
        );
        let outcome = starved.stage_all().await;

        let error = only(&outcome).error.as_deref().expect("refused");
        assert!(error.contains("exceeds the limit"), "{error}");
        assert!(
            is_file(&host.staged("web01").join("vda.full.qcow2")).await,
            "the previous chain must survive a refused run"
        );
        assert_eq!(host.chain("web01").await.trim(), "vda vda.full.qcow2");
    }

    #[tokio::test]
    async fn an_excluded_domain_is_left_alone() {
        let host = FakeHost::new().await;
        host.define("win-ci", "running", "win-ci.qcow2", 8).await;
        let mut config = host.config();
        config.domains = vec![shared::vm::VmDomainConfig {
            name: "win-ci".to_owned(),
            included: false,
            limit_bytes: None,
        }];

        let outcomes = host.stager(config).stage_all().await;

        assert_eq!(only(&outcomes).action, VmRunAction::Skipped);
        assert_eq!(only(&outcomes).mode, VmSnapshotMode::Excluded);
        assert!(!is_file(&host.staged("win-ci")).await);
    }

    #[tokio::test]
    async fn a_shut_off_domain_is_copied_once_and_skipped_while_unchanged() {
        let host = FakeHost::new().await;
        let source = host.define("mail01", "shut off", "mail01.raw", 16).await;
        let stager = host.stager(host.config());

        let first = stager.stage_all().await;
        assert_eq!(only(&first).action, VmRunAction::Copy);
        assert_eq!(only(&first).mode, VmSnapshotMode::OfflineCopy);
        assert!(is_file(&host.staged("mail01").join("vda.img")).await);
        assert_eq!(host.chain("mail01").await.trim(), "vda vda.img");

        let second = stager.stage_all().await;
        assert_eq!(only(&second).action, VmRunAction::Unchanged);

        tokio::fs::write(&source, vec![1u8; 16usize.saturating_mul(1024)])
            .await
            .expect("touch image");
        let third = stager.stage_all().await;
        assert_eq!(only(&third).action, VmRunAction::Copy);
    }

    #[tokio::test]
    async fn a_running_domain_without_incremental_support_is_copied_from_a_snapshot() {
        let host = FakeHost::new().await;
        host.define("build01", "running", "build01.raw", 16).await;

        let outcomes = host.stager(host.config()).stage_all().await;

        assert_eq!(only(&outcomes).action, VmRunAction::Copy);
        assert_eq!(only(&outcomes).mode, VmSnapshotMode::FullCopy);
        assert!(is_file(&host.staged("build01").join("vda.img")).await);

        let calls = tokio::fs::read_to_string(host.state().join("calls.log"))
            .await
            .expect("calls");
        assert!(calls.contains("snapshot-create-as --domain build01"));
        assert!(calls.contains("blockcommit build01 vda --active --pivot"));
        let mut images = tokio::fs::read_dir(host.root.path().join("images"))
            .await
            .expect("images");
        let mut overlays = 0;
        while let Ok(Some(entry)) = images.next_entry().await {
            if entry
                .file_name()
                .to_string_lossy()
                .contains(".assimilate-tmp-")
            {
                overlays += 1;
            }
        }
        assert_eq!(overlays, 0, "the snapshot overlay must be committed away");
    }

    #[tokio::test]
    async fn a_domain_without_a_guest_agent_is_snapshotted_without_quiesce() {
        let host = FakeHost::new().await;
        host.define("build01", "running", "build01.raw", 16).await;
        let stager = host.stager_with_env(
            host.config(),
            vec![("MOCK_VIRT_NO_QUIESCE".to_owned(), "1".to_owned())],
        );

        let outcomes = stager.stage_all().await;

        assert_eq!(only(&outcomes).action, VmRunAction::Copy);
        assert!(
            only(&outcomes).error.is_none(),
            "{:?}",
            only(&outcomes).error
        );
        let calls = tokio::fs::read_to_string(host.state().join("calls.log"))
            .await
            .expect("calls");
        assert!(
            calls
                .lines()
                .any(|line| line.contains("snapshot-create-as") && !line.contains("--quiesce"))
        );
    }

    #[tokio::test]
    async fn a_failing_backup_job_is_reported_against_the_domain() {
        let host = FakeHost::new().await;
        host.define("web01", "running", "web01.qcow2", 8).await;
        let stager = host.stager_with_env(
            host.config(),
            vec![("MOCK_VIRT_FAIL_BACKUP".to_owned(), "1".to_owned())],
        );

        let outcomes = stager.stage_all().await;

        assert!(only(&outcomes).error.is_some());
        assert!(!is_file(&host.staged("web01").join("vda.full.qcow2")).await);
    }

    /// A definition in the shape libvirt writes, with the two attribute quote
    /// styles it uses in practice.
    const DOMAIN_XML: &str = r#"<domain type='kvm'>
  <name>web01</name>
  <uuid>4dea22b3-1d52-d8f3-2516-782e98ab3fa0</uuid>
  <memory unit='KiB'>4194304</memory>
  <os>
    <type arch='x86_64' machine='q35'>hvm</type>
    <loader readonly='yes' type='pflash'>/usr/share/OVMF/OVMF_CODE.fd</loader>
    <nvram>/var/lib/libvirt/qemu/nvram/web01_VARS.fd</nvram>
  </os>
  <devices>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2'/>
      <source file='/var/lib/libvirt/images/web01.qcow2'/>
      <target dev='vda' bus='virtio'/>
    </disk>
    <disk type="file" device="disk">
      <driver name="qemu" type="qcow2"/>
      <source file="/var/lib/libvirt/images/web01-data.qcow2"/>
      <target dev="vdb" bus="virtio"/>
    </disk>
    <disk type='file' device='cdrom'>
      <target dev='sda' bus='sata'/>
    </disk>
    <interface type='bridge'>
      <mac address='52:54:00:6b:3c:58'/>
    </interface>
  </devices>
</domain>
"#;

    #[test]
    fn a_chain_file_becomes_one_entry_per_disk_in_merge_order() {
        let chain = parse_chain(
            "vda vda.full.qcow2\nvdb vdb.full.qcow2\nvda vda.20260903T020000000Z.qcow2\nvdb \
             vdb.20260903T020000000Z.qcow2\n",
        );

        assert_eq!(chain.len(), 2);
        let vda = chain.first().expect("vda entry");
        assert_eq!(vda.target, "vda");
        assert_eq!(
            vda.files,
            vec!["vda.full.qcow2", "vda.20260903T020000000Z.qcow2"]
        );
        assert_eq!(chain.get(1).expect("vdb entry").target, "vdb");
    }

    #[test]
    fn a_blank_or_malformed_chain_line_is_ignored() {
        let chain = parse_chain("\nvda vda.full.qcow2\nnonsense\n\n");
        assert_eq!(chain.len(), 1);
        assert_eq!(
            chain.first().expect("one entry").files,
            vec!["vda.full.qcow2"]
        );
    }

    #[test]
    fn the_definition_yields_the_source_of_each_disk_whichever_quotes_it_uses() {
        let xml = DomainXml::new(DOMAIN_XML.to_owned());
        assert_eq!(
            xml.disk_source("vda").as_deref(),
            Some("/var/lib/libvirt/images/web01.qcow2")
        );
        assert_eq!(
            xml.disk_source("vdb").as_deref(),
            Some("/var/lib/libvirt/images/web01-data.qcow2")
        );
        // A cdrom carries no source, and an unknown target is not there.
        assert_eq!(xml.disk_source("sda"), None);
        assert_eq!(xml.disk_source("vdz"), None);
    }

    #[test]
    fn a_restored_definition_is_renamed_repathed_and_stripped_of_its_uuid() {
        let mut xml = DomainXml::new(DOMAIN_XML.to_owned());
        xml.repath(
            "/var/lib/libvirt/images/web01.qcow2",
            "/var/lib/libvirt/images/web01-restored-vda.qcow2",
        );
        xml.repath(
            "/var/lib/libvirt/images/web01-data.qcow2",
            "/var/lib/libvirt/images/web01-restored-vdb.qcow2",
        );
        xml.replace_element("name", "web01-restored");
        xml.remove_element("uuid");
        let text = xml.into_text();

        assert!(text.contains("<name>web01-restored</name>"));
        assert!(!text.contains("4dea22b3"));
        assert!(text.contains("'/var/lib/libvirt/images/web01-restored-vda.qcow2'"));
        assert!(text.contains("\"/var/lib/libvirt/images/web01-restored-vdb.qcow2\""));
        // The machine itself is untouched: same MAC, same firmware, same disks.
        assert!(text.contains("52:54:00:6b:3c:58"));
        assert!(text.contains("<type arch='x86_64' machine='q35'>hvm</type>"));
    }

    #[test]
    fn the_nvram_path_is_read_from_the_definition() {
        assert_eq!(
            VmStager::nvram_path(DOMAIN_XML).as_deref(),
            Some("/var/lib/libvirt/qemu/nvram/web01_VARS.fd")
        );
        assert_eq!(
            VmStager::nvram_path("<domain><name>x</name></domain>"),
            None
        );
    }

    /// Lays out a restored domain directory the way stage one leaves it.
    async fn restored_domain(host: &FakeHost, chain: &str) -> PathBuf {
        let dir = host.root.path().join("restored").join("web01");
        tokio::fs::create_dir_all(&dir).await.expect("restore dir");
        tokio::fs::write(dir.join("domain.xml"), DOMAIN_XML)
            .await
            .expect("definition");
        tokio::fs::write(dir.join(CHAIN_FILE), chain)
            .await
            .expect("chain");
        for line in chain.lines() {
            if let Some(file) = line.split_whitespace().nth(1) {
                tokio::fs::write(dir.join(file), vec![0u8; 4096])
                    .await
                    .expect("image");
            }
        }
        dir
    }

    #[tokio::test]
    async fn building_merges_the_chain_places_the_image_and_defines_the_domain() {
        let host = FakeHost::new().await;
        let source = restored_domain(
            &host,
            "vda vda.full.qcow2\nvda vda.20260902T020000000Z.qcow2\nvda \
             vda.20260903T020000000Z.qcow2\n",
        )
        .await;
        let images = host.root.path().join("images-restored");

        let outcome = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect("build");

        assert_eq!(outcome.name, "web01-restored");
        assert_eq!(outcome.merged_increments, 2);
        assert!(outcome.defined);
        assert!(!outcome.started);
        assert!(is_file(&images.join("web01-restored-vda.qcow2")).await);

        let calls = tokio::fs::read_to_string(host.state().join("calls.log"))
            .await
            .expect("calls");
        // Each increment is rebased onto the full image and then committed,
        // oldest first, before the domain is defined.
        let rebases = calls.lines().filter(|line| line.contains("rebase")).count();
        let commits = calls.lines().filter(|line| line.contains("commit")).count();
        assert_eq!(rebases, 2);
        assert_eq!(commits, 2);
        assert!(calls.contains("virsh define"));
        assert!(!calls.contains("virsh start"));

        // The definition it defined from is the restored one, renamed, pointed
        // at the placed image and stripped of the UUID it would collide on.
        let defined = tokio::fs::read_to_string(host.state().join("last-defined.xml"))
            .await
            .expect("definition");
        assert!(defined.contains("<name>web01-restored</name>"));
        assert!(!defined.contains("4dea22b3"));
        assert!(defined.contains(&format!(
            "'{}'",
            images.join("web01-restored-vda.qcow2").display()
        )));
        assert!(defined.contains("52:54:00:6b:3c:58"));
    }

    #[tokio::test]
    async fn a_single_file_chain_needs_no_merge() {
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.img\n").await;
        let images = host.root.path().join("images-restored");

        let outcome = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "mail01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::FilesOnly,
            })
            .await
            .expect("build");

        assert_eq!(outcome.merged_increments, 0);
        assert!(!outcome.defined);
        assert!(is_file(&images.join("mail01-restored-vda.img")).await);

        let calls = tokio::fs::read_to_string(host.state().join("calls.log"))
            .await
            .unwrap_or_default();
        assert!(!calls.contains("virsh define"));
    }

    #[tokio::test]
    async fn building_can_start_the_domain_it_defined() {
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.full.qcow2\n").await;
        let images = host.root.path().join("images-restored");

        let outcome = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::DefineAndStart,
            })
            .await
            .expect("build");

        assert!(outcome.defined);
        assert!(outcome.started);
        let calls = tokio::fs::read_to_string(host.state().join("calls.log"))
            .await
            .expect("calls");
        assert!(calls.contains("virsh start web01-restored"));
    }

    #[tokio::test]
    async fn an_incomplete_chain_is_refused_before_anything_is_placed() {
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.full.qcow2\n").await;
        tokio::fs::write(
            source.join(CHAIN_FILE),
            "vda vda.full.qcow2\nvda vda.20260903T020000000Z.qcow2\n",
        )
        .await
        .expect("chain");
        let images = host.root.path().join("images-restored");

        let error = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect_err("incomplete chain");

        assert!(error.to_string().contains("incomplete"), "{error}");
        assert!(!is_file(&images.join("web01-restored-vda.qcow2")).await);
    }

    #[tokio::test]
    async fn a_directory_that_is_not_a_staged_domain_is_refused() {
        let host = FakeHost::new().await;
        let empty = host.root.path().join("empty");
        tokio::fs::create_dir_all(&empty).await.expect("dir");

        let error = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: empty.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: host
                    .root
                    .path()
                    .join("images-restored")
                    .to_string_lossy()
                    .into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect_err("not a staged domain");

        assert!(error.to_string().contains("chain.txt"), "{error}");
    }

    #[test]
    fn libvirt_block_device_columns_parse_into_enums() {
        assert_eq!(BlockDeviceKind::from_str("disk"), Ok(BlockDeviceKind::Disk));
        assert_eq!(
            BlockDeviceKind::from_str("cdrom"),
            Ok(BlockDeviceKind::Cdrom)
        );
        assert_eq!(
            BlockDeviceKind::from_str("nvme"),
            Ok(BlockDeviceKind::Unknown)
        );
        assert_eq!(BlockSource::from_str("-"), Ok(BlockSource(None)));
        assert_eq!(BlockSource::from_str(""), Ok(BlockSource(None)));
        assert_eq!(
            BlockSource::from_str("/var/lib/libvirt/images/a.qcow2"),
            Ok(BlockSource(Some(PathBuf::from(
                "/var/lib/libvirt/images/a.qcow2"
            ))))
        );
    }

    #[test]
    fn qemu_img_formats_and_libvirt_job_types_parse_into_enums() {
        assert_eq!(DiskFormat::from_str("qcow2"), Ok(DiskFormat::Qcow2));
        assert_eq!(DiskFormat::from_str("raw"), Ok(DiskFormat::Other));
        assert_eq!(DiskFormat::from_str(""), Ok(DiskFormat::Other));

        assert_eq!(JobType::from_str(""), Ok(JobType::Absent));
        assert_eq!(JobType::from_str(" None"), Ok(JobType::None));
        assert_eq!(JobType::from_str("Completed"), Ok(JobType::Completed));
        assert_eq!(JobType::from_str("Bounded"), Ok(JobType::Bounded));
        assert_eq!(JobType::from_str("Sideways"), Ok(JobType::Unknown));

        assert!(JobType::Bounded.is_running());
        assert!(JobType::Unbounded.is_running());
        assert!(!JobType::None.is_running());
        assert!(!JobType::Absent.is_running());
        assert!(!JobType::Completed.is_running());
    }

    #[test]
    fn libvirt_states_map_onto_the_shared_enum() {
        assert_eq!(parse_domain_state("running"), VmState::Running);
        assert_eq!(parse_domain_state("shut off"), VmState::ShutOff);
        assert_eq!(parse_domain_state("paused\n"), VmState::Paused);
        assert_eq!(parse_domain_state("pmsuspended"), VmState::Suspended);
        assert_eq!(parse_domain_state("in shutdown"), VmState::Unknown);
    }

    #[test]
    fn byte_counts_render_in_the_units_the_ui_uses() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(3 << 30), "3.0 GiB");
    }

    #[tokio::test]
    async fn a_full_backup_is_refused_only_when_the_disks_do_not_fit() {
        let host = FakeHost::new().await;
        let source = host.root.path().join("images").join("web01.qcow2");
        tokio::fs::create_dir_all(host.root.path().join("images"))
            .await
            .expect("images dir");
        tokio::fs::write(&source, vec![0u8; 64 * 1024])
            .await
            .expect("write image");
        let disks = vec![Disk {
            target: "vda".to_owned(),
            source,
        }];

        assert!(VmStager::check_full_fits("web01", &disks, 0).await.is_ok());
        assert!(
            VmStager::check_full_fits("web01", &disks, 1024 * 1024)
                .await
                .is_ok()
        );
        assert!(
            VmStager::check_full_fits("web01", &disks, 1024)
                .await
                .is_err()
        );
    }

    #[test]
    fn the_backup_document_asks_for_an_increment_only_when_there_is_one() {
        let disks = vec![Disk {
            target: "vda".to_owned(),
            source: PathBuf::from("/var/lib/libvirt/images/web01.qcow2"),
        }];
        let full = VmStager::backup_xml(Path::new("/stage/web01"), &disks, None, "full.qcow2");
        assert!(!full.contains("<incremental>"));
        assert!(full.contains("/stage/web01/vda.full.qcow2"));

        let increment = VmStager::backup_xml(
            Path::new("/stage/web01"),
            &disks,
            Some("assimilate-20260903T020000Z"),
            "20260904T020000Z.qcow2",
        );
        assert!(increment.contains("<incremental>assimilate-20260903T020000Z</incremental>"));
        assert!(increment.contains("/stage/web01/vda.20260904T020000Z.qcow2"));
    }

    #[test]
    fn the_checkpoint_document_names_every_disk() {
        let disks = vec![
            Disk {
                target: "vda".to_owned(),
                source: PathBuf::from("/images/a.qcow2"),
            },
            Disk {
                target: "vdb".to_owned(),
                source: PathBuf::from("/images/b.qcow2"),
            },
        ];
        let xml = VmStager::checkpoint_xml(&disks, "assimilate-20260904T020000Z");
        assert!(xml.contains("<name>assimilate-20260904T020000Z</name>"));
        assert!(xml.contains("<disk name=\"vda\" checkpoint=\"bitmap\"/>"));
        assert!(xml.contains("<disk name=\"vdb\" checkpoint=\"bitmap\"/>"));
    }

    #[tokio::test]
    async fn a_domain_with_no_writable_disks_stages_only_its_definition() {
        // A machine whose only device is a CD-ROM still has a definition worth
        // keeping, so it is staged rather than failed or skipped.
        let host = FakeHost::new().await;
        append(&host.state().join("domains"), "iso-only\n").await;
        append(&host.state().join("states"), "iso-only shut off\n").await;
        tokio::fs::write(host.state().join("disks-iso-only"), "file cdrom sda -\n")
            .await
            .expect("disks");

        let outcomes = host.stager(host.config()).stage_all().await;
        let outcome = only(&outcomes);

        assert_eq!(outcome.action, VmRunAction::Unchanged);
        assert_eq!(outcome.mode, VmSnapshotMode::OfflineCopy);
        assert_eq!(outcome.chain_length, 0);
        assert!(outcome.error.is_none());
        assert!(is_file(&host.staged("iso-only").join("domain.xml")).await);
        assert_eq!(host.chain("iso-only").await, "");
    }

    #[tokio::test]
    async fn a_uefi_domains_nvram_is_staged_beside_its_definition() {
        // Without the variables file a restored UEFI machine boots to defaults
        // and loses its boot entries, so it is staged with the images.
        let host = FakeHost::new().await;
        host.define("uefi01", "shut off", "uefi01.qcow2", 64).await;
        let vars = host.root.path().join("images").join("uefi01_VARS.fd");
        tokio::fs::write(&vars, vec![7u8; 2048])
            .await
            .expect("nvram");
        tokio::fs::write(
            host.state().join("nvram-uefi01"),
            vars.to_string_lossy().as_bytes(),
        )
        .await
        .expect("nvram pointer");

        let outcomes = host.stager(host.config()).stage_all().await;

        assert!(only(&outcomes).error.is_none());
        let staged = host.staged("uefi01").join("nvram.fd");
        assert!(is_file(&staged).await, "nvram staged");
        assert_eq!(
            tokio::fs::read(&staged).await.expect("read nvram"),
            vec![7u8; 2048]
        );
    }

    #[tokio::test]
    async fn a_uefi_domains_nvram_is_placed_and_repathed_on_restore() {
        // The restored machine points at its own copy of the variables, not at
        // the path the original domain used.
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.full.qcow2\n").await;
        tokio::fs::write(source.join("nvram.fd"), vec![9u8; 1024])
            .await
            .expect("nvram");
        let images = host.root.path().join("images-restored");

        host.stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect("build");

        let placed = images.join("web01-restored-nvram.fd");
        assert!(is_file(&placed).await, "nvram placed");
        let defined = tokio::fs::read_to_string(host.state().join("last-defined.xml"))
            .await
            .expect("definition");
        assert!(defined.contains(&placed.to_string_lossy().into_owned()));
        assert!(!defined.contains("/var/lib/libvirt/qemu/nvram/web01_VARS.fd"));
    }

    #[tokio::test]
    async fn a_staged_domain_whose_chain_is_empty_is_refused() {
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.full.qcow2\n").await;
        tokio::fs::write(source.join(CHAIN_FILE), "\n   \n")
            .await
            .expect("chain");

        let error = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: host
                    .root
                    .path()
                    .join("images-restored")
                    .to_string_lossy()
                    .into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect_err("no chain entries");

        assert!(error.to_string().contains("chain.txt"), "{error}");
    }

    #[tokio::test]
    async fn a_disk_whose_base_image_is_missing_is_refused() {
        let host = FakeHost::new().await;
        let source = restored_domain(&host, "vda vda.full.qcow2\n").await;
        tokio::fs::remove_file(source.join("vda.full.qcow2"))
            .await
            .expect("remove base");
        let images = host.root.path().join("images-restored");

        let error = host
            .stager(host.config())
            .build(&VmBuildRequest {
                source_dir: source.to_string_lossy().into_owned(),
                name: "web01-restored".to_owned(),
                image_dir: images.to_string_lossy().into_owned(),
                action: shared::vm::VmBuildAction::Define,
            })
            .await
            .expect_err("missing base image");

        assert!(error.to_string().contains("missing"), "{error}");
        // Refused before anything was placed, so a half-built domain is not
        // left behind for the operator to clean up.
        assert!(!is_file(&images.join("web01-restored-vda.qcow2")).await);
    }
}

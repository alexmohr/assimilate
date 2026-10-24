// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Filesystem snapshots a backup reads from instead of the live files.
//!
//! Right before `borg create`, after the pre-backup commands, the agent
//! snapshots the ZFS dataset, btrfs subvolume or LVM logical volume under
//! each backup source and every disk filesystem mounted below one. borg
//! then runs in a private mount namespace where each snapshot stands in for
//! its origin under the original path (see [`namespace`]), so the archive
//! holds the original paths and browsing, search, diff and restore work as
//! they always do. Once borg is done the snapshots are removed, also when
//! the run fails or is cancelled, and a record of each one survives an
//! agent crash so the next start removes it.

#[cfg(target_os = "linux")]
mod mountinfo;
#[cfg(target_os = "linux")]
mod namespace;
#[cfg(target_os = "linux")]
mod origin;
#[cfg(target_os = "linux")]
mod record;
mod tools;

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;

#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
pub use namespace::MountPlan;
use shared::{
    protocol::AgentToServer,
    snapshot::FsSnapshotMode,
    types::{RepoId, RunEventType},
};
use tokio::sync::mpsc;
pub use tools::Tools;

/// Where a run's steps go on the run's timeline.
#[derive(Debug, Clone)]
pub struct RunEvents {
    pub outbound: mpsc::Sender<AgentToServer>,
    pub run_id: String,
    pub repo_id: RepoId,
}

impl RunEvents {
    async fn send(&self, event_type: RunEventType, message: impl Into<String>) {
        let event = AgentToServer::RunEvent {
            run_id: self.run_id.clone(),
            repo_id: self.repo_id,
            event_type,
            message: message.into(),
        };
        if self.outbound.send(event).await.is_err() {
            tracing::debug!("the run's timeline is gone; a snapshot step was not recorded");
        }
    }
}

async fn report(events: Option<&RunEvents>, event_type: RunEventType, message: &str) {
    if let Some(events) = events {
        events.send(event_type, message).await;
    }
}

/// What a backup asks to snapshot.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct Request {
    /// The backup sources.
    pub sources: Vec<String>,
    pub mode: FsSnapshotMode,
    /// Size of an LVM snapshot, in percent of its volume.
    pub lvm_percent: u8,
    /// Names this run's snapshots: unique per run, letters, digits and `-`.
    pub tag: String,
    /// Directories below a source that must stay live: borg's own state
    /// and the database dump pipes.
    pub keep_live: Vec<PathBuf>,
    pub events: Option<RunEvents>,
    pub tools: Tools,
    /// Where a run's records go.
    pub record_dir: PathBuf,
    /// Below which an LVM snapshot is mounted while borg reads it.
    pub mount_root: PathBuf,
    /// How long to wait for a new LVM snapshot's device to appear.
    pub device_wait: std::time::Duration,
}

/// How long a new LVM snapshot's device may take to appear. lvcreate waits
/// for udev to make it unless udev synchronisation is off, so this is a
/// margin, not something a run normally spends.
#[cfg(target_os = "linux")]
pub const DEVICE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Where LVM snapshots are mounted while borg reads them.
#[cfg(target_os = "linux")]
pub const MOUNT_ROOT: &str = "/run/assimilate/snapshots";

/// The directories borg writes while it runs, from its environment: its
/// cache, configuration, security and key directories, and the temporary
/// directory. A snapshot is read-only, so these must stay live.
#[cfg(target_os = "linux")]
#[must_use]
pub fn borg_dirs(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let base = env("BORG_BASE_DIR")
        .or_else(|| env("HOME"))
        .map_or_else(|| PathBuf::from("/root"), PathBuf::from);
    let named = |variable: &str, default: PathBuf| env(variable).map_or(default, PathBuf::from);
    let mut dirs = vec![
        named("BORG_CACHE_DIR", base.join(".cache/borg")),
        named("BORG_CONFIG_DIR", base.join(".config/borg")),
        std::env::temp_dir(),
    ];
    for variable in ["BORG_SECURITY_DIR", "BORG_KEYS_DIR"] {
        if let Some(dir) = env(variable) {
            dirs.push(PathBuf::from(dir));
        }
    }
    dirs
}

/// The snapshots a run took. Remove them with [`Self::remove`] once borg is
/// done; dropped without that, as a cancelled run is, they are removed in
/// the background once borg has had time to stop.
#[derive(Debug)]
pub struct Snapshots {
    #[cfg(target_os = "linux")]
    taken: Vec<record::Taken>,
    #[cfg(target_os = "linux")]
    record: Option<record::Record>,
    #[cfg(target_os = "linux")]
    plan: Option<std::sync::Arc<MountPlan>>,
    /// How long a dropped run waits for borg to stop before removing them.
    #[cfg(target_os = "linux")]
    reap_after: std::time::Duration,
    warnings: Vec<String>,
    #[cfg(target_os = "linux")]
    tools: Tools,
    #[cfg(target_os = "linux")]
    events: Option<RunEvents>,
}

impl Snapshots {
    /// What borg's namespace mounts, when a snapshot was taken.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn plan(&self) -> Option<std::sync::Arc<MountPlan>> {
        self.plan.clone()
    }

    /// What the backup should say about sources it read live.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Removes every snapshot, newest first, and the run's record. Returns
    /// why each one that could not be removed was not; its record is kept
    /// for the agent's next start.
    #[cfg(target_os = "linux")]
    pub async fn remove(mut self) -> Vec<String> {
        self.remove_all().await
    }

    /// Without Linux nothing was snapshotted, so nothing is removed.
    #[cfg(not(target_os = "linux"))]
    pub fn remove(self) -> impl Future<Output = Vec<String>> {
        drop(self);
        std::future::ready(Vec::new())
    }

    #[cfg(target_os = "linux")]
    async fn remove_all(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        let mounts = mountinfo::MountTable::read().await.unwrap_or_default();
        // Each one is forgotten only once its removal is over, so a run
        // dropped meanwhile still leaves the rest to [`Drop`].
        while let Some(snapshot) = self.taken.last().cloned() {
            let what = snapshot.describe();
            let removed = snapshot.remove(&self.tools, &mounts).await;
            self.taken.pop();
            match removed {
                Ok(()) => {
                    report(self.events.as_ref(), RunEventType::SnapshotRemoved, &what).await;
                }
                Err(e) => {
                    // Forgotten without being discarded, the record stays
                    // for the agent's next start.
                    self.record = None;
                    let failure = format!("could not remove the snapshot of {what}: {e}");
                    report(self.events.as_ref(), RunEventType::SnapshotFailed, &failure).await;
                    failures.push(failure);
                }
            }
        }
        if let Some(record) = self.record.take() {
            record.discard().await;
        }
        failures
    }
}

#[cfg(target_os = "linux")]
impl Drop for Snapshots {
    fn drop(&mut self) {
        if !self.taken.is_empty() {
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                // Without a runtime the record stays, and the agent's next
                // start removes the snapshots.
                return;
            };
            let mut leftover = Leftover(Self {
                taken: std::mem::take(&mut self.taken),
                record: self.record.take(),
                plan: None,
                reap_after: self.reap_after,
                warnings: Vec::new(),
                tools: self.tools.clone(),
                events: self.events.take(),
            });
            handle.spawn(async move {
                // borg is being stopped; a snapshot it still reads cannot
                // be removed, so wait out its kill escalation.
                tokio::time::sleep(leftover.0.reap_after).await;
                for failure in leftover.0.remove_all().await {
                    tracing::warn!("{failure}");
                }
            });
        }
    }
}

/// A dropped run's snapshots, removed in the background. Should the
/// runtime stop first, they are forgotten, and their record leaves them to
/// the agent's next start.
#[cfg(target_os = "linux")]
struct Leftover(Snapshots);

#[cfg(target_os = "linux")]
impl Drop for Leftover {
    fn drop(&mut self) {
        self.0.taken.clear();
    }
}

/// Extra time after borg's kill escalation before a cancelled run's
/// snapshots are removed.
#[cfg(target_os = "linux")]
const REAP_MARGIN: std::time::Duration = std::time::Duration::from_secs(2);

/// Where each run's record of its snapshots is kept.
#[cfg(target_os = "linux")]
#[must_use]
pub fn record_dir() -> PathBuf {
    record::Record::dir()
}

/// Removes the snapshots runs left behind when the agent stopped mid-run,
/// as the records in `dir` name them. Call it before any backup starts.
#[cfg(target_os = "linux")]
pub async fn remove_leftovers(dir: &Path, tools: &Tools) {
    let mounts = mountinfo::MountTable::read().await.unwrap_or_default();
    for failure in record::remove_leftovers(dir, tools, &mounts).await {
        tracing::warn!("{failure}");
    }
}

/// Snapshots what `request.sources` live on. With
/// [`FsSnapshotMode::Required`], a source that cannot be snapshotted fails
/// the run and every snapshot taken so far is removed again; with
/// [`FsSnapshotMode::BestEffort`] it is read live and the result's warnings
/// say so.
///
/// # Errors
///
/// Returns why a required snapshot could not be taken.
#[cfg(target_os = "linux")]
pub async fn take(request: Request) -> Result<Snapshots, String> {
    let table = mountinfo::MountTable::read()
        .await
        .map_err(|e| format!("could not read the mount table: {e}"))?;
    take_between(request, &table, mountinfo::MountTable::read).await
}

/// Without Linux there is nothing to snapshot with: a run that requires
/// snapshots fails, and one that takes them where it can reads its sources
/// live.
///
/// # Errors
///
/// Returns an error when `mode` requires snapshots.
#[cfg(not(target_os = "linux"))]
pub async fn unavailable(
    mode: FsSnapshotMode,
    events: Option<&RunEvents>,
) -> Result<Snapshots, String> {
    let reason = "filesystem snapshots need a Linux host".to_owned();
    report(events, RunEventType::SnapshotFailed, &reason).await;
    match mode {
        FsSnapshotMode::Required => Err(reason),
        FsSnapshotMode::Off | FsSnapshotMode::BestEffort => Ok(Snapshots {
            warnings: vec![format!("{reason}; the sources were backed up live")],
        }),
    }
}

/// [`take`] with the mounts `table` lists, which nothing mounts below
/// meanwhile.
#[cfg(all(test, target_os = "linux"))]
async fn take_on(request: Request, table: &mountinfo::MountTable) -> Result<Snapshots, String> {
    take_between(request, table, || std::future::ready(Ok(table.clone()))).await
}

/// [`take`] with the mounts `table` lists before the snapshots are taken,
/// and those `reread` lists once they are: whatever was mounted below a
/// source meanwhile is in no snapshot.
#[cfg(target_os = "linux")]
async fn take_between<F, Reread>(
    request: Request,
    table: &mountinfo::MountTable,
    reread: Reread,
) -> Result<Snapshots, String>
where
    Reread: FnOnce() -> F,
    F: Future<Output = std::io::Result<mountinfo::MountTable>>,
{
    let found = origin::find(&request.sources, table, &request.tools).await;
    let events = request.events.clone();
    let mut snapshots = Snapshots {
        taken: Vec::new(),
        record: Some(record::Record::new(&request.record_dir, &request.tag)),
        plan: None,
        reap_after: shared::borg::kill_escalation_delay().saturating_add(REAP_MARGIN),
        warnings: Vec::new(),
        tools: request.tools.clone(),
        events: request.events.clone(),
    };
    let mut unsnapshotted: Vec<String> = found
        .unsnapshottable
        .iter()
        .map(|skipped| {
            format!(
                "{} cannot be snapshotted: {}",
                skipped.path.display(),
                skipped.reason
            )
        })
        .collect();
    let mut binds = Vec::new();
    let mut keep_live = Vec::new();
    if unsnapshotted.is_empty() || request.mode == FsSnapshotMode::BestEffort {
        for dir in &request.keep_live {
            // A live directory is bound back over its place in the
            // snapshot, so it must exist before the snapshot is taken. The
            // namespace follows no symbolic link onto it, so it is named by
            // its real path.
            let _ = tokio::fs::create_dir_all(dir).await;
            keep_live.push(
                tokio::fs::canonicalize(dir)
                    .await
                    .unwrap_or_else(|_| dir.clone()),
            );
        }
        for origin in &found.origins {
            match take_one(origin, &request, &mut snapshots).await {
                Ok(from) => {
                    binds.push((from, origin.covers().to_path_buf()));
                    let message = format!("Snapshotted {}", origin.describe());
                    report(events.as_ref(), RunEventType::SnapshotCreated, &message).await;
                }
                Err(e) => {
                    unsnapshotted.push(format!(
                        "{} could not be snapshotted: {e}",
                        origin.describe()
                    ));
                    if request.mode == FsSnapshotMode::Required {
                        break;
                    }
                }
            }
        }
    }
    if !binds.is_empty() {
        // Not in a snapshot, so read live, if at all.
        let (reasons, live) = mounted_meanwhile(table, reread().await, &binds);
        unsnapshotted.extend(reasons);
        keep_live.extend(live);
    }
    for reason in &unsnapshotted {
        report(events.as_ref(), RunEventType::SnapshotFailed, reason).await;
    }
    if request.mode == FsSnapshotMode::Required && !unsnapshotted.is_empty() {
        for failure in snapshots.remove_all().await {
            tracing::warn!("{failure}");
        }
        return Err(unsnapshotted.join("; "));
    }
    snapshots.warnings = unsnapshotted
        .into_iter()
        .map(|reason| format!("{reason}; it was backed up live"))
        .collect();
    if !binds.is_empty() {
        match namespace::MountPlan::new(&binds, table, &keep_live) {
            Ok(plan) => snapshots.plan = Some(std::sync::Arc::new(plan)),
            Err(e) => {
                for failure in snapshots.remove_all().await {
                    tracing::warn!("{failure}");
                }
                let reason = format!("a snapshot path cannot be mounted: {e}");
                if request.mode == FsSnapshotMode::Required {
                    return Err(reason);
                }
                // If possible: every source is read live instead.
                report(events.as_ref(), RunEventType::SnapshotFailed, &reason).await;
                snapshots
                    .warnings
                    .push(format!("{reason}; the sources were backed up live"));
            }
        }
    }
    Ok(snapshots)
}

/// The mount points `after` lists below an origin of `binds` that `before`
/// did not, leaving out the snapshots' own mounts, such as a ZFS snapshot
/// mounted where it was first read: why each is in no snapshot, and the
/// points. A table that could not be read again is a reason of its own.
#[cfg(target_os = "linux")]
fn mounted_meanwhile(
    before: &mountinfo::MountTable,
    after: std::io::Result<mountinfo::MountTable>,
    binds: &[(PathBuf, PathBuf)],
) -> (Vec<String>, Vec<PathBuf>) {
    let after = match after {
        Ok(after) => after,
        Err(e) => {
            let reason =
                format!("could not read the mount table after the snapshots were taken: {e}");
            return (vec![reason], Vec::new());
        }
    };
    let mut new = Vec::new();
    for (_, origin) in binds {
        let known = before.mount_points_below(origin);
        for point in after.mount_points_below(origin) {
            let own = binds
                .iter()
                .any(|(from, _)| from.starts_with(&point) || point.starts_with(from));
            if !own && !known.contains(&point) && !new.contains(&point) {
                new.push(point);
            }
        }
    }
    let reasons = new
        .iter()
        .map(|point| {
            format!(
                "{} was mounted while the snapshots were taken",
                point.display()
            )
        })
        .collect();
    (reasons, new)
}

/// Takes the snapshot of `origin` and returns the directory borg reads it
/// from. The snapshot is noted in the run's record before it is taken.
#[cfg(target_os = "linux")]
async fn take_one(
    origin: &origin::Origin,
    request: &Request,
    snapshots: &mut Snapshots,
) -> Result<PathBuf, String> {
    use origin::Origin;
    use record::Taken;
    let tag = &request.tag;
    let tools = &request.tools;
    let failed = |e: tools::ToolError| e.to_string();
    match origin {
        Origin::Zfs {
            dataset,
            mount_point,
        } => {
            let name = format!("assimilate-{tag}");
            let from = mount_point.join(".zfs/snapshot").join(&name);
            let snapshot = format!("{dataset}@{name}");
            note(
                snapshots,
                Taken::Zfs {
                    dataset: dataset.clone(),
                    name,
                },
            )
            .await?;
            tools::run(&tools.zfs, &["snapshot", &snapshot])
                .await
                .map_err(failed)?;
            // ZFS mounts a snapshot when its directory is first read; read
            // it here, so borg's namespace starts with it mounted.
            drop(
                tokio::fs::read_dir(&from).await.map_err(|e| {
                    format!("could not open the snapshot at {}: {e}", from.display())
                })?,
            );
            Ok(from)
        }
        Origin::Btrfs { subvolume } => {
            let path = subvolume.join(format!(".assimilate-snapshot-{tag}"));
            note(snapshots, Taken::Btrfs { path: path.clone() }).await?;
            tools::run(
                &tools.btrfs,
                &[
                    "subvolume".as_ref(),
                    "snapshot".as_ref(),
                    "-r".as_ref(),
                    subvolume.as_os_str(),
                    path.as_os_str(),
                ],
            )
            .await
            .map_err(failed)?;
            Ok(path)
        }
        Origin::Lvm {
            vg,
            lv,
            thin,
            fstype,
            root,
            ..
        } => {
            let name = format!("assimilate-{lv}-{tag}");
            let mount_dir = request.mount_root.join(tag).join(format!("{vg}-{lv}"));
            note(
                snapshots,
                Taken::Lvm {
                    vg: vg.clone(),
                    name: name.clone(),
                    mount_dir: mount_dir.clone(),
                },
            )
            .await?;
            let origin_lv = format!("{vg}/{lv}");
            let extents = format!("{}%ORIGIN", request.lvm_percent);
            let mut args = vec!["--snapshot", "--name", name.as_str()];
            if *thin {
                // A thin snapshot shares the pool and takes no size, but
                // is created inactive unless told otherwise.
                args.extend(["--setactivationskip", "n", "--activate", "y"]);
            } else {
                args.extend(["--extents", extents.as_str()]);
            }
            args.push(origin_lv.as_str());
            tools::run(&tools.lvcreate, &args).await.map_err(failed)?;
            tokio::fs::create_dir_all(&mount_dir)
                .await
                .map_err(|e| format!("could not create {}: {e}", mount_dir.display()))?;
            let device = PathBuf::from("/dev").join(vg).join(&name);
            // Should it still be missing, mount says so.
            wait_for(&device, request.device_wait).await;
            tools::run(
                &tools.mount,
                &[
                    "-t".as_ref(),
                    fstype.as_ref(),
                    "-o".as_ref(),
                    origin::mount_options(fstype).as_ref(),
                    device.as_os_str(),
                    mount_dir.as_os_str(),
                ],
            )
            .await
            .map_err(failed)?;
            Ok(mount_dir.join(root.strip_prefix("/").unwrap_or(root)))
        }
    }
}

/// Waits until `path` exists, for at most `limit`.
#[cfg(target_os = "linux")]
async fn wait_for(path: &Path, limit: std::time::Duration) {
    let _ = tokio::time::timeout(limit, async {
        while !tokio::fs::try_exists(path).await.unwrap_or(false) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
}

/// Notes `taken` in the run's record, then among the run's snapshots.
#[cfg(target_os = "linux")]
async fn note(snapshots: &mut Snapshots, taken: record::Taken) -> Result<(), String> {
    // Only removing the snapshots takes the record away.
    let record = snapshots
        .record
        .as_mut()
        .ok_or("the run's record is gone")?;
    record
        .note(taken.clone())
        .await
        .map_err(|e| format!("could not record the snapshot before taking it: {e}"))?;
    snapshots.taken.push(taken);
    Ok(())
}

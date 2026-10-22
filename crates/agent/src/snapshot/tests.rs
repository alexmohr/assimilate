// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::{Path, PathBuf};

use shared::{
    protocol::AgentToServer,
    snapshot::FsSnapshotMode,
    types::{RepoId, RunEventType},
};
use tokio::sync::mpsc;

use super::{mountinfo::MountTable, origin::Origin, tools::fake, *};

/// A zfs that snapshots but refuses to destroy.
const ZFS_KEEPS: &str = r#"[ "$1" = destroy ] && { echo busy >&2; exit 1; }; exit 0"#;

/// A temporary directory standing in for a filesystem, by its real path so
/// the mount table matches what a source canonicalizes to.
async fn filesystem() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = tokio::fs::canonicalize(dir.path()).await.unwrap();
    (dir, path)
}

fn request(
    tools: Tools,
    sources: &[&Path],
    mode: FsSnapshotMode,
    state: &Path,
) -> (Request, mpsc::Receiver<AgentToServer>) {
    let (outbound, events) = mpsc::channel(64);
    let request = Request {
        sources: sources
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        mode,
        lvm_percent: 10,
        tag: "manual-1".to_owned(),
        keep_live: Vec::new(),
        events: Some(RunEvents {
            outbound,
            run_id: "run-1".to_owned(),
            repo_id: RepoId(3),
        }),
        tools,
        record_dir: state.join("records"),
        mount_root: state.join("mounts"),
    };
    (request, events)
}

/// The timeline events sent so far.
fn sent(events: &mut mpsc::Receiver<AgentToServer>) -> Vec<(RunEventType, String)> {
    let mut sent = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let AgentToServer::RunEvent {
            run_id,
            repo_id,
            event_type,
            message,
        } = event
        {
            assert_eq!((run_id.as_str(), repo_id), ("run-1", RepoId(3)));
            sent.push((event_type, message));
        }
    }
    sent
}

fn types(sent: &[(RunEventType, String)]) -> Vec<RunEventType> {
    sent.iter().map(|(event_type, _)| *event_type).collect()
}

fn zfs_table(mounts: &[(&Path, &str)]) -> MountTable {
    let lines: Vec<String> = mounts
        .iter()
        .enumerate()
        .map(|(index, (point, dataset))| {
            format!(
                "{index} 0 0:4{index} / {} rw - zfs {dataset} rw",
                point.display()
            )
        })
        .collect();
    MountTable::parse(&lines.join("\n"))
}

/// Makes the directory ZFS would show a snapshot of `point` under.
async fn zfs_snapshot_dir(point: &Path) {
    tokio::fs::create_dir_all(point.join(".zfs/snapshot/assimilate-manual-1"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_zfs_source_is_read_from_its_snapshot_and_removed_after() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let source = point.join("data");
    tokio::fs::create_dir(&source).await.unwrap();
    zfs_snapshot_dir(&point).await;
    let tools = fake::tools(tools_dir.path(), "").await;
    let (mut request, mut events) =
        request(tools, &[&source], FsSnapshotMode::Required, state.path());
    let cache = point.join("cache/borg");
    request.keep_live = vec![cache.clone()];
    let table = zfs_table(&[(&point, "tank/data")]);

    let snapshots = take_on(request, &table).await.unwrap();
    assert_eq!(snapshots.warnings().len(), 0);
    assert_eq!(snapshots.plan().unwrap().live_count(), 1);
    assert!(
        tokio::fs::try_exists(&cache).await.unwrap(),
        "a live directory exists before the snapshot is taken"
    );
    let record = state.path().join("records/manual-1.json");
    assert!(tokio::fs::try_exists(&record).await.unwrap());
    let created = sent(&mut events);
    assert_eq!(types(&created), [RunEventType::SnapshotCreated]);
    assert!(
        created.first().unwrap().1.starts_with("Snapshotted ZFS"),
        "{created:?}"
    );

    assert_eq!(snapshots.remove().await.len(), 0);
    assert_eq!(
        fake::calls(tools_dir.path()).await,
        [
            "zfs snapshot tank/data@assimilate-manual-1",
            "zfs list -H -o name tank/data@assimilate-manual-1",
            "zfs destroy tank/data@assimilate-manual-1",
        ]
    );
    assert_eq!(types(&sent(&mut events)), [RunEventType::SnapshotRemoved]);
    assert!(!tokio::fs::try_exists(&record).await.unwrap());
}

#[tokio::test]
async fn an_lvm_snapshot_is_mounted_read_only_below_the_mount_root() {
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let table = MountTable::parse(&format!(
        "1 0 253:1 /sub {} rw - xfs /dev/assimilate-test-vg/data rw\n",
        point.display()
    ));
    for (attr, size) in [
        ("-wi-ao----", "--extents 10%ORIGIN"),
        ("Vwi-aotz--", "--setactivationskip n --activate y"),
    ] {
        let tools_dir = tempfile::tempdir().unwrap();
        let tools = fake::tools(tools_dir.path(), &format!("  vg9:data:{attr}")).await;
        let (request, mut events) =
            request(tools, &[&point], FsSnapshotMode::Required, state.path());
        let mount_dir = state.path().join("mounts/manual-1/vg9-data");

        let snapshots = take_on(request, &table).await.unwrap();
        assert!(snapshots.plan().is_some());
        assert!(tokio::fs::try_exists(&mount_dir).await.unwrap());
        assert_eq!(
            fake::calls(tools_dir.path()).await.get(1..).unwrap(),
            [
                format!("lvcreate --snapshot --name assimilate-data-manual-1 {size} vg9/data"),
                format!(
                    "mount -t xfs -o ro,nouuid,norecovery /dev/vg9/assimilate-data-manual-1 {}",
                    mount_dir.display()
                ),
            ]
        );
        assert_eq!(types(&sent(&mut events)), [RunEventType::SnapshotCreated]);

        // Never really mounted, so it is only removed, as lvs says it exists.
        assert_eq!(snapshots.remove().await.len(), 0);
        assert!(!tokio::fs::try_exists(&mount_dir).await.unwrap());
        assert_eq!(
            fake::calls(tools_dir.path()).await.get(3..).unwrap(),
            [
                "lvs vg9/assimilate-data-manual-1",
                "lvremove --force vg9/assimilate-data-manual-1",
            ]
        );
    }
}

#[tokio::test]
async fn a_btrfs_subvolume_is_snapshotted_read_only_inside_itself() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let tools = fake::tools(tools_dir.path(), "").await;
    let (request, _events) = request(tools, &[], FsSnapshotMode::Required, state.path());
    let mut snapshots = take_on(request.clone(), &MountTable::default())
        .await
        .unwrap();
    assert!(snapshots.plan().is_none());

    let origin = Origin::Btrfs {
        subvolume: point.clone(),
    };
    let from = take_one(&origin, &request, &mut snapshots).await.unwrap();
    assert_eq!(from, point.join(".assimilate-snapshot-manual-1"));
    assert_eq!(
        fake::calls(tools_dir.path()).await,
        [format!(
            "btrfs subvolume snapshot -r {} {}",
            point.display(),
            from.display()
        )]
    );
    assert_eq!(snapshots.remove().await.len(), 0);
}

#[tokio::test]
async fn a_source_that_cannot_be_snapshotted_fails_a_required_run_or_is_read_live() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let mut tools = fake::tools(tools_dir.path(), "").await;
    tools.lvs = fake::script(tools_dir.path(), "lvs", "echo 'not an LV' >&2; exit 5").await;
    let table = MountTable::parse(&format!(
        "1 0 8:1 / {} rw - ext4 /dev/sda1 rw\n",
        point.display()
    ));

    let (required, mut events) = request(
        tools.clone(),
        &[&point],
        FsSnapshotMode::Required,
        state.path(),
    );
    let error = take_on(required, &table).await.unwrap_err();
    assert!(error.contains("cannot be snapshotted"), "{error}");
    assert_eq!(types(&sent(&mut events)), [RunEventType::SnapshotFailed]);

    let (best_effort, mut events) =
        request(tools, &[&point], FsSnapshotMode::BestEffort, state.path());
    let snapshots = take_on(best_effort, &table).await.unwrap();
    assert!(snapshots.plan().is_none());
    let warning = snapshots.warnings().first().unwrap();
    assert!(warning.ends_with("; it was backed up live"), "{warning}");
    assert_eq!(types(&sent(&mut events)), [RunEventType::SnapshotFailed]);
}

/// "If possible" still snapshots the sources it can when another one
/// cannot be, so the directories kept live over those snapshots must exist
/// too, or borg's namespace could not be entered and it would not start.
#[tokio::test]
async fn a_best_effort_run_with_a_live_source_still_keeps_its_live_dirs() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_zfs, snapshotted) = filesystem().await;
    let (_ext4, live) = filesystem().await;
    zfs_snapshot_dir(&snapshotted).await;
    let mut tools = fake::tools(tools_dir.path(), "").await;
    tools.lvs = fake::script(tools_dir.path(), "lvs", "echo 'not an LV' >&2; exit 5").await;
    let table = MountTable::parse(&format!(
        "1 0 0:41 / {} rw - zfs tank/data rw\n2 0 8:1 / {} rw - ext4 /dev/sda1 rw\n",
        snapshotted.display(),
        live.display()
    ));
    let (mut request, mut events) = request(
        tools,
        &[&snapshotted, &live],
        FsSnapshotMode::BestEffort,
        state.path(),
    );
    let cache = snapshotted.join("cache/borg");
    request.keep_live = vec![cache.clone()];

    let snapshots = take_on(request, &table).await.unwrap();
    assert_eq!(snapshots.plan().unwrap().live_count(), 1);
    assert!(tokio::fs::try_exists(&cache).await.unwrap());
    let warning = snapshots.warnings().first().unwrap();
    assert!(warning.contains("cannot be snapshotted"), "{warning}");
    assert_eq!(snapshots.warnings().len(), 1);
    assert_eq!(
        types(&sent(&mut events)),
        [RunEventType::SnapshotCreated, RunEventType::SnapshotFailed]
    );
    assert_eq!(snapshots.remove().await.len(), 0);
}

#[tokio::test]
async fn a_failed_snapshot_undoes_a_required_run_and_is_skipped_otherwise() {
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let inner = point.join("inner");
    tokio::fs::create_dir(&inner).await.unwrap();
    // Only the outer dataset shows its snapshot, so the inner one fails.
    zfs_snapshot_dir(&point).await;
    let table = zfs_table(&[(&point, "tank/a"), (&inner, "tank/a/inner")]);

    let tools_dir = tempfile::tempdir().unwrap();
    let mut tools = fake::tools(tools_dir.path(), "").await;
    tools.zfs = fake::script(tools_dir.path(), "zfs", ZFS_KEEPS).await;
    let (required, mut events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
    let error = take_on(required, &table).await.unwrap_err();
    assert!(
        error.contains("could not be snapshotted: could not open the snapshot"),
        "{error}"
    );
    let calls = fake::calls(tools_dir.path()).await;
    assert!(
        calls
            .iter()
            .any(|call| call.ends_with("destroy tank/a/inner@assimilate-manual-1")),
        "both snapshots are removed again: {calls:?}"
    );
    assert_eq!(
        types(&sent(&mut events)),
        [
            RunEventType::SnapshotCreated,
            RunEventType::SnapshotFailed,
            RunEventType::SnapshotFailed,
            RunEventType::SnapshotFailed,
        ]
    );

    let tools_dir = tempfile::tempdir().unwrap();
    let tools = fake::tools(tools_dir.path(), "").await;
    let (best_effort, _events) =
        request(tools, &[&point], FsSnapshotMode::BestEffort, state.path());
    let snapshots = take_on(best_effort, &table).await.unwrap();
    assert!(snapshots.plan().is_some());
    assert_eq!(snapshots.warnings().len(), 1);
    assert_eq!(snapshots.remove().await.len(), 0);
}

#[tokio::test]
async fn a_snapshot_that_cannot_be_removed_keeps_its_record() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    zfs_snapshot_dir(&point).await;
    let mut tools = fake::tools(tools_dir.path(), "").await;
    tools.zfs = fake::script(tools_dir.path(), "zfs", ZFS_KEEPS).await;
    let (request, mut events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
    let snapshots = take_on(request, &zfs_table(&[(&point, "tank")]))
        .await
        .unwrap();
    let failures = snapshots.remove().await;
    assert_eq!(failures.len(), 1);
    assert!(
        failures
            .first()
            .unwrap()
            .starts_with("could not remove the snapshot of ZFS snapshot tank@"),
        "{failures:?}"
    );
    assert_eq!(
        types(&sent(&mut events)),
        [RunEventType::SnapshotCreated, RunEventType::SnapshotFailed]
    );
    assert!(
        tokio::fs::try_exists(state.path().join("records/manual-1.json"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn nothing_is_snapshotted_that_could_not_be_recorded_first() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let tools = fake::tools(tools_dir.path(), "").await;
    let (mut request, _events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
    request.record_dir = state.path().join("a-file");
    tokio::fs::write(&request.record_dir, "").await.unwrap();
    let error = take_on(request, &zfs_table(&[(&point, "tank")]))
        .await
        .unwrap_err();
    assert!(
        error.contains("could not record the snapshot before taking it"),
        "{error}"
    );
    assert_eq!(fake::calls(tools_dir.path()).await.len(), 0);
}

#[tokio::test]
async fn an_lvm_snapshot_without_a_place_to_mount_is_read_live() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    let tools = fake::tools(tools_dir.path(), "vg9:data:-wi-ao----").await;
    let (mut request, _events) =
        request(tools, &[&point], FsSnapshotMode::BestEffort, state.path());
    request.mount_root = state.path().join("a-file");
    tokio::fs::write(&request.mount_root, "").await.unwrap();
    let table = MountTable::parse(&format!(
        "1 0 253:1 / {} rw - ext4 /dev/vg9/data rw\n",
        point.display()
    ));
    let snapshots = take_on(request, &table).await.unwrap();
    let warning = snapshots.warnings().first().unwrap();
    assert!(warning.contains("could not create"), "{warning}");
    assert!(snapshots.plan().is_none());
    assert_eq!(snapshots.remove().await.len(), 0);
}

#[tokio::test]
async fn a_live_directory_that_cannot_be_named_to_the_kernel_fails_the_run() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    zfs_snapshot_dir(&point).await;
    let tools = fake::tools(tools_dir.path(), "").await;
    let (mut request, _events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
    request.keep_live = vec![point.join("nul\0byte")];
    let error = take_on(request, &zfs_table(&[(&point, "tank")]))
        .await
        .unwrap_err();
    assert!(
        error.starts_with("a snapshot path cannot be mounted"),
        "{error}"
    );
    assert!(
        fake::calls(tools_dir.path())
            .await
            .contains(&"zfs destroy tank@assimilate-manual-1".to_owned()),
        "the snapshot is removed before the run fails"
    );
}

#[tokio::test]
async fn a_dropped_run_removes_its_snapshots_in_the_background() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, point) = filesystem().await;
    zfs_snapshot_dir(&point).await;
    let mut tools = fake::tools(tools_dir.path(), "").await;
    tools.zfs = fake::script(tools_dir.path(), "zfs", ZFS_KEEPS).await;
    let (request, _events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
    let mut snapshots = take_on(request, &zfs_table(&[(&point, "tank")]))
        .await
        .unwrap();
    snapshots.reap_after = std::time::Duration::ZERO;
    drop(snapshots);
    let destroyed = "zfs destroy tank@assimilate-manual-1".to_owned();
    for _ in 0..200 {
        if fake::calls(tools_dir.path()).await.contains(&destroyed) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the dropped run's snapshot was never removed");
}

#[test]
fn a_run_dropped_without_a_runtime_leaves_its_record_for_the_next_start() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (_fs, snapshots) = runtime.block_on(async {
        let (fs, point) = filesystem().await;
        zfs_snapshot_dir(&point).await;
        let tools = fake::tools(tools_dir.path(), "").await;
        let (request, _events) = request(tools, &[&point], FsSnapshotMode::Required, state.path());
        let snapshots = take_on(request, &zfs_table(&[(&point, "tank")]))
            .await
            .unwrap();
        (fs, snapshots)
    });
    drop(snapshots);
    let record = state.path().join("records/manual-1.json");
    assert!(runtime.block_on(tokio::fs::try_exists(record)).unwrap());
}

#[tokio::test]
async fn the_running_host_can_be_asked_for_nothing() {
    let tools_dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let tools = fake::tools(tools_dir.path(), "").await;
    let (request, _events) = request(tools.clone(), &[], FsSnapshotMode::Required, state.path());
    let snapshots = take(request).await.unwrap();
    assert!(snapshots.plan().is_none());
    assert_eq!(snapshots.remove().await.len(), 0);
    assert!(record_dir().ends_with("snapshots"));
    // A record that cannot be read is only logged: the agent starts anyway.
    let records = state.path().join("leftovers");
    tokio::fs::create_dir(&records).await.unwrap();
    tokio::fs::write(records.join("broken.json"), "{")
        .await
        .unwrap();
    remove_leftovers(&records, &tools).await;
    assert!(
        tokio::fs::try_exists(records.join("broken.json"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_step_for_a_timeline_that_is_gone_is_dropped() {
    let (outbound, events) = mpsc::channel(1);
    drop(events);
    let events = RunEvents {
        outbound,
        run_id: "run-1".to_owned(),
        repo_id: RepoId(3),
    };
    events.send(RunEventType::SnapshotCreated, "x").await;
    report(None, RunEventType::SnapshotCreated, "x").await;
}

#[test]
fn borg_writes_to_its_cache_configuration_and_temporary_directories() {
    let env = std::collections::HashMap::from([
        ("HOME", "/home/backup"),
        ("BORG_CACHE_DIR", "/var/cache/borg"),
        ("BORG_KEYS_DIR", "/etc/borg/keys"),
    ]);
    assert_eq!(
        borg_dirs(|variable| env.get(variable).map(Into::into)),
        [
            PathBuf::from("/var/cache/borg"),
            PathBuf::from("/home/backup/.config/borg"),
            std::env::temp_dir(),
            PathBuf::from("/etc/borg/keys"),
        ]
    );
    assert_eq!(
        borg_dirs(|_| None).first().unwrap(),
        Path::new("/root/.cache/borg")
    );
}

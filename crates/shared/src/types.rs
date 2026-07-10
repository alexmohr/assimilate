// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use utoipa::ToSchema;

fn default_keep_hourly() -> u32 {
    24
}

/// Name of the environment variable borg reads the repository URL from.
pub const BORG_REPO_ENV_KEY: &str = "BORG_REPO";

/// Builds the `borg://`-style SSH repository URL (`ssh://user@host:port/path`)
/// passed to `borg` via [`BORG_REPO_ENV_KEY`], stripping any leading slash
/// from `repo_path` so it is not duplicated after the port.
#[must_use]
pub fn build_repo_url(ssh_user: &str, ssh_host: &str, ssh_port: u16, repo_path: &str) -> String {
    format!(
        "ssh://{ssh_user}@{ssh_host}:{ssh_port}/{}",
        repo_path.trim_start_matches('/')
    )
}

/// Newtype wrapper around the database primary key of an [`Agent`], used to
/// avoid mixing up agent, repo, and report identifiers at the type level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentId(
    /// The underlying database row id.
    #[ts(type = "number")]
    pub i64,
);

/// Newtype wrapper around the database primary key of a [`Repo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RepoId(
    /// The underlying database row id.
    #[ts(type = "number")]
    pub i64,
);

/// Newtype wrapper around the database primary key of a [`BackupReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReportId(
    /// The underlying database row id.
    #[ts(type = "number")]
    pub i64,
);

/// Identifier for a long-running backup/check/verify operation, used to
/// correlate progress updates and cancellation requests with the operation
/// they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(
    /// The opaque operation identifier string.
    pub String,
);

impl From<String> for OperationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// A single file or directory entry matched while searching within one or
/// more borg archives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SearchEntry {
    /// Path of the entry within the archive.
    pub path: String,
    /// Size of the entry in bytes, as reported by borg.
    #[ts(type = "number")]
    pub size: i64,
    /// Last modification time of the entry recorded in the archive.
    pub mtime: DateTime<Utc>,
    /// Kind of filesystem entry (e.g. `"file"`, `"directory"`, `"symlink"`)
    /// as reported by borg.
    pub entry_type: String,
    /// Name of the archive the entry was found in, when searching across
    /// multiple archives; `None` if the search was scoped to a single one.
    pub archive_name: Option<String>,
}

/// A file that would be included in a backup, as reported by a `borg create
/// --dry-run` preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunFile {
    /// Path of the file relative to the backup source root.
    pub path: String,
    /// Size of the file in bytes.
    pub size: i64,
}

impl From<i64> for AgentId {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<i64> for RepoId {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<i64> for ReportId {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl FromStr for ScheduleType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "backup" => Ok(ScheduleType::Backup),
            "check" => Ok(ScheduleType::Check),
            "verify" => Ok(ScheduleType::Verify),
            other => Err(format!("unknown schedule type: {other}")),
        }
    }
}

impl fmt::Display for ScheduleType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleType::Backup => write!(f, "backup"),
            ScheduleType::Check => write!(f, "check"),
            ScheduleType::Verify => write!(f, "verify"),
        }
    }
}

/// Compression algorithm applied by borg when writing new archive chunks to
/// a repository.
#[derive(Debug, Clone, Default, PartialEq, Eq, TS, ToSchema)]
pub enum Compression {
    /// Store chunks uncompressed.
    None,
    /// Compress with LZ4, a fast algorithm with modest compression ratio.
    #[default]
    Lz4,
    /// Compress with Zstandard at the given level (higher is slower but
    /// smaller).
    Zstd {
        /// Zstandard compression level.
        level: i32,
    },
    /// Compress with zlib/deflate at the given level (higher is slower but
    /// smaller).
    Zlib {
        /// Zlib compression level.
        level: i32,
    },
}

impl Serialize for Compression {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Compression {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Accept both the new flat-string format ("zstd,3") and the old
        // tagged-object format ({"type":"Zstd","value":{"level":3}}) for
        // backward compatibility during mixed-version server/agent upgrades.
        match Value::deserialize(deserializer)? {
            Value::String(s) => s.parse().map_err(serde::de::Error::custom),
            Value::Object(ref map) => {
                let type_ = map
                    .get("type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| serde::de::Error::custom("missing 'type' in compression"))?;
                let level = map
                    .get("value")
                    .and_then(|v| v.get("level"))
                    .and_then(Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok());
                match type_ {
                    "None" => Ok(Compression::None),
                    "Lz4" => Ok(Compression::Lz4),
                    "Zstd" => Ok(Compression::Zstd {
                        level: level.unwrap_or(DEFAULT_ZSTD_LEVEL),
                    }),
                    "Zlib" => Ok(Compression::Zlib {
                        level: level.unwrap_or(DEFAULT_ZLIB_LEVEL),
                    }),
                    other => Err(serde::de::Error::custom(format!(
                        "unknown compression type: {other}"
                    ))),
                }
            }
            _ => Err(serde::de::Error::custom("invalid compression format")),
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Compression::None => write!(f, "none"),
            Compression::Lz4 => write!(f, "lz4"),
            Compression::Zstd { level } => write!(f, "zstd,{level}"),
            Compression::Zlib { level } => write!(f, "zlib,{level}"),
        }
    }
}

/// Default zstd level used when a bare "zstd" (no explicit level) is parsed.
const DEFAULT_ZSTD_LEVEL: i32 = 3;
/// Default zlib level used when a bare "zlib" (no explicit level) is parsed.
const DEFAULT_ZLIB_LEVEL: i32 = 6;

impl FromStr for Compression {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Compression::None),
            "lz4" => Ok(Compression::Lz4),
            "zstd" => Ok(Compression::Zstd {
                level: DEFAULT_ZSTD_LEVEL,
            }),
            "zlib" => Ok(Compression::Zlib {
                level: DEFAULT_ZLIB_LEVEL,
            }),
            other => {
                if let Some(level_str) = other.strip_prefix("zstd,") {
                    let level = level_str
                        .parse::<i32>()
                        .map_err(|_| format!("invalid zstd level: {level_str}"))?;
                    Ok(Compression::Zstd { level })
                } else if let Some(level_str) = other.strip_prefix("zlib,") {
                    let level = level_str
                        .parse::<i32>()
                        .map_err(|_| format!("invalid zlib level: {level_str}"))?;
                    Ok(Compression::Zlib { level })
                } else {
                    Err(format!("unknown compression: {other}"))
                }
            }
        }
    }
}

/// Encryption mode used when creating a new borg repository, passed to
/// `borg init --encryption`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
pub enum BorgEncryption {
    /// AES encryption with the key stored inside the repository, protected
    /// by the repository passphrase.
    #[default]
    #[serde(rename = "repokey")]
    Repokey,
    /// Like [`Repokey`](Self::Repokey), but using BLAKE2 for MAC/HMAC
    /// instead of HMAC-SHA256.
    #[serde(rename = "repokey-blake2")]
    RepokeyBlake2,
    /// AES encryption with the key stored in a local keyfile rather than in
    /// the repository itself.
    #[serde(rename = "keyfile")]
    Keyfile,
    /// Like [`Keyfile`](Self::Keyfile), but using BLAKE2 for MAC/HMAC
    /// instead of HMAC-SHA256.
    #[serde(rename = "keyfile-blake2")]
    KeyfileBlake2,
    /// No encryption, but archives are authenticated to detect tampering.
    #[serde(rename = "authenticated")]
    Authenticated,
    /// Like [`Authenticated`](Self::Authenticated), but using BLAKE2 instead
    /// of HMAC-SHA256.
    #[serde(rename = "authenticated-blake2")]
    AuthenticatedBlake2,
    /// No encryption and no authentication.
    #[serde(rename = "none")]
    None,
}

impl BorgEncryption {
    /// Returns the exact string borg expects as the `--encryption` argument
    /// value for this mode.
    #[must_use]
    pub fn as_borg_arg(self) -> &'static str {
        match self {
            Self::Repokey => "repokey",
            Self::RepokeyBlake2 => "repokey-blake2",
            Self::Keyfile => "keyfile",
            Self::KeyfileBlake2 => "keyfile-blake2",
            Self::Authenticated => "authenticated",
            Self::AuthenticatedBlake2 => "authenticated-blake2",
            Self::None => "none",
        }
    }
}

impl std::fmt::Display for BorgEncryption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_borg_arg())
    }
}

impl FromStr for BorgEncryption {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "repokey" => Ok(Self::Repokey),
            "repokey-blake2" => Ok(Self::RepokeyBlake2),
            "keyfile" => Ok(Self::Keyfile),
            "keyfile-blake2" => Ok(Self::KeyfileBlake2),
            "authenticated" => Ok(Self::Authenticated),
            "authenticated-blake2" => Ok(Self::AuthenticatedBlake2),
            "none" => Ok(Self::None),
            other => Err(format!("unknown borg encryption mode: {other}")),
        }
    }
}

/// Kind of borg operation a [`Schedule`] runs on its cron trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleType {
    /// Runs `borg create` to take a new backup of the configured sources.
    #[default]
    Backup,
    /// Runs `borg check` to verify repository and archive integrity.
    Check,
    /// Runs `borg extract --dry-run` (or similar) to verify archives can be
    /// restored without actually writing files.
    Verify,
}

/// How multiple schedules on the same repository are executed relative to
/// each other. Currently only sequential execution is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Schedules run one at a time, never overlapping.
    #[default]
    #[serde(alias = "parallel")]
    Sequential,
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sequential => write!(f, "sequential"),
        }
    }
}

impl FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sequential" | "parallel" => Ok(Self::Sequential),
            other => Err(format!("unknown execution mode: {other}")),
        }
    }
}

/// What a sequential schedule's remaining targets should do when one target's backup fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OnFailure {
    /// Abort the remaining targets in the sequence once one fails.
    #[default]
    Stop,
    /// Keep running the remaining targets even though one failed.
    Continue,
}

impl fmt::Display for OnFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stop => write!(f, "stop"),
            Self::Continue => write!(f, "continue"),
        }
    }
}

impl FromStr for OnFailure {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stop" => Ok(Self::Stop),
            "continue" => Ok(Self::Continue),
            other => Err(format!("unknown on_failure value: {other}")),
        }
    }
}

/// Action taken when a repo or server quota threshold (warn/critical) is breached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaAction {
    /// Send a notification but otherwise let backups proceed as normal.
    #[default]
    NotifyOnly,
    /// Refuse to start new backups until the repo/server is back under quota.
    BlockBackups,
    /// Disable the affected schedule(s) outright until re-enabled manually.
    DisableSchedule,
}

impl fmt::Display for QuotaAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotifyOnly => write!(f, "notify_only"),
            Self::BlockBackups => write!(f, "block_backups"),
            Self::DisableSchedule => write!(f, "disable_schedule"),
        }
    }
}

impl FromStr for QuotaAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "notify_only" => Ok(Self::NotifyOnly),
            "block_backups" => Ok(Self::BlockBackups),
            "disable_schedule" => Ok(Self::DisableSchedule),
            other => Err(format!("unknown quota action: {other}")),
        }
    }
}

/// Overall outcome of a single backup run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum BackupStatus {
    /// The backup completed with no errors or warnings.
    #[default]
    #[serde(alias = "Success")]
    Success,
    /// The backup completed but borg reported at least one warning.
    #[serde(alias = "Warning")]
    Warning,
    /// The backup did not complete successfully.
    #[serde(alias = "Failed")]
    Failed,
}

impl fmt::Display for BackupStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success => write!(f, "success"),
            Self::Warning => write!(f, "warning"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for BackupStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" | "Success" => Ok(Self::Success),
            "warning" | "Warning" => Ok(Self::Warning),
            "failed" | "Failed" => Ok(Self::Failed),
            other => Err(format!("unknown backup status: {other}")),
        }
    }
}

/// Visibility scope of a repository, agent, or schedule — controls whether
/// the resource is visible only to its owner or shared with all users that
/// share a group with the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TS, ToSchema, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "lowercase")]
pub enum Visibility {
    /// Visible only to the owner and users sharing a group with the owner.
    #[default]
    Private,
    /// Visible to all users.
    Shared,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Private => write!(f, "private"),
            Self::Shared => write!(f, "shared"),
        }
    }
}

impl FromStr for Visibility {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "private" => Ok(Self::Private),
            "shared" => Ok(Self::Shared),
            other => Err(format!("unknown visibility: {other}")),
        }
    }
}

impl From<String> for Visibility {
    fn from(s: String) -> Self {
        s.parse().unwrap_or_default()
    }
}

impl From<Visibility> for String {
    fn from(v: Visibility) -> Self {
        v.to_string()
    }
}

impl Serialize for Visibility {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Visibility {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Well-known system event types recorded in the `system_events` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS, ToSchema, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[sqlx(rename_all = "snake_case")]
pub enum SystemEventType {
    /// An agent authentication attempt failed.
    AuthFailed,
    /// A periodic repository sync completed.
    RepoSync,
    /// A periodic repository sync took longer than the warning threshold.
    RepoSyncSlow,
    /// A periodic repository sync failed.
    RepoSyncFailed,
    /// An archive deletion operation failed.
    ArchiveDeleteFailed,
    /// A security-related violation was detected.
    SecurityViolation,
}

impl std::fmt::Display for SystemEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "auth_failed"),
            Self::RepoSync => write!(f, "repo_sync"),
            Self::RepoSyncSlow => write!(f, "repo_sync_slow"),
            Self::RepoSyncFailed => write!(f, "repo_sync_failed"),
            Self::ArchiveDeleteFailed => write!(f, "archive_delete_failed"),
            Self::SecurityViolation => write!(f, "security_violation"),
        }
    }
}

impl FromStr for SystemEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auth_failed" => Ok(Self::AuthFailed),
            "repo_sync" => Ok(Self::RepoSync),
            "repo_sync_slow" => Ok(Self::RepoSyncSlow),
            "repo_sync_failed" => Ok(Self::RepoSyncFailed),
            "archive_delete_failed" => Ok(Self::ArchiveDeleteFailed),
            "security_violation" => Ok(Self::SecurityViolation),
            other => Err(format!("unknown system event type: {other}")),
        }
    }
}

impl Serialize for SystemEventType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SystemEventType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Severity level of a dashboard finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    /// A critical issue requiring immediate attention.
    #[default]
    Critical,
    /// A warning that should be reviewed.
    Warning,
    /// Informational finding.
    Info,
}

impl std::fmt::Display for FindingSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
        }
    }
}

impl FromStr for FindingSeverity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "critical" => Ok(Self::Critical),
            "warning" => Ok(Self::Warning),
            "info" => Ok(Self::Info),
            other => Err(format!("unknown finding severity: {other}")),
        }
    }
}

/// Status value of a dashboard finding or operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// The operation is currently running.
    Running,
    /// The finding is a warning (non-fatal but noteworthy).
    Warning,
    /// The finding indicates a failure.
    Failed,
    /// A scheduled task is overdue.
    Overdue,
    /// A schedule target has never succeeded.
    NeverSucceeded,
    /// A host is offline and a schedule is due soon.
    OfflineDueSoon,
}

impl std::fmt::Display for FindingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Warning => write!(f, "warning"),
            Self::Failed => write!(f, "failed"),
            Self::Overdue => write!(f, "overdue"),
            Self::NeverSucceeded => write!(f, "never_succeeded"),
            Self::OfflineDueSoon => write!(f, "offline_due_soon"),
        }
    }
}

impl FromStr for FindingStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(Self::Running),
            "warning" => Ok(Self::Warning),
            "failed" => Ok(Self::Failed),
            "overdue" => Ok(Self::Overdue),
            "never_succeeded" => Ok(Self::NeverSucceeded),
            "offline_due_soon" => Ok(Self::OfflineDueSoon),
            other => Err(format!("unknown finding status: {other}")),
        }
    }
}

/// Kind/category of a dashboard finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// Agent host has no backup schedule assigned.
    HostUnassigned,
    /// Repository has no enabled backup schedule.
    RepositoryUnscheduled,
    /// Repository storage is at or above its critical quota.
    RepositoryQuotaCritical,
    /// Repository storage is at or above its warning quota.
    RepositoryQuotaWarning,
    /// Import of a repository failed.
    RepositoryImportFailed,
    /// The latest backup failed.
    BackupFailed,
    /// Backup completed with warnings.
    BackupWarning,
    /// A schedule target's backup is overdue.
    ScheduleTargetOverdue,
    /// A schedule target has never succeeded.
    ScheduleTargetNeverSucceeded,
    /// A host is offline and a schedule is due soon.
    HostOfflineDueSoon,
}

impl std::fmt::Display for FindingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HostUnassigned => write!(f, "host_unassigned"),
            Self::RepositoryUnscheduled => write!(f, "repository_unscheduled"),
            Self::RepositoryQuotaCritical => write!(f, "repository_quota_critical"),
            Self::RepositoryQuotaWarning => write!(f, "repository_quota_warning"),
            Self::RepositoryImportFailed => write!(f, "repository_import_failed"),
            Self::BackupFailed => write!(f, "backup_failed"),
            Self::BackupWarning => write!(f, "backup_warning"),
            Self::ScheduleTargetOverdue => write!(f, "schedule_target_overdue"),
            Self::ScheduleTargetNeverSucceeded => write!(f, "schedule_target_never_succeeded"),
            Self::HostOfflineDueSoon => write!(f, "host_offline_due_soon"),
        }
    }
}

impl FromStr for FindingKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "host_unassigned" => Ok(Self::HostUnassigned),
            "repository_unscheduled" => Ok(Self::RepositoryUnscheduled),
            "repository_quota_critical" => Ok(Self::RepositoryQuotaCritical),
            "repository_quota_warning" => Ok(Self::RepositoryQuotaWarning),
            "repository_import_failed" => Ok(Self::RepositoryImportFailed),
            "backup_failed" => Ok(Self::BackupFailed),
            "backup_warning" => Ok(Self::BackupWarning),
            "schedule_target_overdue" => Ok(Self::ScheduleTargetOverdue),
            "schedule_target_never_succeeded" => Ok(Self::ScheduleTargetNeverSucceeded),
            "host_offline_due_soon" => Ok(Self::HostOfflineDueSoon),
            other => Err(format!("unknown finding kind: {other}")),
        }
    }
}

/// Status of an archive indexing job in the content-search index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum IndexStatus {
    /// Index job not yet started.
    #[default]
    Pending,
    /// Indexing is in progress.
    Indexing,
    /// Indexing completed successfully.
    Done,
    /// Indexing failed.
    Failed,
}

impl std::fmt::Display for IndexStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Indexing => write!(f, "indexing"),
            Self::Done => write!(f, "done"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for IndexStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "indexing" => Ok(Self::Indexing),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown index status: {other}")),
        }
    }
}

/// Whether a registered agent currently has a live connection to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// The agent has an active WebSocket connection to the server.
    Online,
    /// The agent is not currently connected.
    Offline,
}

/// A registered backup agent (a machine running the `assimilate-agent` binary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique database identifier for this agent.
    pub id: AgentId,
    /// The agent's machine hostname, used to match it to schedules and archives.
    pub hostname: String,
    /// Optional human-friendly name shown in the UI in place of the hostname.
    pub display_name: Option<String>,
    /// Bcrypt hash of the agent's authentication token; the plaintext token is never stored.
    pub agent_token_hash: String,
    /// When this agent was first registered with the server.
    pub created_at: DateTime<Utc>,
    /// When the agent last connected or sent a heartbeat, if ever.
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// A configured borg repository that agents back up into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    /// Unique database identifier for this repository.
    pub id: RepoId,
    /// Human-friendly name used to refer to the repository in the UI and config.
    pub name: String,
    /// Filesystem path to the repository on the remote borg host.
    pub repo_path: String,
    /// SSH user used to reach the borg host.
    pub ssh_user: String,
    /// Hostname or IP address of the borg host.
    pub ssh_host: String,
    /// SSH port used to reach the borg host.
    pub ssh_port: u16,
    /// The repository passphrase, encrypted at rest with the server's encryption key.
    pub passphrase_encrypted: Vec<u8>,
    /// Compression algorithm/level borg uses when writing to this repository.
    pub compression: Compression,
    /// Whether backups and schedules for this repository are currently active.
    pub enabled: bool,
}

/// A single filesystem path an agent backs up into a given repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSource {
    /// Repository this source path is backed up into.
    pub repo_id: RepoId,
    /// Absolute filesystem path on the agent to include in the backup.
    pub path: String,
}

/// A recurring backup schedule for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// Repository this schedule backs up into.
    pub repo_id: RepoId,
    /// How often the schedule runs (hourly/daily/weekly/custom).
    pub schedule_type: ScheduleType,
    /// Cron expression controlling exactly when the schedule fires.
    pub cron_expression: String,
    /// Whether the schedule is currently active.
    pub enabled: bool,
    /// When this schedule last ran, if ever.
    pub last_run_at: Option<DateTime<Utc>>,
    /// When this schedule is next due to run, if known.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Number of yearly archives to retain when pruning.
    pub keep_yearly: u32,
}

/// A record of a single completed (or failed) backup run, persisted for
/// history, stats, and quota tracking.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BackupReport {
    /// Unique database identifier for this report.
    pub id: ReportId,
    /// Agent that performed the backup this report describes.
    pub agent_id: AgentId,
    /// Repository the backup was written into.
    pub repo_id: RepoId,
    /// Schedule that triggered this backup, if it wasn't run manually.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    /// When the backup began.
    pub started_at: DateTime<Utc>,
    /// When the backup finished (successfully or not).
    pub finished_at: DateTime<Utc>,
    /// Overall outcome of the backup.
    pub status: BackupStatus,
    /// Total uncompressed size of the files borg processed, in bytes.
    #[ts(type = "number")]
    pub original_size: i64,
    /// Total compressed size borg wrote for this backup, in bytes.
    #[ts(type = "number")]
    pub compressed_size: i64,
    /// Size after deduplication against existing chunks in the repository, in bytes.
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    /// Total unique compressed size of the repository at backup time (`cache.stats.unique_csize`).
    /// This is the actual on-disk usage of the repository.
    #[serde(default)]
    #[ts(type = "number")]
    pub repo_unique_csize: i64,
    /// Number of files borg processed during this backup.
    #[ts(type = "number")]
    pub files_processed: i64,
    /// How long the backup took to run, in seconds.
    #[ts(type = "number")]
    pub duration_secs: i64,
    /// Human-readable error message, present when the backup failed.
    pub error_message: Option<String>,
    /// Warning messages borg emitted during the backup, if any.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Version string reported by the agent's `borg --version`, if known.
    pub borg_version: Option<String>,
    /// Name of the archive borg created for this backup, if it got that far.
    #[serde(default)]
    pub archive_name: Option<String>,
    /// The exact borg command line that was executed, for troubleshooting.
    #[serde(default)]
    pub borg_command: Option<String>,
    /// Correlation ID linking this report to related log lines and events.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// The full backup configuration an agent receives from the server, describing
/// every repository and schedule it is responsible for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Hostname this configuration was assembled for.
    pub agent_hostname: String,
    /// Target names to skip when running schedules, e.g. after a manual override.
    #[serde(default)]
    pub skip_targets: Vec<String>,
    /// Repositories (and their schedules) this agent should back up into.
    pub repos: Vec<RepoConfig>,
}

/// A single repository's configuration as delivered to the agent, including
/// connection details and its schedules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    /// Unique database identifier for this repository.
    pub repo_id: RepoId,
    /// Human-friendly name used to refer to the repository in the UI and config.
    pub name: String,
    /// Filesystem path to the repository on the remote borg host.
    pub repo_path: String,
    /// SSH user used to reach the borg host.
    pub ssh_user: String,
    /// Hostname or IP address of the borg host.
    pub ssh_host: String,
    /// SSH port used to reach the borg host.
    pub ssh_port: u16,
    /// Expected SSH host key for the borg host, used to pin/verify the connection.
    #[serde(default)]
    pub ssh_host_key: String,
    /// The repository passphrase in plaintext, decrypted for agent use.
    pub passphrase: String,
    /// Compression algorithm/level borg uses when writing to this repository.
    pub compression: Compression,
    /// Whether backups and schedules for this repository are currently active.
    pub enabled: bool,
    /// Whether the agent may accept a relocated repository (moved path/URL) without failing.
    #[serde(default)]
    pub accept_relocation: bool,
    /// Backup schedules configured for this repository.
    pub schedules: Vec<ScheduleConfig>,
}

/// What to do when a backup source's set of file changes looks unusually
/// large or small compared to prior runs (a possible ransomware/corruption signal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS, ToSchema)]
pub enum FileChangeAction {
    /// Take no action; let the backup proceed regardless of the change volume.
    Ignore,
    /// Log a warning but let the backup proceed.
    #[default]
    Warn,
    /// Abort the backup rather than let it complete.
    Fatal,
}

impl fmt::Display for FileChangeAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ignore => write!(f, "ignore"),
            Self::Warn => write!(f, "warn"),
            Self::Fatal => write!(f, "fatal"),
        }
    }
}

impl FromStr for FileChangeAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ignore" => Ok(Self::Ignore),
            "warn" => Ok(Self::Warn),
            "fatal" => Ok(Self::Fatal),
            other => Err(format!("unknown file change action: {other}")),
        }
    }
}

/// A glob pattern paired with the action to take when a changed file matches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
pub struct FileChangePattern {
    /// Glob pattern matched against changed file paths within the backup source.
    pub path: String,
    /// What to do when a change matching this pattern is detected.
    pub action: FileChangeAction,
}

/// A single schedule's configuration as delivered to the agent, including
/// retention policy, rate limiting, and pre/post-backup hooks.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent per-schedule toggles mirroring the API/DB contract, not \
              mutually-exclusive states"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    /// Unique database identifier for this schedule.
    #[serde(default)]
    pub id: i64,
    /// How often the schedule runs (hourly/daily/weekly/custom).
    pub schedule_type: ScheduleType,
    /// Cron expression controlling exactly when the schedule fires.
    pub cron_expression: String,
    /// Whether the schedule is currently active.
    pub enabled: bool,
    /// Backup source paths this schedule covers; falls back to the repo's defaults when empty.
    #[serde(default)]
    pub backup_sources: Vec<String>,
    /// Maximum upload rate for this schedule's backups, in KiB/s; unlimited when unset.
    #[serde(default)]
    pub rate_limit_kbps: Option<u32>,
    /// Whether canary (test-restore) verification runs after this schedule's backups.
    #[serde(default)]
    pub canary_enabled: bool,
    /// Glob patterns excluded from this schedule's backups, in addition to global excludes.
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    /// Whether to skip the server's global exclude patterns for this schedule.
    #[serde(default)]
    pub ignore_global_excludes: bool,
    /// Number of hourly archives to retain when pruning.
    #[serde(default = "default_keep_hourly")]
    pub keep_hourly: u32,
    /// Number of daily archives to retain when pruning.
    pub keep_daily: u32,
    /// Number of weekly archives to retain when pruning.
    pub keep_weekly: u32,
    /// Number of monthly archives to retain when pruning.
    pub keep_monthly: u32,
    /// Number of yearly archives to retain when pruning.
    pub keep_yearly: u32,
    /// Whether to run `borg compact` after pruning to reclaim freed space.
    pub compact_enabled: bool,
    /// Shell commands to run on the agent before the backup starts.
    #[serde(default)]
    pub pre_backup_commands: Vec<String>,
    /// Shell commands to run on the agent after the backup finishes.
    #[serde(default)]
    pub post_backup_commands: Vec<String>,
    /// Patterns used to flag unusually large file-change volumes as a possible incident.
    #[serde(default)]
    pub file_change_patterns: Vec<FileChangePattern>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_display_roundtrip() {
        assert_eq!(ExecutionMode::Sequential.to_string(), "sequential");
        assert_eq!(
            "sequential".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Sequential
        );
        // "parallel" maps to Sequential for backward compatibility
        assert_eq!(
            "parallel".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Sequential
        );
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn execution_mode_default_is_sequential() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::Sequential);
    }

    #[test]
    fn on_failure_display_roundtrip() {
        assert_eq!(OnFailure::Stop.to_string(), "stop");
        assert_eq!(OnFailure::Continue.to_string(), "continue");
        assert_eq!("stop".parse::<OnFailure>().unwrap(), OnFailure::Stop);
        assert_eq!(
            "continue".parse::<OnFailure>().unwrap(),
            OnFailure::Continue
        );
        assert!("invalid".parse::<OnFailure>().is_err());
    }

    #[test]
    fn on_failure_default_is_stop() {
        assert_eq!(OnFailure::default(), OnFailure::Stop);
    }

    #[test]
    fn borg_encryption_display_roundtrip() {
        let variants = [
            (BorgEncryption::Repokey, "repokey"),
            (BorgEncryption::RepokeyBlake2, "repokey-blake2"),
            (BorgEncryption::Keyfile, "keyfile"),
            (BorgEncryption::KeyfileBlake2, "keyfile-blake2"),
            (BorgEncryption::Authenticated, "authenticated"),
            (BorgEncryption::AuthenticatedBlake2, "authenticated-blake2"),
            (BorgEncryption::None, "none"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            assert_eq!(expected.parse::<BorgEncryption>().unwrap(), variant);
        }
        assert!("unknown".parse::<BorgEncryption>().is_err());
    }

    #[test]
    fn compression_display_roundtrip() {
        assert_eq!(Compression::None.to_string(), "none");
        assert_eq!(Compression::Lz4.to_string(), "lz4");
        assert_eq!("none".parse::<Compression>().unwrap(), Compression::None);
        assert_eq!("lz4".parse::<Compression>().unwrap(), Compression::Lz4);
        assert_eq!(
            "zstd,3".parse::<Compression>().unwrap(),
            Compression::Zstd { level: 3 }
        );
        assert_eq!(
            "zlib,6".parse::<Compression>().unwrap(),
            Compression::Zlib { level: 6 }
        );
        assert!("bad".parse::<Compression>().is_err());
    }

    #[test]
    fn schedule_type_serde_roundtrip() {
        let json = serde_json::to_string(&ScheduleType::Backup).unwrap();
        assert_eq!(json, "\"backup\"");
        let parsed: ScheduleType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ScheduleType::Backup);

        let json = serde_json::to_string(&ScheduleType::Check).unwrap();
        assert_eq!(json, "\"check\"");

        let json = serde_json::to_string(&ScheduleType::Verify).unwrap();
        assert_eq!(json, "\"verify\"");
    }

    #[test]
    fn execution_mode_serde_roundtrip() {
        let json = serde_json::to_string(&ExecutionMode::Sequential).unwrap();
        assert_eq!(json, "\"sequential\"");
        let parsed: ExecutionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ExecutionMode::Sequential);
    }

    #[test]
    fn on_failure_serde_roundtrip() {
        let json = serde_json::to_string(&OnFailure::Stop).unwrap();
        assert_eq!(json, "\"stop\"");
        let parsed: OnFailure = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, OnFailure::Stop);

        let json = serde_json::to_string(&OnFailure::Continue).unwrap();
        assert_eq!(json, "\"continue\"");
    }

    #[test]
    fn backup_report_archive_name_field_is_optional() {
        #[derive(Debug, Deserialize)]
        struct Partial {
            #[serde(default)]
            archive_name: Option<String>,
        }
        let json = r"{}";
        let p: Partial = serde_json::from_str(json).unwrap();
        assert_eq!(p.archive_name, None);

        let json = r#"{"archive_name": "test-2026"}"#;
        let p: Partial = serde_json::from_str(json).unwrap();
        assert_eq!(p.archive_name.as_deref(), Some("test-2026"));
    }

    #[test]
    fn repo_config_accept_relocation_defaults_to_false() {
        #[derive(Debug, Deserialize)]
        struct Partial {
            #[serde(default)]
            accept_relocation: bool,
        }
        let json = r"{}";
        let p: Partial = serde_json::from_str(json).unwrap();
        assert!(!p.accept_relocation);
    }

    #[test]
    fn build_repo_url_strips_leading_slash() {
        assert_eq!(
            build_repo_url("root", "host.example.com", 22, "/mnt/backup/borg"),
            "ssh://root@host.example.com:22/mnt/backup/borg"
        );
    }

    #[test]
    fn build_repo_url_no_leading_slash() {
        assert_eq!(
            build_repo_url("borg", "host.example.com", 2222, "mnt/backup/borg"),
            "ssh://borg@host.example.com:2222/mnt/backup/borg"
        );
    }

    #[test]
    fn visibility_display_roundtrip() {
        assert_eq!(Visibility::Private.to_string(), "private");
        assert_eq!(Visibility::Shared.to_string(), "shared");
        assert_eq!("private".parse::<Visibility>().unwrap(), Visibility::Private);
        assert_eq!("shared".parse::<Visibility>().unwrap(), Visibility::Shared);
        assert!("invalid".parse::<Visibility>().is_err());
    }

    #[test]
    fn visibility_default_is_private() {
        assert_eq!(Visibility::default(), Visibility::Private);
    }

    #[test]
    fn system_event_type_display_roundtrip() {
        let variants = [
            (SystemEventType::AuthFailed, "auth_failed"),
            (SystemEventType::RepoSync, "repo_sync"),
            (SystemEventType::RepoSyncSlow, "repo_sync_slow"),
            (SystemEventType::RepoSyncFailed, "repo_sync_failed"),
            (SystemEventType::ArchiveDeleteFailed, "archive_delete_failed"),
            (SystemEventType::SecurityViolation, "security_violation"),
        ];
        for (variant, expected) in variants {
            assert_eq!(variant.to_string(), expected);
            assert_eq!(expected.parse::<SystemEventType>().unwrap(), variant);
        }
        assert!("unknown".parse::<SystemEventType>().is_err());
    }

    #[test]
    fn finding_severity_display_roundtrip() {
        assert_eq!(FindingSeverity::Critical.to_string(), "critical");
        assert_eq!(FindingSeverity::Warning.to_string(), "warning");
        assert_eq!(FindingSeverity::Info.to_string(), "info");
        assert_eq!("critical".parse::<FindingSeverity>().unwrap(), FindingSeverity::Critical);
        assert_eq!("warning".parse::<FindingSeverity>().unwrap(), FindingSeverity::Warning);
        assert_eq!("info".parse::<FindingSeverity>().unwrap(), FindingSeverity::Info);
        assert!("bogus".parse::<FindingSeverity>().is_err());
    }

    #[test]
    fn finding_status_display_roundtrip() {
        assert_eq!(FindingStatus::Running.to_string(), "running");
        assert_eq!(FindingStatus::Warning.to_string(), "warning");
        assert_eq!(FindingStatus::Failed.to_string(), "failed");
        assert_eq!(FindingStatus::Overdue.to_string(), "overdue");
        assert_eq!("running".parse::<FindingStatus>().unwrap(), FindingStatus::Running);
        assert_eq!("warning".parse::<FindingStatus>().unwrap(), FindingStatus::Warning);
        assert!("bogus".parse::<FindingStatus>().is_err());
    }

    #[test]
    fn finding_kind_display_roundtrip() {
        assert_eq!(FindingKind::HostUnassigned.to_string(), "host_unassigned");
        assert_eq!(FindingKind::RepositoryUnscheduled.to_string(), "repository_unscheduled");
        assert_eq!("backup_failed".parse::<FindingKind>().unwrap(), FindingKind::BackupFailed);
        assert!("bogus".parse::<FindingKind>().is_err());
    }

    #[test]
    fn index_status_display_roundtrip() {
        assert_eq!(IndexStatus::Pending.to_string(), "pending");
        assert_eq!(IndexStatus::Indexing.to_string(), "indexing");
        assert_eq!(IndexStatus::Done.to_string(), "done");
        assert_eq!(IndexStatus::Failed.to_string(), "failed");
        assert_eq!("done".parse::<IndexStatus>().unwrap(), IndexStatus::Done);
        assert!("bogus".parse::<IndexStatus>().is_err());
    }

    #[test]
    fn index_status_default_is_pending() {
        assert_eq!(IndexStatus::default(), IndexStatus::Pending);
    }
}

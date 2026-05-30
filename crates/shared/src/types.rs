// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReportId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub String);

impl From<String> for OperationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEntry {
    pub path: String,
    pub size: i64,
    pub mtime: DateTime<Utc>,
    pub entry_type: String,
    pub archive_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunFile {
    pub path: String,
    pub size: i64,
}

impl From<i64> for ClientId {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Compression {
    None,
    Lz4,
    Zstd { level: i32 },
    Zlib { level: i32 },
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

impl FromStr for Compression {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Compression::None),
            "lz4" => Ok(Compression::Lz4),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BorgEncryption {
    #[serde(rename = "repokey")]
    Repokey,
    #[serde(rename = "repokey-blake2")]
    RepokeyBlake2,
    #[serde(rename = "keyfile")]
    Keyfile,
    #[serde(rename = "keyfile-blake2")]
    KeyfileBlake2,
    #[serde(rename = "authenticated")]
    Authenticated,
    #[serde(rename = "authenticated-blake2")]
    AuthenticatedBlake2,
    #[serde(rename = "none")]
    None,
}

impl BorgEncryption {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleType {
    #[default]
    Backup,
    Check,
    Verify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupStatus {
    Success,
    Warning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: ClientId,
    pub hostname: String,
    pub display_name: Option<String>,
    pub agent_token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub id: RepoId,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub passphrase_encrypted: Vec<u8>,
    pub compression: Compression,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSource {
    pub repo_id: RepoId,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub repo_id: RepoId,
    pub schedule_type: ScheduleType,
    pub cron_expression: String,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub keep_yearly: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupReport {
    pub id: ReportId,
    pub client_id: ClientId,
    pub repo_id: RepoId,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: BackupStatus,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub error_message: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub borg_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub client_hostname: String,
    #[serde(default)]
    pub skip_targets: Vec<String>,
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub repo_id: RepoId,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub passphrase: String,
    pub compression: Compression,
    pub enabled: bool,
    #[serde(default)]
    pub accept_relocation: bool,
    pub schedules: Vec<ScheduleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    #[serde(default)]
    pub id: i64,
    pub schedule_type: ScheduleType,
    pub cron_expression: String,
    pub enabled: bool,
    #[serde(default)]
    pub backup_sources: Vec<String>,
    #[serde(default)]
    pub rate_limit_kbps: Option<u32>,
    #[serde(default)]
    pub canary_enabled: bool,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub ignore_global_excludes: bool,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub keep_yearly: u32,
    pub compact_enabled: bool,
    #[serde(default)]
    pub pre_backup_commands: Vec<String>,
    #[serde(default)]
    pub post_backup_commands: Vec<String>,
}

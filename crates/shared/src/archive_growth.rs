// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Where an archive's breakdown of its deduplicated size stands.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema, strum_macros::Display,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
#[ts(export)]
pub enum ArchiveGrowthStatus {
    /// Requested, waiting for the repository.
    Pending,
    /// borg is reading the archive's files against the repository.
    Computing,
    /// The breakdown is ready.
    Done,
    /// Working it out failed; the error says why.
    Failed,
}

impl ArchiveGrowthStatus {
    /// Whether a computation is queued or running, so another is not started.
    #[must_use]
    pub const fn is_in_progress(self) -> bool {
        match self {
            Self::Pending | Self::Computing => true,
            Self::Done | Self::Failed => false,
        }
    }
}

impl FromStr for ArchiveGrowthStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "computing" => Ok(Self::Computing),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown archive growth status: {other}")),
        }
    }
}

/// What a node of the breakdown stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum GrowthNodeKind {
    /// A folder; its children say where inside it the data is.
    Directory,
    /// A single file.
    File,
    /// Entries of a folder too small to list one by one, taken together.
    Others,
}

/// One folder or file of an archive's breakdown, with the data in it that
/// no other archive holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GrowthNode {
    /// The entry's name in its parent. A folder that holds nothing but one
    /// other folder is merged with it, so this can span several levels
    /// (`home/alex`). Empty for [`GrowthNodeKind::Others`].
    pub name: String,
    /// What the node stands for.
    pub kind: GrowthNodeKind,
    /// Deduplicated, compressed bytes no other archive holds.
    #[ts(type = "number")]
    pub bytes: i64,
    /// How many files those bytes are in.
    #[ts(type = "number")]
    pub files: i64,
    /// A folder's entries, largest first. Entries too small to list are
    /// folded into a trailing [`GrowthNodeKind::Others`] node.
    #[schema(no_recursion)]
    pub children: Vec<GrowthNode>,
}

/// A file among the largest contributors to an archive's deduplicated size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GrowthFile {
    /// The file's path in the archive.
    pub path: String,
    /// Deduplicated, compressed bytes of the file no other archive holds.
    #[ts(type = "number")]
    pub bytes: i64,
    /// The file's own size.
    #[ts(type = "number")]
    pub size: i64,
}

/// Where an archive's deduplicated size sits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ArchiveGrowth {
    /// Deduplicated, compressed bytes no other archive holds, in total.
    #[ts(type = "number")]
    pub total_bytes: i64,
    /// How many files hold any of it.
    #[ts(type = "number")]
    pub files: i64,
    /// The top level of the archive, largest first.
    pub children: Vec<GrowthNode>,
    /// The files holding the most of it, largest first.
    pub largest_files: Vec<GrowthFile>,
}

/// An archive's breakdown of its deduplicated size, or where working it out
/// stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ArchiveGrowthResponse {
    /// Where the breakdown stands.
    pub status: ArchiveGrowthStatus,
    /// The breakdown, once done. A recomputation keeps showing nothing until
    /// it finishes.
    pub growth: Option<ArchiveGrowth>,
    /// When the breakdown was taken (RFC 3339). Later archives that keep the
    /// same data share it, so a breakdown taken later shows less.
    pub computed_at: Option<String>,
    /// Why working it out failed.
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_stored_form() {
        for status in [
            ArchiveGrowthStatus::Pending,
            ArchiveGrowthStatus::Computing,
            ArchiveGrowthStatus::Done,
            ArchiveGrowthStatus::Failed,
        ] {
            assert_eq!(
                status.to_string().parse::<ArchiveGrowthStatus>(),
                Ok(status)
            );
        }
        assert!("indexing".parse::<ArchiveGrowthStatus>().is_err());
    }

    #[test]
    fn only_queued_and_running_count_as_in_progress() {
        assert!(ArchiveGrowthStatus::Pending.is_in_progress());
        assert!(ArchiveGrowthStatus::Computing.is_in_progress());
        assert!(!ArchiveGrowthStatus::Done.is_in_progress());
        assert!(!ArchiveGrowthStatus::Failed.is_in_progress());
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What `virsh domjobinfo --completed` says about a backup job that ended,
//! and how a job that did not succeed is explained to the operator.

use std::str::FromStr;

use super::JobType;

/// The result of a finished libvirt job, as `virsh domjobinfo --completed`
/// prints it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct JobReport {
    /// How the job ended.
    pub(super) job_type: JobType,
    /// The job type as libvirt spelled it, kept for a type this build does not
    /// know.
    pub(super) raw_type: String,
    /// The reason libvirt gives for a failed job. libvirt has reported it
    /// since 6.3 as `Error message:`, and it is the only place the cause of a
    /// failed job (a full target disk, a target QEMU may not write) shows up
    /// without opening the domain's QEMU log.
    pub(super) error: Option<String>,
}

impl JobReport {
    /// Reads the fields this agent uses out of `domjobinfo` output.
    pub(super) fn parse(text: &str) -> Self {
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(name))
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let raw_type = field("Job type:").unwrap_or_default();
        Self {
            job_type: JobType::from_str(&raw_type).unwrap_or_default(),
            raw_type,
            error: field("Error message:"),
        }
    }

    /// Whether the job finished successfully.
    pub(super) fn completed(&self) -> bool {
        self.job_type == JobType::Completed
    }

    /// One sentence saying how the job for `domain` ended and, where libvirt
    /// says, why. Where it does not, it says where the reason can be found,
    /// because a bare "Failed" gives the operator nothing to act on.
    pub(super) fn describe(&self, domain: &str) -> String {
        let outcome = match self.job_type {
            JobType::Failed => format!("libvirt's backup job for {domain} failed"),
            JobType::Cancelled => format!(
                "libvirt's backup job for {domain} was cancelled, by `virsh domjobabort` or \
                 because the domain stopped"
            ),
            JobType::Absent | JobType::None => {
                format!("libvirt reported no result for the backup job for {domain}")
            }
            JobType::Bounded | JobType::Unbounded | JobType::Completed | JobType::Unknown => {
                format!(
                    "libvirt's backup job for {domain} ended as \"{}\"",
                    self.raw_type
                )
            }
        };
        self.error.as_ref().map_or_else(
            || {
                format!(
                    "{outcome}. libvirt gave no reason; the domain's QEMU log \
                     (/var/log/libvirt/qemu/{domain}.log on most hosts) names it."
                )
            },
            |error| format!("{outcome}: {error}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_job_is_explained_with_the_reason_libvirt_gives() {
        let report = JobReport::parse(
            "Job type:         Failed\nOperation:        Backup\nError message:    No space left \
             on device\n",
        );
        assert_eq!(report.job_type, JobType::Failed);
        assert!(!report.completed());
        assert_eq!(
            report.describe("truenas"),
            "libvirt's backup job for truenas failed: No space left on device"
        );
    }

    #[test]
    fn a_failed_job_without_a_reason_points_at_the_qemu_log() {
        let report = JobReport::parse("Job type:         Failed\n");
        let text = report.describe("truenas");
        assert!(
            text.starts_with("libvirt's backup job for truenas failed."),
            "{text}"
        );
        assert!(text.contains("/var/log/libvirt/qemu/truenas.log"), "{text}");
    }

    #[test]
    fn a_completed_job_is_recognised() {
        let report = JobReport::parse("Job type:         Completed\nOperation:        Backup\n");
        assert!(report.completed());
        assert_eq!(report.error, None);
    }

    #[test]
    fn missing_or_unknown_job_types_are_named() {
        assert!(
            JobReport::parse("")
                .describe("web01")
                .starts_with("libvirt reported no result")
        );
        assert!(
            JobReport::parse("Job type: Exploded\n")
                .describe("web01")
                .contains("ended as \"Exploded\"")
        );
        assert!(
            JobReport::parse("Job type: Cancelled\n")
                .describe("web01")
                .contains("was cancelled")
        );
    }
}

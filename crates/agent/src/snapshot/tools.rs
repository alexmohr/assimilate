// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The command-line tools that take and remove snapshots.
//!
//! Starting them is a deliberate exception to the rule against shelling out:
//! no maintained library takes ZFS or LVM snapshots, and the tools are what
//! the filesystems' own documentation uses. Each binary can be pointed
//! elsewhere through an environment variable, which the tests use to stand
//! in fakes.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::{ffi::OsStr, time::Duration};

use shared::snapshot::FsSnapshotKind;

/// How long one tool may take before it is stopped. Taking or removing a
/// snapshot is quick; one that hangs must not hold the backup forever.
#[cfg(target_os = "linux")]
const TOOL_TIMEOUT: Duration = Duration::from_mins(5);

/// Why a tool did not do what it was asked.
#[cfg(target_os = "linux")]
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("could not start {program}: {error}")]
    Spawn {
        program: String,
        #[source]
        error: std::io::Error,
    },
    #[error("{command} did not finish within {} seconds", TOOL_TIMEOUT.as_secs())]
    Timeout { command: String },
    #[error("{command} failed: {stderr}")]
    Failed { command: String, stderr: String },
}

/// Where each tool is started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub zfs: PathBuf,
    pub btrfs: PathBuf,
    pub lvcreate: PathBuf,
    pub lvremove: PathBuf,
    pub lvs: PathBuf,
    pub mount: PathBuf,
    pub umount: PathBuf,
}

impl Tools {
    /// The tools as this host's environment names them: each one's
    /// `ASSIMILATE_<TOOL>_BINARY` variable, or its plain name, found on the
    /// `PATH`.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|variable| std::env::var_os(variable))
    }

    /// The tools as `lookup` names them, given an environment variable's
    /// name.
    fn from_lookup(lookup: impl Fn(&str) -> Option<std::ffi::OsString>) -> Self {
        let tool = |name: &str| {
            lookup(&format!("ASSIMILATE_{}_BINARY", name.to_uppercase()))
                .map_or_else(|| PathBuf::from(name), PathBuf::from)
        };
        Self {
            zfs: tool("zfs"),
            btrfs: tool("btrfs"),
            lvcreate: tool("lvcreate"),
            lvremove: tool("lvremove"),
            lvs: tool("lvs"),
            mount: tool("mount"),
            umount: tool("umount"),
        }
    }

    /// The kinds of snapshot this host can take: those whose tools are all
    /// installed. Linux only; another host can take none.
    pub async fn available(&self) -> Vec<FsSnapshotKind> {
        if !cfg!(target_os = "linux") {
            return Vec::new();
        }
        let mut kinds = Vec::new();
        for (kind, tools) in [
            (FsSnapshotKind::Zfs, vec![&self.zfs]),
            (FsSnapshotKind::Btrfs, vec![&self.btrfs]),
            (
                FsSnapshotKind::Lvm,
                vec![
                    &self.lvcreate,
                    &self.lvremove,
                    &self.lvs,
                    &self.mount,
                    &self.umount,
                ],
            ),
        ] {
            let mut installed = true;
            for tool in tools {
                installed = installed && is_installed(tool).await;
            }
            if installed {
                kinds.push(kind);
            }
        }
        kinds
    }
}

/// Whether `program` can be started: an executable file at that path, or
/// for a bare name, in one of the `PATH` directories.
async fn is_installed(program: &Path) -> bool {
    if program.components().count() > 1 {
        return is_executable(program).await;
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&paths) {
        if is_executable(&dir.join(program)).await {
            return true;
        }
    }
    false
}

async fn is_executable(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|meta| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.is_file() && meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            meta.is_file()
        }
    })
}

/// Runs `program` with `args` and returns what it wrote to stdout. The
/// output is read in the C locale, so error messages are the tool's own
/// English ones. A tool still running after [`TOOL_TIMEOUT`] is killed.
///
/// # Errors
///
/// Returns an error when the tool cannot be started, takes too long, or
/// exits with anything but 0.
#[cfg(target_os = "linux")]
pub async fn run<A: AsRef<OsStr>>(program: &Path, args: &[A]) -> Result<String, ToolError> {
    let command = std::iter::once(program.as_os_str())
        .chain(args.iter().map(AsRef::as_ref))
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let child = tokio::process::Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| ToolError::Spawn {
            program: program.display().to_string(),
            error,
        })?;
    let output = tokio::time::timeout(TOOL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| ToolError::Timeout {
            command: command.clone(),
        })?
        .map_err(|error| ToolError::Spawn {
            program: program.display().to_string(),
            error,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(ToolError::Failed {
            command,
            stderr: if stderr.is_empty() {
                format!("exited with {}", output.status)
            } else {
                stderr
            },
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
#[cfg(unix)]
pub mod fake {
    //! Stand-in tools: shell scripts that log their arguments and answer
    //! with what a test tells them.

    use std::path::{Path, PathBuf};

    use super::Tools;

    /// Writes an executable script at `dir/name` that appends its name and
    /// arguments to `dir/calls.log` and then runs `body`.
    ///
    /// A shell writes it: had this process held the file open for writing,
    /// a child another test forks meanwhile could inherit that descriptor
    /// and make running the script fail with "Text file busy".
    pub async fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        let log = dir.join("calls.log");
        let text = format!(
            "#!/bin/sh\nprintf '%s' '{name}' >> '{log}'\nfor a in \"$@\"; do printf ' %s' \"$a\" \
             >> '{log}'; done\necho >> '{log}'\n{body}\n",
            log = log.display()
        );
        let written = tokio::process::Command::new("sh")
            .args([
                "-c",
                r#"printf '%s' "$1" > "$0.new" && chmod 755 "$0.new" && mv "$0.new" "$0""#,
            ])
            .arg(&path)
            .arg(text)
            .status()
            .await
            .unwrap();
        assert!(written.success(), "could not write {}", path.display());
        path
    }

    /// Every tool as a script that succeeds and prints nothing, except `lvs`,
    /// which answers `lvs_output`.
    pub async fn tools(dir: &Path, lvs_output: &str) -> Tools {
        let ok = |name: &'static str| script(dir, name, "exit 0");
        Tools {
            zfs: ok("zfs").await,
            btrfs: ok("btrfs").await,
            lvcreate: ok("lvcreate").await,
            lvremove: ok("lvremove").await,
            lvs: script(dir, "lvs", &format!("printf '%s\\n' '{lvs_output}'")).await,
            mount: ok("mount").await,
            umount: ok("umount").await,
        }
    }

    /// The commands the scripts in `dir` were called with, one per line.
    pub async fn calls(dir: &Path) -> Vec<String> {
        tokio::fs::read_to_string(dir.join("calls.log"))
            .await
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tool_can_be_named_by_the_environment() {
        let env = std::collections::HashMap::from([("ASSIMILATE_ZFS_BINARY", "/opt/zfs/bin/zfs")]);
        let tools = Tools::from_lookup(|variable| env.get(variable).map(Into::into));
        assert_eq!(tools.zfs, PathBuf::from("/opt/zfs/bin/zfs"));
        assert_eq!(tools.lvs, PathBuf::from("lvs"));
        assert_eq!(tools.umount, PathBuf::from("umount"));
        assert_eq!(
            Tools::from_env().mount.file_name(),
            Some(std::ffi::OsStr::new("mount"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_kinds_whose_tools_are_installed_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        let mut tools = fake::tools(dir.path(), "").await;
        if cfg!(target_os = "linux") {
            assert_eq!(
                tools.available().await,
                [
                    FsSnapshotKind::Zfs,
                    FsSnapshotKind::Btrfs,
                    FsSnapshotKind::Lvm
                ]
            );
        }
        tools.zfs = dir.path().join("missing");
        tools.umount = PathBuf::from("assimilate-no-such-tool");
        tools.btrfs = PathBuf::from("sh");
        let available = tools.available().await;
        assert!(!available.contains(&FsSnapshotKind::Zfs));
        assert!(!available.contains(&FsSnapshotKind::Lvm));
        assert_eq!(
            available.contains(&FsSnapshotKind::Btrfs),
            cfg!(target_os = "linux"),
            "a bare name is looked up on the PATH"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_tool_answers_with_its_output_or_why_it_failed() {
        let dir = tempfile::tempdir().unwrap();
        let ok = fake::script(dir.path(), "ok", "echo hello").await;
        assert_eq!(run(&ok, &["a"]).await.unwrap(), "hello\n");

        let fails = fake::script(dir.path(), "fails", "echo 'no space' >&2; exit 5").await;
        let error = run(&fails, &["-x", "vg/lv"]).await.unwrap_err();
        assert!(
            error
                .to_string()
                .ends_with("fails -x vg/lv failed: no space"),
            "{error}"
        );
        let silent = fake::script(dir.path(), "silent", "exit 3").await;
        let error = run(&silent, &[] as &[&str]).await.unwrap_err();
        assert!(error.to_string().contains("exited with"), "{error}");

        let missing = dir.path().join("missing");
        let error = run(&missing, &[] as &[&str]).await.unwrap_err();
        assert!(matches!(error, ToolError::Spawn { .. }), "{error}");
        assert_eq!(
            fake::calls(dir.path()).await,
            ["ok a", "fails -x vg/lv", "silent"]
        );
    }
}

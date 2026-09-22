// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Builds the environment and pattern files borg needs to run a backup or
//! maintenance command against a repository: `BORG_REPO`/`BORG_RSH`/etc, and
//! the exclude/include pattern files passed via `--exclude-from`/
//! `--patterns-from`. The agent's backup engine and its task executor both
//! run borg against the same kind of repository connection, so they share
//! this rather than each building the environment and pattern files their
//! own way.

use std::{io::Write as _, path::Path};

use tempfile::NamedTempFile;

use crate::{
    ssh::{borg_rsh, borg_rsh_with_known_hosts},
    types::{BORG_REPO_ENV_KEY, build_repo_url},
};

/// The connection details needed to build a borg environment. Callers with a
/// larger domain type (e.g. the agent's `BackupTarget`) extract these fields
/// rather than passing the type itself, so this crate doesn't need to depend
/// on it.
pub struct EnvParams<'a> {
    /// SSH user borg connects as.
    pub ssh_user: &'a str,
    /// SSH host borg connects to.
    pub ssh_host: &'a str,
    /// SSH port borg connects to.
    pub ssh_port: u16,
    /// Path to the repository on `ssh_host`.
    pub repo_path: &'a str,
    /// The repository's encryption passphrase.
    pub passphrase: &'a str,
    /// This agent's hostname, recorded as `BORG_HOST_ID` so borg can tell
    /// which host holds a lock.
    pub hostname: &'a str,
    /// The host key recorded for `ssh_host` out of band (e.g. pinned at
    /// repository setup), if any. An empty string means none is recorded.
    pub ssh_host_key: &'a str,
    /// A `known_hosts` file pinning `ssh_host`'s key, if one has been
    /// written for this repository.
    pub known_hosts_path: Option<&'a Path>,
    /// Whether this repository is allowed to have moved since it was last
    /// used, sent as `BORG_RELOCATED_REPO_ACCESS_IS_OK` when true.
    pub accept_relocation: bool,
    /// An `SSH_AUTH_SOCK` to forward into borg's environment, if agent
    /// forwarding is in use.
    pub ssh_auth_sock: Option<&'a Path>,
}

/// Picks the `--rsh` command: pinned to `known_hosts_path` when one is
/// recorded, otherwise auto-accepting a new host key unless one was already
/// pinned out of band (`ssh_host_key` non-empty) - in which case `false`
/// refuses the connection rather than silently trusting an unpinned key.
#[must_use]
pub fn rsh_for_target(ssh_host_key: &str, known_hosts_path: Option<&Path>) -> String {
    known_hosts_path.map_or_else(
        || {
            if ssh_host_key.is_empty() {
                borg_rsh()
            } else {
                "false".to_owned()
            }
        },
        borg_rsh_with_known_hosts,
    )
}

/// Builds the environment variables borg needs to run a command against
/// `params`'s repository: `BORG_REPO`, `BORG_PASSPHRASE`, `BORG_HOST_ID`,
/// `BORG_RSH`, locale, and (when applicable) relocation acceptance and SSH
/// agent forwarding.
#[must_use]
pub fn build_env(params: &EnvParams<'_>) -> Vec<(String, String)> {
    let repo_url = build_repo_url(
        params.ssh_user,
        params.ssh_host,
        params.ssh_port,
        params.repo_path,
    );

    let mut env = vec![
        (BORG_REPO_ENV_KEY.to_owned(), repo_url),
        ("BORG_PASSPHRASE".to_owned(), params.passphrase.to_owned()),
        ("BORG_HOST_ID".to_owned(), params.hostname.to_owned()),
        (
            "BORG_RSH".to_owned(),
            rsh_for_target(params.ssh_host_key, params.known_hosts_path),
        ),
        ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
        ("LC_CTYPE".to_owned(), "en_US.UTF-8".to_owned()),
    ];

    if params.accept_relocation {
        env.push((
            "BORG_RELOCATED_REPO_ACCESS_IS_OK".to_owned(),
            "yes".to_owned(),
        ));
    }

    if let Some(sock) = params.ssh_auth_sock {
        env.push((
            "SSH_AUTH_SOCK".to_owned(),
            sock.to_string_lossy().into_owned(),
        ));
    }

    env
}

/// Writes `patterns` to a temp file, one per line, for `borg --exclude-from`.
/// Always creates the file, even when `patterns` is empty, so callers can
/// rely on a path always being available.
///
/// # Errors
///
/// Returns an error if the temp file can't be created or written.
pub fn write_exclude_file(patterns: &[String]) -> std::io::Result<NamedTempFile> {
    let mut file = NamedTempFile::new()?;
    for pattern in patterns {
        writeln!(file, "{pattern}")?;
    }
    file.flush()?;
    Ok(file)
}

/// Writes a borg patterns file rescuing `patterns` from the exclude list -
/// each line prefixed `+` (include), checked before `--exclude-from` and
/// winning first-match-wins. Returns `None` when there is nothing to rescue,
/// so a caller with no include patterns runs the exact command it always has.
///
/// # Errors
///
/// Returns an error if the temp file can't be created or written.
pub fn write_include_patterns_file(patterns: &[String]) -> std::io::Result<Option<NamedTempFile>> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut file = NamedTempFile::new()?;
    for pattern in patterns {
        writeln!(file, "+ {pattern}")?;
    }
    file.flush()?;
    Ok(Some(file))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous assertions"
)]
mod tests {
    use super::*;

    fn params() -> EnvParams<'static> {
        EnvParams {
            ssh_user: "borg",
            ssh_host: "backup-server",
            ssh_port: 22,
            repo_path: "backup/test",
            passphrase: "secret",
            hostname: "test-host",
            ssh_host_key: "",
            known_hosts_path: None,
            accept_relocation: false,
            ssh_auth_sock: None,
        }
    }

    #[test]
    fn build_env_sets_the_core_borg_variables() {
        let env = build_env(&params());
        let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(
            get(BORG_REPO_ENV_KEY),
            Some("ssh://borg@backup-server:22/backup/test")
        );
        assert_eq!(get("BORG_PASSPHRASE"), Some("secret"));
        assert_eq!(get("BORG_HOST_ID"), Some("test-host"));
        assert_eq!(get("LANG"), Some("en_US.UTF-8"));
    }

    #[test]
    fn build_env_omits_relocation_and_ssh_auth_sock_by_default() {
        let env = build_env(&params());
        assert!(
            !env.iter()
                .any(|(k, _)| k == "BORG_RELOCATED_REPO_ACCESS_IS_OK")
        );
        assert!(!env.iter().any(|(k, _)| k == "SSH_AUTH_SOCK"));
    }

    #[test]
    fn build_env_includes_relocation_flag_when_accepted() {
        let env = build_env(&EnvParams {
            accept_relocation: true,
            ..params()
        });
        assert!(
            env.iter()
                .any(|(k, v)| k == "BORG_RELOCATED_REPO_ACCESS_IS_OK" && v == "yes")
        );
    }

    #[test]
    fn build_env_forwards_ssh_auth_sock_when_present() {
        let sock = Path::new("/tmp/agent.sock");
        let env = build_env(&EnvParams {
            ssh_auth_sock: Some(sock),
            ..params()
        });
        assert!(
            env.iter()
                .any(|(k, v)| k == "SSH_AUTH_SOCK" && v == "/tmp/agent.sock")
        );
    }

    #[test]
    fn rsh_for_target_uses_pinned_known_hosts_file_when_present() {
        let known_hosts = tempfile::NamedTempFile::new().unwrap();
        let rsh = rsh_for_target("ssh-ed25519 AAAATEST", Some(known_hosts.path()));
        assert_eq!(rsh, borg_rsh_with_known_hosts(known_hosts.path()));
    }

    #[test]
    fn rsh_for_target_refuses_when_host_key_recorded_but_not_pinned() {
        assert_eq!(rsh_for_target("ssh-ed25519 AAAATEST", None), "false");
    }

    #[test]
    fn rsh_for_target_auto_accepts_when_nothing_pinned() {
        assert_eq!(rsh_for_target("", None), borg_rsh());
    }

    #[test]
    fn write_exclude_file_writes_one_pattern_per_line() {
        let file = write_exclude_file(&["*.tmp".to_owned(), "/proc/*".to_owned()]).unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(content, "*.tmp\n/proc/*\n");
    }

    #[test]
    fn write_exclude_file_creates_an_empty_file_when_no_patterns() {
        let file = write_exclude_file(&[]).unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn write_include_patterns_file_returns_none_when_empty() {
        assert!(write_include_patterns_file(&[]).unwrap().is_none());
    }

    #[test]
    fn write_include_patterns_file_prefixes_each_pattern_with_plus() {
        let file =
            write_include_patterns_file(&["/home/keep".to_owned(), "pp:/var/keep".to_owned()])
                .unwrap()
                .unwrap();
        let content = std::fs::read_to_string(file.path()).unwrap();
        assert_eq!(content, "+ /home/keep\n+ pp:/var/keep\n");
    }
}

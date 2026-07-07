// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

/// Builds the `--rsh` command line passed to `borg` for SSH transport,
/// using batch mode and auto-accepting unknown host keys on first
/// connection. Suitable when no pinned `known_hosts` file is available.
#[must_use]
pub fn borg_rsh() -> String {
    [
        "ssh",
        "-o BatchMode=yes",
        "-o StrictHostKeyChecking=accept-new",
        "-o ServerAliveInterval=15",
        "-o ServerAliveCountMax=3",
        "-o ConnectTimeout=30",
    ]
    .join(" ")
}

/// Builds the `--rsh` command line passed to `borg` for SSH transport,
/// pinning host key verification to the `known_hosts` file at `path` instead
/// of accepting new keys automatically.
#[must_use]
pub fn borg_rsh_with_known_hosts(path: &Path) -> String {
    [
        "ssh",
        "-o BatchMode=yes",
        "-o StrictHostKeyChecking=yes",
        "-o ServerAliveInterval=15",
        "-o ServerAliveCountMax=3",
        "-o ConnectTimeout=30",
        &format!("-o UserKnownHostsFile={}", path.display()),
    ]
    .join(" ")
}

/// Formats `host` as it should appear in a `known_hosts` file entry: the
/// bare hostname for the default SSH port (22), or a bracketed
/// `[host]:port` form for any non-default port.
#[must_use]
pub fn known_hosts_host(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borg_rsh_uses_default_known_hosts_file() {
        let ssh = borg_rsh();

        assert!(ssh.contains("BatchMode=yes"));
        assert!(ssh.contains("StrictHostKeyChecking=accept-new"));
        assert!(ssh.contains("ServerAliveInterval=15"));
        assert!(ssh.contains("ServerAliveCountMax=3"));
        assert!(ssh.contains("ConnectTimeout=30"));
        assert!(!ssh.contains("UserKnownHostsFile"));
    }

    #[test]
    fn pinned_borg_rsh_requires_the_provided_known_hosts_file() {
        let ssh = borg_rsh_with_known_hosts(Path::new("/tmp/known-hosts"));

        assert!(ssh.contains("StrictHostKeyChecking=yes"));
        assert!(ssh.contains("ServerAliveInterval=15"));
        assert!(ssh.contains("UserKnownHostsFile=/tmp/known-hosts"));
    }

    #[test]
    fn known_hosts_host_includes_nonstandard_ports() {
        assert_eq!(known_hosts_host("repo.example.com", 22), "repo.example.com");
        assert_eq!(
            known_hosts_host("repo.example.com", 2222),
            "[repo.example.com]:2222"
        );
    }
}

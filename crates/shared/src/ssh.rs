// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

pub fn borg_rsh() -> String {
    "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null"
        .to_owned()
}

pub fn borg_rsh_with_known_hosts(path: &Path) -> String {
    format!(
        "ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile={}",
        path.display()
    )
}

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
    fn borg_rsh_uses_transient_known_hosts_file() {
        let ssh = borg_rsh();

        assert!(ssh.contains("BatchMode=yes"));
        assert!(ssh.contains("StrictHostKeyChecking=accept-new"));
        assert!(ssh.contains("UserKnownHostsFile=/dev/null"));
    }

    #[test]
    fn pinned_borg_rsh_requires_the_provided_known_hosts_file() {
        let ssh = borg_rsh_with_known_hosts(Path::new("/tmp/known-hosts"));

        assert!(ssh.contains("StrictHostKeyChecking=yes"));
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

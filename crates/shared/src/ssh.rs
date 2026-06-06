// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub fn borg_rsh() -> String {
    "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/dev/null"
        .to_owned()
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
}

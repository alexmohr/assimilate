// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The macOS launchd daemon's files: where they live, the property list
//! launchd reads and who may own the executable it runs. Nothing here
//! touches the system, so it builds and is tested everywhere; `launchd.rs` is
//! what installs it on a Mac.

use std::path::{Path, PathBuf};

/// The daemon's launchd label; its property list is named for it.
pub const LABEL: &str = "com.assimilate.agent";

/// The `PATH` the daemon runs with. launchd's own has only the system's
/// folders, which would hide the database clients and `ssh` Homebrew installs.
const PATH: &str =
    "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// Seconds launchd waits before it starts the agent again after it exits.
pub const RESTART_DELAY_SECS: u32 = 10;

/// Where the daemon keeps its files.
pub struct DaemonPaths {
    daemons_dir: PathBuf,
    support_dir: PathBuf,
    log: PathBuf,
}

impl DaemonPaths {
    /// The paths on a real Mac.
    pub fn system() -> Self {
        Self::under(Path::new("/"))
    }

    /// The paths below `root`, which is `/` on a real Mac.
    pub fn under(root: &Path) -> Self {
        Self {
            daemons_dir: root.join("Library/LaunchDaemons"),
            support_dir: root.join("Library/Application Support/Assimilate/agent"),
            log: root.join("var/log/assimilate-agent.log"),
        }
    }

    /// The property list launchd loads the daemon from.
    pub fn plist(&self) -> PathBuf {
        self.daemons_dir.join(format!("{LABEL}.plist"))
    }

    /// The folder only root can open, which holds the configuration.
    pub fn support_dir(&self) -> &Path {
        &self.support_dir
    }

    /// The stored server URL, token and server key.
    pub fn config(&self) -> PathBuf {
        self.support_dir.join("agent.json")
    }

    /// Where launchd sends the agent's standard output and error.
    pub fn log(&self) -> &Path {
        &self.log
    }
}

/// Whether a file or folder owned by `uid` with permission bits `mode` can
/// be changed only by root, which the daemon's executable and its folder must
/// be: launchd runs it as root.
pub const fn only_root_can_write(uid: u32, mode: u32) -> bool {
    uid == 0 && mode & 0o022 == 0
}

const DOCTYPE: &str = concat!(
    r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "#,
    r#""http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#
);

/// `text` as XML character data.
fn escape(text: &str) -> String {
    text.chars().fold(String::new(), |mut escaped, c| {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
        escaped
    })
}

/// The property list that runs `exe service run` as a daemon that launchd
/// starts at boot and starts again after every exit, which is how the
/// dashboard's "Restart agent" works.
pub fn render(exe: &Path, log: &Path) -> String {
    let exe = escape(&exe.to_string_lossy());
    let log = escape(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
{DOCTYPE}
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>service</string>
    <string>run</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>{PATH}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>{RESTART_DELAY_SECS}</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_live_in_the_system_folders_launchd_expects() {
        let paths = DaemonPaths::system();

        assert_eq!(
            paths.plist(),
            Path::new("/Library/LaunchDaemons/com.assimilate.agent.plist")
        );
        assert_eq!(
            paths.support_dir(),
            Path::new("/Library/Application Support/Assimilate/agent")
        );
        assert_eq!(
            paths.config(),
            Path::new("/Library/Application Support/Assimilate/agent/agent.json")
        );
        assert_eq!(paths.log(), Path::new("/var/log/assimilate-agent.log"));
    }

    #[test]
    fn the_plist_runs_the_service_and_keeps_it_alive() {
        let plist = render(
            Path::new("/usr/local/lib/assimilate-agent/assimilate-agent"),
            Path::new("/var/log/assimilate-agent.log"),
        );

        assert!(plist.contains("<string>com.assimilate.agent</string>"));
        assert!(plist.contains(
            "<string>/usr/local/lib/assimilate-agent/assimilate-agent</string>\n    \
             <string>service</string>\n    <string>run</string>"
        ));
        assert!(plist.contains("<key>PATH</key>\n    <string>/opt/homebrew/bin:"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>\n  <true/>"));
        assert!(plist.contains("<integer>10</integer>"));
        assert_eq!(
            plist
                .matches("<string>/var/log/assimilate-agent.log</string>")
                .count(),
            2,
            "both output streams go to the log"
        );
    }

    #[test]
    fn only_root_owned_files_nobody_else_can_write_are_trusted() {
        assert!(only_root_can_write(0, 0o755));
        assert!(only_root_can_write(0, 0o500));
        assert!(!only_root_can_write(501, 0o755), "owned by a user");
        assert!(!only_root_can_write(0, 0o775), "group-writable");
        assert!(!only_root_can_write(0, 0o757), "world-writable");
    }

    #[test]
    fn a_path_with_markup_cannot_break_out_of_the_plist() {
        let plist = render(
            Path::new("/Applications/R&D <x>/assimilate-agent"),
            Path::new("/var/log/a.log"),
        );

        assert!(
            plist.contains("<string>/Applications/R&amp;D &lt;x&gt;/assimilate-agent</string>")
        );
        assert!(!plist.contains("<x>"));
    }
}

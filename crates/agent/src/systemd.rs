// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

#[cfg(all(unix, not(target_os = "macos")))]
use std::process::Stdio;

#[cfg(all(unix, not(target_os = "macos")))]
use tokio::process::Command;
#[cfg(all(unix, not(target_os = "macos")))]
use tracing::debug;

#[cfg(all(unix, not(target_os = "macos")))]
const SERVICE_NAME: &str = "assimilate-agent";

pub struct RestartCapability {
    pub supported: bool,
    pub unavailable_reason: Option<String>,
}

/// Run from a console, the agent has no service manager to start it again;
/// the Windows service (`service run`) reports its own capability.
#[cfg(windows)]
pub fn detect_restart_capability() -> std::future::Ready<RestartCapability> {
    std::future::ready(RestartCapability {
        supported: false,
        unavailable_reason: Some(
            "the agent restarts itself only when it runs as the Windows service (`service \
             install`)"
                .to_owned(),
        ),
    })
}

/// Run from a console, the agent has no service manager to start it again;
/// the launchd daemon (`service run`) reports its own capability.
#[cfg(target_os = "macos")]
pub fn detect_restart_capability() -> std::future::Ready<RestartCapability> {
    std::future::ready(RestartCapability {
        supported: false,
        unavailable_reason: Some(
            "the agent restarts itself only when it runs as the launchd daemon (`service install`)"
                .to_owned(),
        ),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
pub async fn detect_restart_capability() -> RestartCapability {
    let Some(systemctl) = which_systemctl() else {
        return RestartCapability {
            supported: false,
            unavailable_reason: Some("systemd is not available on this system".to_owned()),
        };
    };

    let our_pid = std::process::id().to_string();

    let output = Command::new(&systemctl)
        .args([
            "show",
            &format!("{SERVICE_NAME}.service"),
            "--property=MainPID",
            "--value",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;

    let Ok(output) = output else {
        return RestartCapability {
            supported: false,
            unavailable_reason: Some("failed to query systemd service status".to_owned()),
        };
    };

    if !output.status.success() {
        return RestartCapability {
            supported: false,
            unavailable_reason: Some(format!("systemd service '{SERVICE_NAME}' not found")),
        };
    }

    let main_pid = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    if main_pid == our_pid {
        debug!("running as systemd service '{SERVICE_NAME}' (PID {our_pid})");
        RestartCapability {
            supported: true,
            unavailable_reason: None,
        }
    } else {
        RestartCapability {
            supported: false,
            unavailable_reason: Some(format!(
                "agent is not running as the '{SERVICE_NAME}' systemd service (expected PID \
                 {main_pid}, got {our_pid})"
            )),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn which_systemctl() -> Option<String> {
    which::which("systemctl")
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_console_agent_points_at_the_service() {
        let manager = if cfg!(windows) {
            "Windows service"
        } else {
            "launchd daemon"
        };
        let capability = detect_restart_capability().await;

        assert!(!capability.supported);
        assert!(
            capability
                .unavailable_reason
                .is_some_and(|reason| reason.contains(manager))
        );
    }
}

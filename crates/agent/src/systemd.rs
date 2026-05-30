// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::process::Stdio;

use tokio::process::Command;
use tracing::debug;

const SERVICE_NAME: &str = "assimilate-agent";

pub struct RestartCapability {
    pub supported: bool,
    pub unavailable_reason: Option<String>,
}

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

fn which_systemctl() -> Option<String> {
    which::which("systemctl")
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

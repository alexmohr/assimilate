// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::Engine;
use russh::{
    client,
    keys::{PrivateKey, PublicKey, key::PrivateKeyWithHashAlg},
};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("SSH connection failed: {0}")]
    Connection(String),
    #[error("SSH authentication failed: {0}")]
    Auth(String),
    #[error("SFTP error: {0}")]
    Sftp(String),
    #[error("command execution failed: {0}")]
    Exec(String),
    #[error("server public key not found at {0}")]
    PublicKeyNotFound(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct TestConnectionRequest {
    pub ssh_host: String,
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<u16>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TestConnectionResponse {
    pub ssh_ok: bool,
    pub borg_installed: bool,
    pub borg_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DeployKeyRequest {
    pub ssh_host: String,
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<u16>,
    pub password: String,
    #[serde(default = "default_use_sftp")]
    pub use_sftp: bool,
}

fn default_ssh_user() -> String {
    "borg".to_string()
}

fn default_use_sftp() -> bool {
    true
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeployKeyResponse {
    pub success: bool,
    pub already_deployed: bool,
    pub error: Option<String>,
}

pub(crate) struct SshClientHandler {
    pub expected_host_key: Option<String>,
}

impl client::Handler for SshClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> {
        let Some(expected) = &self.expected_host_key else {
            return std::future::ready(Ok(true));
        };
        let actual = server_public_key.to_openssh().unwrap_or_default();
        if actual.trim() == expected.trim() {
            std::future::ready(Ok(true))
        } else {
            tracing::error!("SSH host key mismatch: expected {expected}, got {actual}");
            std::future::ready(Ok(false))
        }
    }
}

struct HostKeyCaptureHandler {
    host_key: Arc<Mutex<Option<String>>>,
}

impl client::Handler for HostKeyCaptureHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> {
        if let Ok(mut host_key) = self.host_key.lock() {
            *host_key = server_public_key.to_openssh().ok();
        }
        std::future::ready(Ok(true))
    }
}

fn ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..client::Config::default()
    })
}

/// # Errors
///
/// Returns [`SshError::Connection`] if the operation fails.
pub async fn scan_host_key(host: &str, port: u16) -> Result<String, SshError> {
    let host_key = Arc::new(Mutex::new(None));
    let handler = HostKeyCaptureHandler {
        host_key: Arc::clone(&host_key),
    };
    let session = client::connect(ssh_config(), (host, port), handler)
        .await
        .map_err(|e| SshError::Connection(format!("{host}:{port}: {e}")))?;
    drop(session);

    host_key
        .lock()
        .map_err(|_| SshError::Connection("SSH host key capture lock poisoned".to_owned()))?
        .clone()
        .ok_or_else(|| SshError::Connection(format!("{host}:{port}: no SSH host key received")))
}

/// # Errors
///
/// Returns [`SshError::PublicKeyNotFound`] if the operation fails.
pub async fn read_server_public_key() -> Result<String, SshError> {
    let ssh_key_dir = std::env::var("SSH_KEY_DIR").unwrap_or_else(|_| "/ssh-keys".to_string());
    let pub_key_path = PathBuf::from(&ssh_key_dir).join("id_ed25519.pub");

    tokio::fs::read_to_string(&pub_key_path)
        .await
        .map(|s| s.trim().to_string())
        .map_err(|_| SshError::PublicKeyNotFound(pub_key_path))
}

/// # Errors
///
/// Returns an error if:
/// - [`SshError::PublicKeyNotFound`]: the operation fails
/// - [`SshError::Auth`]: SSH authentication fails
pub async fn load_server_private_key() -> Result<PrivateKey, SshError> {
    let ssh_key_dir = std::env::var("SSH_KEY_DIR").unwrap_or_else(|_| "/ssh-keys".to_string());
    let key_path = PathBuf::from(&ssh_key_dir).join("id_ed25519");

    let key_data = tokio::fs::read_to_string(&key_path)
        .await
        .map_err(|_| SshError::PublicKeyNotFound(key_path.clone()))?;

    russh::keys::decode_secret_key(&key_data, None).map_err(|e| {
        SshError::Auth(format!(
            "failed to decode private key at {}: {e}",
            key_path.display()
        ))
    })
}

pub(crate) async fn connect_with_key(
    host: &str,
    user: &str,
    port: u16,
    expected_host_key: Option<String>,
) -> Result<client::Handle<SshClientHandler>, SshError> {
    let config = ssh_config();
    let handler = SshClientHandler { expected_host_key };

    let mut session = client::connect(config, (host, port), handler)
        .await
        .map_err(|e| SshError::Connection(format!("{host}:{port}: {e}")))?;

    let key = load_server_private_key().await?;
    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);

    let auth_result = session
        .authenticate_publickey(user, key_with_alg)
        .await
        .map_err(|e| SshError::Auth(e.to_string()))?;

    if !auth_result.success() {
        return Err(SshError::Auth(
            "public key authentication rejected".to_string(),
        ));
    }

    Ok(session)
}

async fn connect_with_password(
    host: &str,
    user: &str,
    port: u16,
    password: &str,
) -> Result<client::Handle<SshClientHandler>, SshError> {
    let config = ssh_config();
    let handler = SshClientHandler {
        expected_host_key: None,
    };

    let mut session = client::connect(config, (host, port), handler)
        .await
        .map_err(|e| SshError::Connection(format!("{host}:{port}: {e}")))?;

    let auth_result = session
        .authenticate_password(user, password)
        .await
        .map_err(|e| SshError::Auth(e.to_string()))?;

    if !auth_result.success() {
        return Err(SshError::Auth("password authentication failed".to_string()));
    }

    Ok(session)
}

async fn open_sftp(session: &client::Handle<SshClientHandler>) -> Result<SftpSession, SshError> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Sftp(format!("failed to open channel: {e}")))?;

    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| SshError::Sftp(format!("failed to request sftp subsystem: {e}")))?;

    SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| SshError::Sftp(format!("failed to init sftp session: {e}")))
}

pub(crate) async fn exec_command(
    session: &client::Handle<SshClientHandler>,
    command: &str,
) -> Result<(u32, String, String), SshError> {
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Exec(format!("failed to open channel: {e}")))?;

    channel
        .exec(true, command)
        .await
        .map_err(|e| SshError::Exec(format!("failed to exec command: {e}")))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = 0u32;

    loop {
        let Some(msg) = channel.wait().await else {
            break;
        };
        match msg {
            russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                stderr.extend_from_slice(&data);
            }
            russh::ChannelMsg::ExitStatus { exit_status: code } => exit_status = code,
            _ => {}
        }
    }

    Ok((
        exit_status,
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}

pub async fn test_connection(req: &TestConnectionRequest) -> TestConnectionResponse {
    let port = req.ssh_port.unwrap_or(22);

    let session = match connect_with_key(&req.ssh_host, &req.ssh_user, port, None).await {
        Ok(s) => s,
        Err(e) => {
            return TestConnectionResponse {
                ssh_ok: false,
                borg_installed: false,
                borg_version: None,
                error: Some(e.to_string()),
            };
        }
    };

    let (exit_code, stdout, _stderr) = match exec_command(&session, "borg --version").await {
        Ok(result) => result,
        Err(e) => {
            return TestConnectionResponse {
                ssh_ok: true,
                borg_installed: false,
                borg_version: None,
                error: Some(format!("SSH ok, but failed to check borg: {e}")),
            };
        }
    };

    if exit_code == 0 {
        let version = stdout.trim().to_string();
        info!(host = %req.ssh_host, version = %version, "connection test: borg found");
        TestConnectionResponse {
            ssh_ok: true,
            borg_installed: true,
            borg_version: Some(version),
            error: None,
        }
    } else {
        TestConnectionResponse {
            ssh_ok: true,
            borg_installed: false,
            borg_version: None,
            error: None,
        }
    }
}

pub async fn deploy_key(req: &DeployKeyRequest) -> DeployKeyResponse {
    let port = req.ssh_port.unwrap_or(22);

    if connect_with_key(&req.ssh_host, &req.ssh_user, port, None)
        .await
        .is_ok()
    {
        info!(host = %req.ssh_host, "key already deployed");
        return DeployKeyResponse {
            success: true,
            already_deployed: true,
            error: None,
        };
    }

    let our_key = match read_server_public_key().await {
        Ok(k) => k,
        Err(e) => {
            return DeployKeyResponse {
                success: false,
                already_deployed: false,
                error: Some(e.to_string()),
            };
        }
    };

    let session =
        match connect_with_password(&req.ssh_host, &req.ssh_user, port, &req.password).await {
            Ok(s) => s,
            Err(e) => {
                return DeployKeyResponse {
                    success: false,
                    already_deployed: false,
                    error: Some(e.to_string()),
                };
            }
        };

    let deploy_result = if req.use_sftp {
        deploy_key_sftp(&session, &our_key).await
    } else {
        deploy_key_shell(&session, &our_key).await
    };

    if let Err(e) = deploy_result {
        return DeployKeyResponse {
            success: false,
            already_deployed: false,
            error: Some(e.to_string()),
        };
    }

    match connect_with_key(&req.ssh_host, &req.ssh_user, port, None).await {
        Ok(_) => {
            info!(host = %req.ssh_host, "key deployed and verified");
            DeployKeyResponse {
                success: true,
                already_deployed: false,
                error: None,
            }
        }
        Err(e) => {
            warn!(host = %req.ssh_host, error = %e, "key deployed but verification failed");
            DeployKeyResponse {
                success: false,
                already_deployed: false,
                error: Some(format!("key was uploaded but verification failed: {e}")),
            }
        }
    }
}

async fn deploy_key_sftp(
    session: &client::Handle<SshClientHandler>,
    public_key: &str,
) -> Result<(), SshError> {
    let sftp = open_sftp(session).await?;

    if let Err(e) = sftp.create_dir(".ssh").await {
        tracing::debug!(error = %e, "sftp create_dir .ssh failed (may already exist)");
    }

    let existing = match sftp.read(".ssh/authorized_keys").await {
        Ok(data) => String::from_utf8_lossy(&data).into_owned(),
        Err(e) => {
            tracing::debug!(error = %e, "reading .ssh/authorized_keys failed (may not exist)");
            String::new()
        }
    };

    if existing.contains(public_key) {
        return Ok(());
    }

    let new_content = if existing.is_empty() {
        format!("{public_key}\n")
    } else {
        format!("{}\n{public_key}\n", existing.trim_end())
    };

    sftp.create(".ssh/authorized_keys")
        .await
        .map_err(|e| SshError::Sftp(format!("failed to write authorized_keys: {e}")))?
        .write_all(new_content.as_bytes())
        .await
        .map_err(|e| SshError::Sftp(format!("failed to write authorized_keys: {e}")))?;

    Ok(())
}

async fn deploy_key_shell(
    session: &client::Handle<SshClientHandler>,
    public_key: &str,
) -> Result<(), SshError> {
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|e| SshError::Exec(format!("failed to open channel: {e}")))?;

    channel
        .exec(true, "mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys")
        .await
        .map_err(|e| SshError::Exec(format!("failed to exec: {e}")))?;

    channel
        .data(format!("{public_key}\n").as_bytes())
        .await
        .map_err(|e| SshError::Exec(format!("failed to send key data: {e}")))?;

    channel
        .eof()
        .await
        .map_err(|e| SshError::Exec(format!("failed to send eof: {e}")))?;

    loop {
        let Some(msg) = channel.wait().await else {
            break;
        };
        if let russh::ChannelMsg::ExitStatus { exit_status } = msg
            && exit_status != 0
        {
            return Err(SshError::Exec(format!(
                "shell deploy command exited with status {exit_status}"
            )));
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListDirRequest {
    pub ssh_host: String,
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<u16>,
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ListDirResponse {
    pub path: String,
    pub entries: Vec<DirEntryInfo>,
    pub error: Option<String>,
}

pub async fn list_dir(req: &ListDirRequest) -> ListDirResponse {
    let port = req.ssh_port.unwrap_or(22);
    let path = if req.path.is_empty() { "/" } else { &req.path };

    let session = match connect_with_key(&req.ssh_host, &req.ssh_user, port, None).await {
        Ok(s) => s,
        Err(e) => {
            return ListDirResponse {
                path: path.to_string(),
                entries: Vec::new(),
                error: Some(e.to_string()),
            };
        }
    };

    let sftp = match open_sftp(&session).await {
        Ok(s) => s,
        Err(e) => {
            return ListDirResponse {
                path: path.to_string(),
                entries: Vec::new(),
                error: Some(e.to_string()),
            };
        }
    };

    let canonical = sftp
        .canonicalize(path.to_string())
        .await
        .unwrap_or_else(|_| path.to_string());

    let read_dir = match sftp.read_dir(canonical.clone()).await {
        Ok(rd) => rd,
        Err(e) => {
            return ListDirResponse {
                path: canonical,
                entries: Vec::new(),
                error: Some(format!("failed to read directory: {e}")),
            };
        }
    };

    let mut entries: Vec<DirEntryInfo> = read_dir
        .map(|entry| DirEntryInfo {
            name: entry.file_name(),
            is_dir: entry.file_type().is_dir(),
        })
        .collect();

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    ListDirResponse {
        path: canonical,
        entries,
        error: None,
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MkdirRequest {
    pub ssh_host: String,
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    pub ssh_port: Option<u16>,
    pub path: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MkdirResponse {
    pub success: bool,
    pub path: String,
    pub error: Option<String>,
}

pub async fn mkdir(req: &MkdirRequest) -> MkdirResponse {
    let port = req.ssh_port.unwrap_or(22);
    let path = if req.path.is_empty() {
        return MkdirResponse {
            success: false,
            path: String::new(),
            error: Some("path must not be empty".to_string()),
        };
    } else {
        &req.path
    };

    let session = match connect_with_key(&req.ssh_host, &req.ssh_user, port, None).await {
        Ok(s) => s,
        Err(e) => {
            return MkdirResponse {
                success: false,
                path: path.clone(),
                error: Some(e.to_string()),
            };
        }
    };

    let sftp = match open_sftp(&session).await {
        Ok(s) => s,
        Err(e) => {
            return MkdirResponse {
                success: false,
                path: path.clone(),
                error: Some(e.to_string()),
            };
        }
    };

    if let Err(e) = sftp.create_dir(path.clone()).await {
        return MkdirResponse {
            success: false,
            path: path.clone(),
            error: Some(format!("failed to create directory: {e}")),
        };
    }

    let canonical = sftp
        .canonicalize(path.clone())
        .await
        .unwrap_or_else(|_| path.clone());

    MkdirResponse {
        success: true,
        path: canonical,
        error: None,
    }
}

pub struct DeployAgentParams<'a> {
    pub host: &'a str,
    pub user: &'a str,
    pub port: u16,
    pub binary_dir: &'a std::path::Path,
    pub remote_path: &'a str,
    pub server_url: &'a str,
    pub token: &'a str,
    pub password: Option<&'a str>,
    pub systemd_service_content: Option<&'a str>,
}

fn default_unit_content(remote_path: &str, server_url: &str, token: &str) -> String {
    format!(
        "\
[Unit]
Description=Assimilate Backup Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={remote_path}
Environment=BORG_SERVER_URL={server_url}
Environment=BORG_AGENT_TOKEN={token}
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
"
    )
}

/// Build a sudo command string from a base command and optional password.
fn build_sudo_cmd(command: &str, password: Option<&str>) -> String {
    match password {
        Some(pw) => format!(
            "echo {} | sudo -S sh -c {}",
            shell_escape(pw),
            shell_escape(command)
        ),
        None => format!("sudo sh -c {}", shell_escape(command)),
    }
}

async fn exec_sudo_command(
    session: &client::Handle<SshClientHandler>,
    command: &str,
    password: Option<&str>,
) -> Result<(u32, String, String), SshError> {
    exec_command(session, &build_sudo_cmd(command, password)).await
}

/// Execute a command with sudo. If sudo fails for any reason (not installed, not in sudoers,
/// password required, etc.), automatically retry the command without sudo.
///
/// This ensures the deploy works for both root users (where sudo is unnecessary but harmless),
/// non-root users with passwordless sudo, non-root users connecting without a password where
/// sudo prompts would otherwise hang, and machines where sudo is not installed at all.
/// The only cost of a sudo failure + retry is one additional SSH round trip.
async fn exec_with_sudo_fallback(
    session: &client::Handle<SshClientHandler>,
    command: &str,
    password: Option<&str>,
) -> Result<(u32, String, String), SshError> {
    let (exit_code, stdout, stderr) = exec_sudo_command(session, command, password).await?;
    resolve_fallback(exit_code, stdout, stderr, || exec_command(session, command)).await
}

/// If the command under sudo exited non-zero, retry the same command without sudo.
///
/// The common causes are: sudo not installed (exit 127), user not in sudoers, or
/// sudo requiring a password when none was provided.  In all these cases the command
/// should be re-run without sudo so that the actual command result (success or a
/// meaningful error) is what the caller sees.
async fn resolve_fallback<F, Fut>(
    exit_code: u32,
    stdout: String,
    stderr: String,
    fallback: F,
) -> Result<(u32, String, String), SshError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(u32, String, String), SshError>>,
{
    if exit_code == 0 {
        Ok((exit_code, stdout, stderr))
    } else {
        fallback().await
    }
}

fn build_write_unit_cmd(content: &str, path: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    format!("echo {encoded} | base64 -d > {}", shell_escape(path))
}

pub(crate) fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn inject_env_vars(content: &str, server_url: &str, token: &str) -> String {
    let result = replace_or_insert_environment(content, "BORG_SERVER_URL", server_url);
    replace_or_insert_environment(&result, "BORG_AGENT_TOKEN", token)
}

#[allow(
    unknown_lints,
    reason = "no_string_control_flow is a workspace-local dylint lint, unknown to plain \
              rustc/clippy"
)]
#[allow(
    no_string_control_flow,
    reason = "\"[Service]\" is a systemd unit-file section header token, not domain state"
)]
fn replace_or_insert_environment(content: &str, key: &str, value: &str) -> String {
    let assignment = format!("{key}=");
    let mut replaced = false;
    let mut lines = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            let environment = trimmed
                .strip_prefix("Environment=")
                .map_or(trimmed, |value| value.trim_matches('"'));

            if environment.starts_with(&assignment) {
                replaced = true;
                format!("Environment={key}={value}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>();

    if !replaced
        && let Some(service_index) = lines.iter().position(|line| line.trim() == "[Service]")
    {
        lines.insert(
            service_index.saturating_add(1),
            format!("Environment={key}={value}"),
        );
    }

    let mut result = lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Canonical CPU architecture names used to pick the right agent binary.
/// `Other` passes unrecognized `uname -m` output through unchanged.
enum Architecture {
    X86_64,
    Aarch64,
    Armv7,
    Other(String),
}

impl From<&str> for Architecture {
    fn from(raw: &str) -> Self {
        match raw {
            "x86_64" => Self::X86_64,
            "aarch64" | "arm64" => Self::Aarch64,
            "armv7l" | "armhf" => Self::Armv7,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl std::fmt::Display for Architecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::X86_64 => write!(f, "x86_64"),
            Self::Aarch64 => write!(f, "aarch64"),
            Self::Armv7 => write!(f, "armv7"),
            Self::Other(raw) => write!(f, "{raw}"),
        }
    }
}

fn canonical_arch(raw: &str) -> String {
    Architecture::from(raw.trim()).to_string()
}

async fn detect_remote_arch(
    session: &client::Handle<SshClientHandler>,
) -> Result<String, SshError> {
    let (exit_code, stdout, stderr) = exec_command(session, "uname -m").await?;
    if exit_code != 0 {
        return Err(SshError::Exec(format!(
            "uname -m failed (exit {exit_code}): {stderr}"
        )));
    }
    Ok(canonical_arch(&stdout))
}

pub(crate) async fn list_agent_binaries(dir: &std::path::Path) -> Vec<String> {
    let mut available = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("agent-") {
                available.push(name);
            }
        }
    }
    available
}

/// # Errors
///
/// Returns an error if:
/// - [`SshError::Io`]: an I/O error occurs over the SSH connection
/// - [`SshError::Sftp`]: the SFTP operation fails
/// - [`SshError::Exec`]: the operation fails
pub async fn deploy_agent(params: &DeployAgentParams<'_>) -> Result<(), SshError> {
    let session = match params.password {
        Some(pw) => connect_with_password(params.host, params.user, params.port, pw).await?,
        None => connect_with_key(params.host, params.user, params.port, None).await?,
    };
    let sftp = open_sftp(&session).await?;

    let arch = detect_remote_arch(&session).await?;
    let binary_path = params.binary_dir.join(format!("agent-{arch}"));

    if !tokio::fs::try_exists(&binary_path).await.unwrap_or(false) {
        let available = list_agent_binaries(params.binary_dir).await;
        return Err(SshError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "no agent binary found for arch {arch} in {}; available binaries: {available:?}",
                params.binary_dir.display()
            ),
        )));
    }

    let binary_data = tokio::fs::read(&binary_path).await.map_err(|e| {
        SshError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("failed to read agent binary: {e}"),
        ))
    })?;

    // Always upload to /tmp first - SFTP write to the final path often fails due to
    // missing directories or permission issues (SFTP reports these as "No such file").
    let upload_path = format!("/tmp/assimilate-agent-{}", std::process::id());

    sftp.create(upload_path.clone())
        .await
        .map_err(|e| SshError::Sftp(format!("failed to upload agent binary: {e}")))?
        .write_all(&binary_data)
        .await
        .map_err(|e| SshError::Sftp(format!("failed to upload agent binary: {e}")))?;

    let escaped_remote = shell_escape(params.remote_path);
    let move_cmd = format!(
        "mv {} {escaped_remote} && chmod +x {escaped_remote}",
        shell_escape(&upload_path),
    );

    let (exit_code, _, stderr) =
        exec_with_sudo_fallback(&session, &move_cmd, params.password).await?;
    if exit_code != 0 {
        return Err(SshError::Exec(format!(
            "mv/chmod failed (exit {exit_code}): {stderr}"
        )));
    }

    let unit_content = params.systemd_service_content.map_or_else(
        || default_unit_content(params.remote_path, params.server_url, params.token),
        |custom| inject_env_vars(custom, params.server_url, params.token),
    );

    let unit_path = "/etc/systemd/system/assimilate-agent.service";
    let write_cmd = build_write_unit_cmd(&unit_content, unit_path);

    let (exit_code, _, stderr) =
        exec_with_sudo_fallback(&session, &write_cmd, params.password).await?;
    if exit_code != 0 {
        return Err(SshError::Exec(format!(
            "failed to write systemd unit (exit {exit_code}): {stderr}"
        )));
    }

    let enable_cmd = "systemctl daemon-reload && systemctl enable assimilate-agent && systemctl \
                      restart assimilate-agent";
    let (exit_code, _, stderr) =
        exec_with_sudo_fallback(&session, enable_cmd, params.password).await?;
    if exit_code != 0 {
        return Err(SshError::Exec(format!(
            "systemctl enable/restart failed (exit {exit_code}): {stderr}"
        )));
    }

    info!(host = %params.host, "agent deployed and service restarted");
    Ok(())
}

pub struct ReadFileParams<'a> {
    pub host: &'a str,
    pub user: &'a str,
    pub port: u16,
    pub password: Option<&'a str>,
    pub path: &'a str,
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn read_remote_file(params: &ReadFileParams<'_>) -> Result<Option<String>, SshError> {
    let session = match params.password {
        Some(pw) => connect_with_password(params.host, params.user, params.port, pw).await?,
        None => connect_with_key(params.host, params.user, params.port, None).await?,
    };

    let cat_cmd = format!("cat {}", shell_escape(params.path));
    let (exit_code, stdout, _stderr) =
        exec_with_sudo_fallback(&session, &cat_cmd, params.password).await?;

    if exit_code != 0 {
        return Ok(None);
    }

    Ok(Some(stdout))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup/assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_plain_string() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_string_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn canonical_arch_maps_known_values() {
        assert_eq!(canonical_arch("x86_64"), "x86_64");
        assert_eq!(canonical_arch("aarch64"), "aarch64");
        assert_eq!(canonical_arch("arm64"), "aarch64");
        assert_eq!(canonical_arch("armv7l"), "armv7");
        assert_eq!(canonical_arch("armhf"), "armv7");
    }

    #[test]
    fn canonical_arch_passes_through_unknown() {
        assert_eq!(canonical_arch("riscv64"), "riscv64");
    }

    #[test]
    fn canonical_arch_trims_whitespace() {
        assert_eq!(canonical_arch("x86_64\n"), "x86_64");
        assert_eq!(canonical_arch("  aarch64  "), "aarch64");
    }

    #[test]
    fn shell_escape_string_with_single_quote() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_escape_empty_string() {
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn shell_escape_simple_path() {
        assert_eq!(shell_escape("/data/repo"), "'/data/repo'");
    }

    #[test]
    fn shell_escape_path_with_spaces() {
        assert_eq!(shell_escape("/my repo/x"), "'/my repo/x'");
    }

    #[test]
    fn shell_escape_path_with_single_quote() {
        assert_eq!(shell_escape("a'b"), "'a'\\''b'");
    }

    #[test]
    fn build_write_unit_cmd_escapes_path_with_special_chars() {
        let cmd = build_write_unit_cmd("unit", "/etc/sys d/it's.service");
        assert!(
            cmd.ends_with("> '/etc/sys d/it'\\''s.service'"),
            "path was not shell-escaped: {cmd}"
        );
    }

    #[test]
    fn inject_env_vars_replaces_placeholders() {
        let content = concat!(
            "[Service]\n",
            "BORG_SERVER_URL=<will be set automatically>\n",
            "BORG_AGENT_TOKEN=<will be set automatically>\n",
        );
        let result = inject_env_vars(content, "https://example.com", "mytoken");
        assert!(result.contains("BORG_SERVER_URL=https://example.com"));
        assert!(result.contains("BORG_AGENT_TOKEN=mytoken"));
    }

    #[test]
    fn inject_env_vars_injects_missing_vars() {
        let content = "[Service]\nExecStart=/usr/local/bin/agent\n";
        let result = inject_env_vars(content, "https://example.com", "mytoken");
        assert!(result.contains("Environment=BORG_SERVER_URL=https://example.com"));
        assert!(result.contains("Environment=BORG_AGENT_TOKEN=mytoken"));
    }

    #[test]
    fn inject_env_vars_refreshes_existing_vars() {
        let content = concat!(
            "[Service]\n",
            "Environment=BORG_SERVER_URL=https://other.com\n",
            "Environment=\"BORG_AGENT_TOKEN=othertoken\"\n",
        );
        let result = inject_env_vars(content, "https://example.com", "mytoken");
        assert!(result.contains("Environment=BORG_SERVER_URL=https://example.com"));
        assert!(result.contains("Environment=BORG_AGENT_TOKEN=mytoken"));
        assert!(!result.contains("https://other.com"));
        assert!(!result.contains("othertoken"));
    }

    #[test]
    fn build_sudo_cmd_no_password() {
        let cmd = build_sudo_cmd("echo hello", None);
        assert_eq!(cmd, "sudo sh -c 'echo hello'");
    }

    #[test]
    fn build_sudo_cmd_with_password() {
        let cmd = build_sudo_cmd("echo hello", Some("mypass"));
        assert_eq!(cmd, "echo 'mypass' | sudo -S sh -c 'echo hello'");
    }

    #[test]
    fn build_sudo_cmd_with_special_chars_in_password() {
        let cmd = build_sudo_cmd("echo test", Some("pa$$word ' quote"));
        assert!(cmd.starts_with("echo 'pa$$word '\\'' quote'"));
        assert!(cmd.contains("sudo -S sh -c 'echo test'"));
    }

    #[test]
    fn build_sudo_cmd_with_special_chars_in_command() {
        let cmd = build_sudo_cmd("cat /etc/sys d/it's.service", None);
        assert_eq!(cmd, "sudo sh -c 'cat /etc/sys d/it'\\''s.service'");
    }

    #[tokio::test]
    async fn resolve_fallback_calls_fallback_on_exit_127() {
        let result = resolve_fallback(127, String::new(), String::new(), || async {
            Ok((0, "fallback-executed".to_string(), String::new()))
        })
        .await;
        let (code, stdout, _) = result.unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "fallback-executed");
    }

    #[tokio::test]
    async fn resolve_fallback_returns_original_on_exit_0() {
        let result = resolve_fallback(0, "sudo-ok".to_string(), String::new(), || async {
            panic!("fallback must not be called for exit code 0")
        })
        .await;
        let (code, stdout, _) = result.unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "sudo-ok");
    }

    #[tokio::test]
    async fn resolve_fallback_preserves_stderr_and_stdout_on_success() {
        let result = resolve_fallback(0, "output".to_string(), "error-msg".to_string(), || async {
            panic!("fallback must not be called for exit code 0")
        })
        .await;
        let (code, stdout, stderr) = result.unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "output");
        assert_eq!(stderr, "error-msg");
    }

    #[tokio::test]
    async fn resolve_fallback_retries_on_sudo_auth_failure() {
        let result = resolve_fallback(
            1,
            String::new(),
            "sudo: a password is required".to_string(),
            || async { Ok((0, "ran-without-sudo".to_string(), String::new())) },
        )
        .await;
        let (code, stdout, _) = result.unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout, "ran-without-sudo");
    }

    #[tokio::test]
    async fn resolve_fallback_propagates_fallback_error() {
        let err = resolve_fallback(127, String::new(), String::new(), || async {
            Err(SshError::Exec("fallback failed".to_string()))
        })
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("fallback failed"),
            "error should propagate from fallback"
        );
    }

    #[test]
    fn default_unit_content_contains_exec_and_env() {
        let content = default_unit_content("/usr/local/bin/agent", "https://example.com", "tok");
        assert!(content.contains("ExecStart=/usr/local/bin/agent"));
        assert!(content.contains("BORG_SERVER_URL=https://example.com"));
        assert!(content.contains("BORG_AGENT_TOKEN=tok"));
        assert!(content.contains("[Unit]"));
        assert!(content.contains("[Service]"));
        assert!(content.contains("[Install]"));
    }

    #[test]
    fn build_write_unit_cmd_produces_valid_shell_command() {
        let unit = default_unit_content("/usr/local/bin/agent", "https://example.com", "tok123");
        let out_path = "/tmp/assimilate-test-unit.service";
        let cmd = build_write_unit_cmd(&unit, out_path);

        assert!(cmd.starts_with("echo "));
        assert!(cmd.contains("| base64 -d > "));
        assert!(cmd.contains(out_path));
    }

    #[test]
    fn build_write_unit_cmd_roundtrips_via_shell() {
        let unit = default_unit_content("/usr/local/bin/agent", "https://example.com", "tok123");
        let tmp = std::env::temp_dir().join("assimilate-test-unit-nosudo.service");
        let cmd = build_write_unit_cmd(&unit, tmp.to_str().unwrap());

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "shell command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let written = std::fs::read_to_string(&tmp).unwrap();
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(written, unit);
    }

    #[test]
    fn build_write_unit_cmd_roundtrips_via_sudo_shell_escape() {
        let unit = default_unit_content("/usr/local/bin/agent", "https://srv.io", "secret-tok");
        let tmp = std::env::temp_dir().join("assimilate-test-unit-sudo.service");
        let cmd = build_write_unit_cmd(&unit, tmp.to_str().unwrap());

        let escaped = shell_escape(&cmd);
        let sudo_sim = format!("sh -c {escaped}");

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&sudo_sim)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "sudo-style shell command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let written = std::fs::read_to_string(&tmp).unwrap();
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(written, unit);
    }

    #[test]
    fn build_write_unit_cmd_handles_custom_content_with_special_chars() {
        let custom = "[Unit]\nDescription=Test's \"special\" $VARS & \
                      more\n\n[Service]\nExecStart=/bin/true\n\n[Install]\nWantedBy=multi-user.\
                      target\n";
        let tmp = std::env::temp_dir().join("assimilate-test-unit-special.service");
        let cmd = build_write_unit_cmd(custom, tmp.to_str().unwrap());

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output()
            .unwrap();
        assert!(output.status.success());
        let written = std::fs::read_to_string(&tmp).unwrap();
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(written, custom);

        let cmd = build_write_unit_cmd(custom, tmp.to_str().unwrap());
        let escaped = shell_escape(&cmd);
        let sudo_sim = format!("sh -c {escaped}");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&sudo_sim)
            .output()
            .unwrap();
        assert!(output.status.success());
        let written = std::fs::read_to_string(&tmp).unwrap();
        std::fs::remove_file(&tmp).unwrap();
        assert_eq!(written, custom);
    }

    #[tokio::test]
    async fn list_agent_binaries_filters_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent-x86_64"), b"").unwrap();
        std::fs::write(dir.path().join("agent-aarch64"), b"").unwrap();
        std::fs::write(dir.path().join("README.md"), b"").unwrap();

        let mut found = list_agent_binaries(dir.path()).await;
        found.sort();
        assert_eq!(found, vec!["agent-aarch64", "agent-x86_64"]);
    }

    #[tokio::test]
    async fn list_agent_binaries_empty_for_missing_dir() {
        let found = list_agent_binaries(std::path::Path::new("/no-such-dir-assimilate-test")).await;
        assert!(found.is_empty());
    }

    // Combined into one test: both helpers read from SSH_KEY_DIR, mutating the
    // shared env var races if split across parallel tests.
    #[tokio::test]
    async fn ssh_key_helpers_read_from_ssh_key_dir() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("SSH_KEY_DIR", dir.path()) };

        assert!(matches!(
            read_server_public_key().await,
            Err(SshError::PublicKeyNotFound(_))
        ));
        assert!(matches!(
            load_server_private_key().await,
            Err(SshError::PublicKeyNotFound(_))
        ));

        let private_key = ssh_key::PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )
        .unwrap();
        let private_pem = private_key.to_openssh(ssh_key::LineEnding::LF).unwrap();
        std::fs::write(dir.path().join("id_ed25519"), private_pem.as_bytes()).unwrap();
        let public_str = private_key.public_key().to_openssh().unwrap();
        let public_line = format!("{public_str} assimilate-test");
        std::fs::write(
            dir.path().join("id_ed25519.pub"),
            format!("{public_line}\n"),
        )
        .unwrap();

        assert_eq!(read_server_public_key().await.unwrap(), public_line);
        assert!(load_server_private_key().await.is_ok());

        unsafe { std::env::remove_var("SSH_KEY_DIR") };
    }
}

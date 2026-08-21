// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use russh::{
    Channel, client,
    keys::{PrivateKeyWithHashAlg, PublicKey},
};
use shared::protocol::{ServerToUi, TunnelStatus};
use sqlx::PgPool;
use tokio::{
    io::copy_bidirectional,
    net::TcpStream,
    sync::{Notify, RwLock},
};
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use crate::{db, ws::ui_broadcast::UiBroadcast};

/// Replace an unspecified IP (0.0.0.0 / ::) with the loopback address.
#[must_use]
pub fn tunnel_target_addr(bind_addr: SocketAddr) -> SocketAddr {
    if !bind_addr.ip().is_unspecified() {
        return bind_addr;
    }

    let ip = match bind_addr.ip() {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    SocketAddr::new(ip, bind_addr.port())
}

/// Handles SSH channel open requests for forwarded TCP/IP connections in a reverse tunnel.
pub struct TunnelSshHandler {
    /// Address to forward connections to on the server side.
    pub server_addr: SocketAddr,
    /// Broadcast channel for UI status updates.
    pub ui_broadcast: UiBroadcast,
    /// Agent this tunnel belongs to.
    pub agent_id: i64,
    /// Expected SSH host key fingerprint, if pinned.
    pub expected_host_key: Option<String>,
}

impl client::Handler for TunnelSshHandler {
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
            tracing::error!("tunnel SSH host key mismatch: expected {expected}, got {actual}");
            std::future::ready(Ok(false))
        }
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> {
        let server_addr = self.server_addr;

        tokio::spawn(async move {
            let mut tcp = match TcpStream::connect(server_addr).await {
                Ok(s) => s,
                Err(e) => {
                    warn!(
                        addr = %server_addr,
                        "tunnel: failed to connect to local addr: {e}"
                    );
                    return;
                }
            };

            let mut stream = channel.into_stream();
            if let Err(e) = copy_bidirectional(&mut stream, &mut tcp).await {
                error!("tunnel: copy_bidirectional error: {e}");
            }
        });

        std::future::ready(Ok(()))
    }
}

/// SSH client configuration for tunnel connections (no inactivity timeout, 15 s keepalive).
#[must_use]
pub fn tunnel_ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        ..client::Config::default()
    })
}

/// Manages SSH reverse tunnels to agents, including lifecycle, retry, and status tracking.
#[derive(Clone)]
pub struct TunnelManager {
    pool: PgPool,
    ui_broadcast: UiBroadcast,
    server_addr: SocketAddr,
    tunnels: Arc<RwLock<HashMap<i64, TunnelState>>>,
}

#[derive(Clone)]
struct TunnelState {
    agent_id: i64,
    status: TunnelStatus,
    cancel: CancellationToken,
    completion: Arc<Notify>,
}

struct TunnelTaskCompletion(Arc<Notify>);

impl Drop for TunnelTaskCompletion {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

impl TunnelManager {
    /// Create a new tunnel manager with the given pool, broadcast, and server address.
    #[must_use]
    pub fn new(pool: PgPool, ui_broadcast: UiBroadcast, server_addr: SocketAddr) -> Self {
        Self {
            pool,
            ui_broadcast,
            server_addr,
            tunnels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load all enabled tunnels from the database and start them with staggered delays.
    pub async fn run(&self) {
        let tunnels = match db::list_enabled_tunnels(&self.pool).await {
            Ok(t) => t,
            Err(e) => {
                error!("failed to load tunnels: {e}");
                return;
            }
        };
        for tunnel in tunnels {
            let delay_ms = u64::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0u32, |d| d.subsec_nanos())
                    .checked_rem(450)
                    .unwrap_or(0)
                    .saturating_add(50),
            );
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            self.start_tunnel(tunnel.id).await;
        }
    }

    /// Start a reverse tunnel by its ID, stopping any previous instance first.
    pub async fn start_tunnel(&self, tunnel_id: i64) {
        self.stop_tunnel(tunnel_id).await;

        let tunnel = match db::get_tunnel_by_id(&self.pool, tunnel_id).await {
            Ok(t) => t,
            Err(e) => {
                error!(tunnel_id, "failed to load tunnel: {e}");
                return;
            }
        };

        let cancel = CancellationToken::new();
        let completion = Arc::new(Notify::new());
        let hostname = tunnel.ssh_host.clone();

        {
            let mut map = self.tunnels.write().await;
            map.insert(
                tunnel_id,
                TunnelState {
                    agent_id: tunnel.agent_id,
                    status: TunnelStatus::Disconnected,
                    cancel: cancel.clone(),
                    completion: Arc::clone(&completion),
                },
            );
        }

        let manager = self.clone();
        tokio::spawn(run_reconnect_loop(
            manager, tunnel_id, hostname, cancel, completion,
        ));
    }

    /// Stop a running tunnel by its ID, cancelling the task and waiting for it to complete.
    pub async fn stop_tunnel(&self, tunnel_id: i64) {
        // Extract the state in a separate let binding so the write guard is
        // dropped before awaiting completion. Holding it across notified().await
        // would deadlock: the task's cancellation path calls set_status() which
        // also needs the write lock.
        let maybe_state = self.tunnels.write().await.remove(&tunnel_id);
        if let Some(state) = maybe_state {
            state.cancel.cancel();
            state.completion.notified().await;
        }
    }

    /// Return the current tunnel status for the given tunnel ID, if it exists.
    pub async fn tunnel_status(&self, tunnel_id: i64) -> Option<TunnelStatus> {
        self.tunnels
            .read()
            .await
            .get(&tunnel_id)
            .map(|s| s.status.clone())
    }

    /// Return all active tunnel IDs and their current status.
    pub async fn all_statuses(&self) -> Vec<(i64, TunnelStatus)> {
        self.tunnels
            .read()
            .await
            .iter()
            .map(|(id, s)| (*id, s.status.clone()))
            .collect()
    }

    /// Cancel all running tunnels without waiting for them to stop.
    pub async fn shutdown(&self) {
        let tunnels = self.tunnels.read().await;
        for state in tunnels.values() {
            state.cancel.cancel();
        }
    }

    /// Ensures the tunnel for the given agent is started and not in a disconnected/error state.
    /// If it's not running or disconnected, restarts it. Returns `true` if the tunnel is
    /// connected or was just restarted (best-effort).
    pub async fn ensure_agent_tunnel_connected(&self, agent_id: i64) -> bool {
        let Ok(tunnel) = db::get_tunnel_by_agent_id(&self.pool, agent_id).await else {
            return true;
        };

        if !tunnel.enabled {
            return true;
        }

        let needs_restart = {
            let map = self.tunnels.read().await;
            match map.get(&tunnel.id) {
                None => true,
                Some(state) => matches!(
                    state.status,
                    TunnelStatus::Disconnected | TunnelStatus::Error { .. }
                ),
            }
        };

        if needs_restart {
            self.stop_tunnel(tunnel.id).await;
            self.start_tunnel(tunnel.id).await;
        }

        true
    }

    async fn set_status(&self, tunnel_id: i64, hostname: &str, status: TunnelStatus) {
        let agent_id = {
            let mut map = self.tunnels.write().await;
            if let Some(state) = map.get_mut(&tunnel_id) {
                state.status = status.clone();
                Some(state.agent_id)
            } else {
                None
            }
        };

        if let Some(cid) = agent_id {
            self.ui_broadcast.send(ServerToUi::TunnelStatusChanged {
                agent_id: cid,
                hostname: hostname.to_string(),
                status,
            });
        }
    }
}

#[derive(Debug)]
enum ConnectionOutcome {
    Stop,
    Retry(Duration),
}

/// Runs a tunnel's connect/retry loop until `ConnectionOutcome::Stop`.
/// Spawned in the background by `start_tunnel`; factored out of that
/// `tokio::spawn` closure so tests can await it directly instead of racing a
/// detached background task against the test's own completion - previously
/// nothing in the test suite drove this loop at all, so its coverage in CI
/// depended entirely on incidental scheduling from unrelated tests.
async fn run_reconnect_loop(
    manager: TunnelManager,
    tunnel_id: i64,
    hostname: String,
    cancel: CancellationToken,
    completion: Arc<Notify>,
) {
    let _completion = TunnelTaskCompletion(completion);
    let mut backoff = Duration::from_secs(1);
    loop {
        match run_tunnel_connection_attempt(&manager, tunnel_id, &hostname, &cancel, backoff).await
        {
            ConnectionOutcome::Stop => return,
            ConnectionOutcome::Retry(next_backoff) => backoff = next_backoff,
        }
    }
}

struct TunnelConnectionParams {
    tunnel: db::SshTunnel,
    ssh_port: u16,
    tunnel_port: u32,
    key: russh::keys::PrivateKey,
    handler: TunnelSshHandler,
}

/// Loads the current tunnel row and validates/prepares everything needed to
/// attempt a connection (ports, server private key, expected host key).
/// Returns `Err` (after setting the tunnel's status, if applicable) when the
/// tunnel task should stop retrying entirely.
async fn resolve_tunnel_connection_params(
    manager: &TunnelManager,
    tunnel_id: i64,
    hostname: &str,
    cancel: &CancellationToken,
) -> Result<TunnelConnectionParams, ConnectionOutcome> {
    let current = tokio::select! {
        () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
        result = db::get_tunnel_by_id(&manager.pool, tunnel_id) => result,
    };
    let current = match current {
        Ok(t) => t,
        Err(e) => {
            error!(tunnel_id, "tunnel DB lookup failed: {e}");
            return Err(ConnectionOutcome::Stop);
        }
    };

    if !current.enabled {
        manager
            .set_status(tunnel_id, hostname, TunnelStatus::Disconnected)
            .await;
        return Err(ConnectionOutcome::Stop);
    }

    let Ok(ssh_port) = u16::try_from(current.ssh_port) else {
        manager
            .set_status(
                tunnel_id,
                hostname,
                TunnelStatus::Error {
                    message: format!("invalid ssh_port: {}", current.ssh_port),
                },
            )
            .await;
        return Err(ConnectionOutcome::Stop);
    };

    let Ok(tunnel_port) = u32::try_from(current.tunnel_port) else {
        manager
            .set_status(
                tunnel_id,
                hostname,
                TunnelStatus::Error {
                    message: format!("invalid tunnel_port: {}", current.tunnel_port),
                },
            )
            .await;
        return Err(ConnectionOutcome::Stop);
    };

    let key = match crate::ssh::load_server_private_key().await {
        Ok(k) => k,
        Err(e) => {
            manager
                .set_status(
                    tunnel_id,
                    hostname,
                    TunnelStatus::Error {
                        message: e.to_string(),
                    },
                )
                .await;
            return Err(ConnectionOutcome::Stop);
        }
    };

    let scanned_key = match &current.ssh_host_key {
        Some(key) if !key.is_empty() => None,
        _ => match crate::ssh::scan_host_key(&current.ssh_host, ssh_port).await {
            Ok(k) => Some(k),
            Err(e) => {
                warn!(tunnel_id, "failed to scan SSH host key: {e}");
                None
            }
        },
    };
    let expected_host_key = resolve_and_persist_host_key(
        &manager.pool,
        current.id,
        current.ssh_host_key.clone(),
        scanned_key.as_deref(),
    )
    .await;

    let handler = TunnelSshHandler {
        server_addr: manager.server_addr,
        ui_broadcast: manager.ui_broadcast.clone(),
        agent_id: current.agent_id,
        expected_host_key,
    };

    Ok(TunnelConnectionParams {
        tunnel: current,
        ssh_port,
        tunnel_port,
        key,
        handler,
    })
}

/// Connects, authenticates, and requests the remote port forward. Returns
/// the established session on success, or the [`ConnectionOutcome`] the
/// caller should propagate (already having slept for backoff, on the retry
/// path) otherwise.
async fn connect_and_forward(
    manager: &TunnelManager,
    tunnel_id: i64,
    hostname: &str,
    cancel: &CancellationToken,
    params: TunnelConnectionParams,
    backoff: Duration,
) -> Result<client::Handle<TunnelSshHandler>, ConnectionOutcome> {
    let TunnelConnectionParams {
        tunnel: current,
        ssh_port,
        tunnel_port,
        key,
        handler,
    } = params;

    let next_backoff = backoff.saturating_mul(2).min(Duration::from_mins(2));

    let session = tokio::select! {
        () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
        result = client::connect(
            tunnel_ssh_config(),
            (current.ssh_host.as_str(), ssh_port),
            handler,
        ) => result,
    };
    let mut session = match session {
        Ok(s) => s,
        Err(e) => {
            warn!(tunnel_id, "tunnel connect failed: {e}");
            manager
                .set_status(tunnel_id, hostname, TunnelStatus::Reconnecting)
                .await;
            tokio::select! {
                () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
                () = tokio::time::sleep(backoff) => {}
            }
            return Err(ConnectionOutcome::Retry(next_backoff));
        }
    };

    let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
    let auth = tokio::select! {
        () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
        result = session.authenticate_publickey(&current.ssh_user, key_with_alg) => result,
    };
    let auth = match auth {
        Ok(a) => a,
        Err(e) => {
            warn!(tunnel_id, "tunnel auth error: {e}");
            manager
                .set_status(
                    tunnel_id,
                    hostname,
                    TunnelStatus::Error {
                        message: format!("auth error: {e}"),
                    },
                )
                .await;
            return Err(ConnectionOutcome::Stop);
        }
    };

    if !auth.success() {
        manager
            .set_status(
                tunnel_id,
                hostname,
                TunnelStatus::Error {
                    message: "public key authentication rejected".to_string(),
                },
            )
            .await;
        return Err(ConnectionOutcome::Stop);
    }

    let forward = tokio::select! {
        () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
        result = session.tcpip_forward("127.0.0.1", tunnel_port) => result,
    };
    match forward {
        Ok(_bound_port) => {
            manager
                .set_status(tunnel_id, hostname, TunnelStatus::Connected)
                .await;
            Ok(session)
        }
        Err(e) => {
            warn!(tunnel_id, "tcpip_forward failed: {e}");
            manager
                .set_status(tunnel_id, hostname, TunnelStatus::Reconnecting)
                .await;
            tokio::select! {
                () = cancel.cancelled() => return Err(ConnectionOutcome::Stop),
                () = tokio::time::sleep(backoff) => {}
            }
            Err(ConnectionOutcome::Retry(next_backoff))
        }
    }
}

/// Runs one connect-authenticate-forward-and-wait cycle for a tunnel.
/// Returns whether the caller's retry loop should stop entirely or retry
/// after the returned backoff.
async fn run_tunnel_connection_attempt(
    manager: &TunnelManager,
    tunnel_id: i64,
    hostname: &str,
    cancel: &CancellationToken,
    backoff: Duration,
) -> ConnectionOutcome {
    let params = match resolve_tunnel_connection_params(manager, tunnel_id, hostname, cancel).await
    {
        Ok(p) => p,
        Err(outcome) => return outcome,
    };

    let session =
        match connect_and_forward(manager, tunnel_id, hostname, cancel, params, backoff).await {
            Ok(s) => s,
            Err(outcome) => return outcome,
        };

    run_connected_session(manager, tunnel_id, hostname, cancel, session, backoff).await
}

/// Waits for a connected tunnel session to end - either the caller cancels
/// it, or the periodic liveness check finds it closed - then decides
/// whether the reconnect loop should stop entirely or retry after backoff.
/// Split out of `run_tunnel_connection_attempt` so it can be exercised
/// directly against a local test SSH server: reaching this code otherwise
/// requires a real successful SSH connect-authenticate-forward cycle, which
/// nothing in the test suite ever drove, so its coverage in CI depended
/// entirely on incidental scheduling from unrelated tests.
async fn run_connected_session(
    manager: &TunnelManager,
    tunnel_id: i64,
    hostname: &str,
    cancel: &CancellationToken,
    session: client::Handle<TunnelSshHandler>,
    backoff: Duration,
) -> ConnectionOutcome {
    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                if let Ok(Err(e)) = tokio::time::timeout(
                    Duration::from_secs(2),
                    session.disconnect(russh::Disconnect::ByApplication, "", "en"),
                )
                .await
                {
                    tracing::debug!(error = %e, "tunnel disconnect failed");
                }
                manager
                    .set_status(tunnel_id, hostname, TunnelStatus::Disconnected)
                    .await;
                return ConnectionOutcome::Stop;
            }
            () = tokio::time::sleep(Duration::from_secs(5)) => {
                if session.is_closed() {
                    break;
                }
            }
        }
    }

    manager
        .set_status(tunnel_id, hostname, TunnelStatus::Reconnecting)
        .await;

    tokio::select! {
        () = cancel.cancelled() => {
            manager
                .set_status(tunnel_id, hostname, TunnelStatus::Disconnected)
                .await;
            ConnectionOutcome::Stop
        }
        () = tokio::time::sleep(backoff) => {
            ConnectionOutcome::Retry(backoff.saturating_mul(2).min(Duration::from_mins(2)))
        }
    }
}

/// Resolves the expected SSH host key for a tunnel connection.
///
/// When `existing_key` is `Some` and non-empty it is returned as-is and
/// `scanned_key` is ignored (no DB write).  Otherwise `scanned_key` is
/// persisted via `db::update_tunnel_ssh_host_key` and returned.
/// Returns `None` when neither key is available.
async fn resolve_and_persist_host_key(
    pool: &sqlx::PgPool,
    tunnel_id: i64,
    existing_key: Option<String>,
    scanned_key: Option<&str>,
) -> Option<String> {
    match existing_key {
        Some(key) if !key.is_empty() => Some(key),
        _ => {
            let scanned = scanned_key?;
            if let Err(e) = db::update_tunnel_ssh_host_key(pool, tunnel_id, scanned).await {
                error!(tunnel_id, "failed to persist scanned SSH host key: {e}");
            }
            Some(scanned.to_owned())
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup/assertions"
)]
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use russh::{client::Handler, keys::PublicKey, server::Server as _};
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::{
        ConnectionOutcome, TunnelConnectionParams, TunnelManager, TunnelState,
        TunnelTaskCompletion, connect_and_forward, resolve_and_persist_host_key,
        run_connected_session, run_reconnect_loop, tunnel_ssh_config, tunnel_target_addr,
    };
    use crate::{db, ws::ui_broadcast::UiBroadcast};

    fn dummy_manager() -> TunnelManager {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let ui = UiBroadcast::new();
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        TunnelManager::new(pool, ui, addr)
    }

    #[test]
    fn tunnel_ssh_config_no_inactivity_timeout() {
        let config = tunnel_ssh_config();
        assert_eq!(config.inactivity_timeout, None);
    }

    #[test]
    fn tunnel_ssh_config_keepalive() {
        let config = tunnel_ssh_config();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
    }

    #[test]
    fn tunnel_ssh_config_keepalive_max() {
        let config = tunnel_ssh_config();
        assert_eq!(config.keepalive_max, 3);
    }

    #[test]
    fn tunnel_target_uses_ipv4_loopback_for_wildcard_bind() {
        let bind_addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let expected: SocketAddr = "127.0.0.1:8080".parse().unwrap();

        assert_eq!(tunnel_target_addr(bind_addr), expected);
    }

    #[test]
    fn tunnel_target_uses_ipv6_loopback_for_wildcard_bind() {
        let bind_addr: SocketAddr = "[::]:8080".parse().unwrap();
        let expected: SocketAddr = "[::1]:8080".parse().unwrap();

        assert_eq!(tunnel_target_addr(bind_addr), expected);
    }

    #[test]
    fn tunnel_target_preserves_specific_bind_address() {
        let bind_addr: SocketAddr = "192.0.2.10:8080".parse().unwrap();

        assert_eq!(tunnel_target_addr(bind_addr), bind_addr);
    }

    #[tokio::test]
    async fn tunnel_manager_new_creates_empty_map() {
        let mgr = dummy_manager();
        let statuses = mgr.all_statuses().await;
        assert_eq!(statuses.len(), 0);
    }

    #[tokio::test]
    async fn stop_nonexistent_tunnel_is_no_op() {
        let mgr = dummy_manager();
        mgr.stop_tunnel(999).await;
        let statuses = mgr.all_statuses().await;
        assert_eq!(statuses.len(), 0);
    }

    #[tokio::test]
    async fn stop_tunnel_waits_for_task_completion() {
        let mgr = dummy_manager();
        let cancel = CancellationToken::new();
        let completion = Arc::new(Notify::new());
        let task_finished = Arc::new(AtomicBool::new(false));

        mgr.tunnels.write().await.insert(
            1,
            TunnelState {
                agent_id: 2,
                status: shared::protocol::TunnelStatus::Connected,
                cancel: cancel.clone(),
                completion: Arc::clone(&completion),
            },
        );

        tokio::spawn({
            let task_finished = Arc::clone(&task_finished);
            async move {
                let _completion = TunnelTaskCompletion(completion);
                cancel.cancelled().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
                task_finished.store(true, Ordering::SeqCst);
            }
        });

        mgr.stop_tunnel(1).await;

        assert!(task_finished.load(Ordering::SeqCst));
        assert!(mgr.tunnel_status(1).await.is_none());
    }

    fn check_server_key_sync(
        expected_host_key: Option<String>,
        public_key: &PublicKey,
    ) -> Result<bool, russh::Error> {
        let addr: SocketAddr = "127.0.0.1:2222".parse().unwrap();
        let mut handler = super::TunnelSshHandler {
            server_addr: addr,
            ui_broadcast: crate::ws::ui_broadcast::UiBroadcast::new(),
            agent_id: 1,
            expected_host_key,
        };

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.check_server_key(public_key))
    }

    #[test]
    fn ssh_handler_accepts_when_keys_match() {
        let key_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAINwxkbeQjd0zydveueMhRPJE+cxoP0DNuUcYAwqmOs6S";
        let public = russh::keys::parse_public_key_base64(key_b64).unwrap();
        let expected = public.to_openssh().unwrap();

        let result = check_server_key_sync(Some(expected), &public);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn ssh_handler_rejects_when_keys_differ() {
        let key1_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAINwxkbeQjd0zydveueMhRPJE+cxoP0DNuUcYAwqmOs6S";
        let key2_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAIC2A0E0TgtMfIkRqPBL6S1a60f1VMJEbaDsaeS2KJoC8";
        let public1 = russh::keys::parse_public_key_base64(key1_b64).unwrap();
        let public2 = russh::keys::parse_public_key_base64(key2_b64).unwrap();
        let expected = public1.to_openssh().unwrap();

        let result = check_server_key_sync(Some(expected), &public2);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn ssh_handler_accepts_when_no_expected_key() {
        let key_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAINwxkbeQjd0zydveueMhRPJE+cxoP0DNuUcYAwqmOs6S";
        let public = russh::keys::parse_public_key_base64(key_b64).unwrap();

        let result = check_server_key_sync(None, &public);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// Regression: `stop_tunnel` must release the write lock before awaiting
    /// task completion so that the task's cancellation path (which calls
    /// `set_status` and needs the write lock) can proceed without deadlocking.
    #[tokio::test]
    async fn stop_tunnel_does_not_deadlock_when_task_acquires_write_lock_on_cancel() {
        let mgr = dummy_manager();
        let cancel = CancellationToken::new();
        let completion = Arc::new(Notify::new());

        mgr.tunnels.write().await.insert(
            42,
            TunnelState {
                agent_id: 7,
                status: shared::protocol::TunnelStatus::Connected,
                cancel: cancel.clone(),
                completion: Arc::clone(&completion),
            },
        );

        let mgr2 = mgr.clone();
        tokio::spawn(async move {
            let _completion = TunnelTaskCompletion(completion);
            cancel.cancelled().await;
            // Simulate what the connected-tunnel cancel path does: acquire the
            // write lock to update status. This would deadlock if stop_tunnel
            // were still holding the write lock here.
            mgr2.tunnels.write().await.remove(&42);
        });

        // Must complete without deadlocking (use a timeout to catch regressions).
        tokio::time::timeout(Duration::from_secs(5), mgr.stop_tunnel(42))
            .await
            .expect("stop_tunnel deadlocked while holding the write lock");
    }

    #[tokio::test]
    async fn resolve_returns_existing_key_when_present() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let result =
            resolve_and_persist_host_key(&pool, 1, Some("ssh-ed25519 AAAA".to_string()), None)
                .await;
        assert_eq!(result, Some("ssh-ed25519 AAAA".to_string()));
    }

    #[tokio::test]
    async fn resolve_returns_scanned_key_when_no_existing() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let scanned = "ssh-ed25519 AABB";
        let result = resolve_and_persist_host_key(&pool, 1, None, Some(scanned)).await;
        assert_eq!(result, Some(scanned.to_string()));
    }

    #[tokio::test]
    async fn resolve_returns_scanned_key_when_existing_is_empty() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let scanned = "ssh-ed25519 AACC";
        let result =
            resolve_and_persist_host_key(&pool, 1, Some(String::new()), Some(scanned)).await;
        assert_eq!(result, Some(scanned.to_string()));
    }

    #[tokio::test]
    async fn resolve_returns_none_when_no_keys() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let result = resolve_and_persist_host_key(&pool, 1, None, None).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn resolve_ignores_scanned_key_when_existing_present() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let result = resolve_and_persist_host_key(
            &pool,
            1,
            Some("ssh-ed25519 EXISTING".to_string()),
            Some("ssh-ed25519 SCANNED"),
        )
        .await;
        assert_eq!(result, Some("ssh-ed25519 EXISTING".to_string()));
    }

    /// Regression: `run_reconnect_loop`'s outer `match` (the `Stop => return` /
    /// `Retry => backoff = ...` arms) previously had no dedicated test - it was
    /// only ever covered by chance, when some other test's leftover
    /// `start_tunnel`-spawned background task happened to still be scheduled
    /// when the coverage-instrumented process exited. That produced
    /// non-deterministic coverage on this loop (see #440). `dummy_manager()`'s
    /// pool points at a nonexistent database, so the very first
    /// `run_tunnel_connection_attempt` call fails its DB lookup and returns
    /// `ConnectionOutcome::Stop` on the first iteration, deterministically.
    #[tokio::test]
    async fn reconnect_loop_stops_immediately_when_tunnel_lookup_fails() {
        let mgr = dummy_manager();
        let cancel = CancellationToken::new();
        let completion = Arc::new(Notify::new());

        tokio::time::timeout(
            Duration::from_secs(5),
            run_reconnect_loop(mgr, 999, "unreachable-host".to_string(), cancel, completion),
        )
        .await
        .expect("reconnect loop should stop immediately on DB lookup failure");
    }

    /// Minimal local SSH server accepting any public key and granting any
    /// `tcpip_forward` request, so `connect_and_forward`/`run_connected_session`
    /// can be exercised against a real SSH session instead of only through
    /// incidental background-task scheduling during unrelated tests (see
    /// #440's coverage-diff investigation for how that showed up as
    /// non-deterministic coverage on these functions).
    struct AcceptAllHandler;

    impl russh::server::Handler for AcceptAllHandler {
        type Error = russh::Error;

        fn auth_publickey(
            &mut self,
            _user: &str,
            _public_key: &russh::keys::PublicKey,
        ) -> impl std::future::Future<Output = Result<russh::server::Auth, Self::Error>> {
            std::future::ready(Ok(russh::server::Auth::Accept))
        }

        fn tcpip_forward(
            &mut self,
            _address: &str,
            port: &mut u32,
            _session: &mut russh::server::Session,
        ) -> impl std::future::Future<Output = Result<bool, Self::Error>> {
            if *port == 0 {
                *port = 2222;
            }
            std::future::ready(Ok(true))
        }
    }

    struct AcceptAllServer;

    impl russh::server::Server for AcceptAllServer {
        type Handler = AcceptAllHandler;

        fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
            AcceptAllHandler
        }
    }

    /// Generates a fresh ed25519 key pair. Shared by [`generate_test_key`]
    /// and [`TestSshKeyDir::setup`], which each PEM-encode the result
    /// differently (one round-trips it into a `russh` key, the other writes
    /// it straight to disk) - factoring out the PEM encoding too would mean
    /// naming `to_openssh`'s `Zeroizing<String>` return type, which isn't
    /// reachable without adding the `zeroize` crate as a direct dependency.
    fn generate_ed25519_key() -> ssh_key::PrivateKey {
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .expect("generate test key")
    }

    /// Generates a fresh ed25519 key pair. `russh::keys::PrivateKey` pins its
    /// own `ssh-key`/`rand_core` versions internally (not the crate's direct
    /// `ssh-key` dependency used elsewhere, e.g. in `api::system`), so this
    /// generates with the crate's own `ssh_key::PrivateKey::random` and
    /// round-trips through OpenSSH PEM - the same conversion
    /// `crate::ssh::load_server_private_key` already does when loading a key
    /// from disk.
    fn generate_test_key() -> russh::keys::PrivateKey {
        let pem = generate_ed25519_key()
            .to_openssh(ssh_key::LineEnding::LF)
            .expect("encode test key as OpenSSH PEM");
        russh::keys::decode_secret_key(&pem, None).expect("decode test key")
    }

    /// A running [`AcceptAllServer`] test fixture. Call [`Self::shutdown`] to
    /// deterministically drain it at the end of a test, instead of leaving it
    /// running until the process exits - the same class of non-determinism
    /// this whole set of changes exists to eliminate.
    struct TestSshServer {
        port: u16,
        handle: russh::server::RunningServerHandle,
        join: tokio::task::JoinHandle<()>,
    }

    impl TestSshServer {
        async fn shutdown(self) {
            self.handle.shutdown(String::new());
            tokio::time::timeout(Duration::from_secs(5), self.join)
                .await
                .expect("test SSH server should shut down promptly")
                .expect("test SSH server task should not panic");
        }
    }

    /// Starts [`AcceptAllServer`] on an ephemeral loopback port.
    async fn spawn_test_ssh_server() -> TestSshServer {
        let host_key = generate_test_key();
        let config = Arc::new(russh::server::Config {
            keys: vec![host_key],
            ..Default::default()
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test SSH server");
        let port = listener.local_addr().expect("listener addr").port();

        let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
        let join = tokio::spawn(async move {
            let mut server = AcceptAllServer;
            let running = server.run_on_socket(config, &listener);
            let _ = handle_tx.send(running.handle());
            let _ = running.await;
        });
        let handle = handle_rx.await.expect("test SSH server handle");

        TestSshServer { port, handle, join }
    }

    fn test_tunnel_row(ssh_port: u16) -> crate::db::SshTunnel {
        crate::db::SshTunnel {
            id: 1,
            agent_id: 1,
            ssh_host: "127.0.0.1".to_string(),
            ssh_user: "test".to_string(),
            ssh_port: i32::from(ssh_port),
            tunnel_port: 0,
            ssh_host_key: None,
            enabled: true,
            created_at: chrono::Utc::now(),
        }
    }

    fn test_connection_params(ssh_port: u16) -> TunnelConnectionParams {
        let client_key = generate_test_key();

        TunnelConnectionParams {
            tunnel: test_tunnel_row(ssh_port),
            ssh_port,
            tunnel_port: 0,
            key: client_key,
            handler: super::TunnelSshHandler {
                server_addr: "127.0.0.1:0".parse().unwrap(),
                ui_broadcast: UiBroadcast::new(),
                agent_id: 1,
                expected_host_key: None,
            },
        }
    }

    #[tokio::test]
    async fn connect_and_forward_succeeds_against_local_ssh_server() {
        let server = spawn_test_ssh_server().await;
        let mgr = dummy_manager();
        let cancel = CancellationToken::new();

        let session = tokio::time::timeout(
            Duration::from_secs(5),
            connect_and_forward(
                &mgr,
                1,
                "test-host",
                &cancel,
                test_connection_params(server.port),
                Duration::from_millis(10),
            ),
        )
        .await
        .expect("connect_and_forward should not hang")
        .expect("connect_and_forward should succeed against the local test server");

        drop(session);
        server.shutdown().await;
    }

    /// Regression: reaching `run_connected_session`'s cancellation branch
    /// (disconnect the live session, mark the tunnel disconnected, return
    /// `Stop`) requires a real successful SSH connect-authenticate-forward
    /// cycle, which nothing in the test suite drove before this test existed
    /// - see #440.
    #[tokio::test]
    async fn connected_session_disconnects_and_stops_when_cancelled() {
        let server = spawn_test_ssh_server().await;
        let mgr = dummy_manager();
        let cancel = CancellationToken::new();

        let session = connect_and_forward(
            &mgr,
            1,
            "test-host",
            &cancel,
            test_connection_params(server.port),
            Duration::from_millis(10),
        )
        .await
        .expect("connect_and_forward should succeed against the local test server");

        // Cancel before waiting so the `cancel.cancelled()` arm wins the
        // `tokio::select!` deterministically instead of racing the 5-second
        // liveness-check sleep.
        cancel.cancel();

        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_connected_session(
                &mgr,
                1,
                "test-host",
                &cancel,
                session,
                Duration::from_millis(10),
            ),
        )
        .await
        .expect("run_connected_session should not hang");

        assert!(matches!(outcome, ConnectionOutcome::Stop));
        server.shutdown().await;
    }

    /// Points `SSH_KEY_DIR` at a fresh tempdir containing a generated
    /// `id_ed25519`, so `crate::ssh::load_server_private_key` (called by
    /// `resolve_tunnel_connection_params`, unconditionally, for every
    /// connection attempt) succeeds. Mirrors the established pattern in
    /// `crate::ssh::tests::ssh_key_helpers_read_from_ssh_key_dir`. Combined
    /// with `#[ignore = "requires DATABASE_URL"]`, this never runs in the
    /// same process as that (non-ignored) test, so there's no risk of the
    /// two racing on the shared env var.
    ///
    /// # Safety
    /// `std::env::set_var` is only unsound under concurrent access; every
    /// place this codebase runs `#[ignore = "requires DATABASE_URL"]` tests
    /// (the "Database Integration Tests" CI job, and the coverage job's
    /// `--include-ignored` run) forces `--test-threads=1`.
    struct TestSshKeyDir {
        _dir: tempfile::TempDir,
    }

    impl TestSshKeyDir {
        fn setup() -> Self {
            let dir = tempfile::tempdir().expect("create tempdir");
            unsafe { std::env::set_var("SSH_KEY_DIR", dir.path()) };
            let pem = generate_ed25519_key()
                .to_openssh(ssh_key::LineEnding::LF)
                .expect("encode test server key as OpenSSH PEM");
            std::fs::write(dir.path().join("id_ed25519"), pem.as_bytes())
                .expect("write test server key");
            Self { _dir: dir }
        }
    }

    impl Drop for TestSshKeyDir {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("SSH_KEY_DIR") };
        }
    }

    async fn insert_test_agent(pool: &sqlx::PgPool, hostname: &str) -> i64 {
        sqlx::query_scalar!(
            "INSERT INTO agents (hostname, agent_token_hash) VALUES ($1, 'fakehash') RETURNING id",
            hostname,
        )
        .fetch_one(pool)
        .await
        .expect("insert test agent")
    }

    /// End-to-end regression test for the gap this whole file's changes
    /// close: `start_tunnel`'s own `tokio::spawn(run_reconnect_loop(...))`
    /// call site, and the full `resolve_tunnel_connection_params` ->
    /// `connect_and_forward` -> `run_connected_session` chain it drives, were
    /// never reachable by any test - `TunnelManager::start_tunnel` was only
    /// ever invoked (in production) once an SSH tunnel row existed, and
    /// nothing in the test suite created one with `enabled = true` (see
    /// #440's coverage-diff investigation).
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn start_tunnel_connects_and_stops_against_local_ssh_server(pool: sqlx::PgPool) {
        let _key_dir = TestSshKeyDir::setup();
        let server = spawn_test_ssh_server().await;
        let agent_id = insert_test_agent(&pool, "tunnel-e2e-host").await;
        let tunnel = db::insert_tunnel(
            &pool,
            &db::NewSshTunnel {
                agent_id,
                ssh_host: "127.0.0.1".to_string(),
                ssh_user: "test".to_string(),
                ssh_port: Some(i32::from(server.port)),
                tunnel_port: 0,
                enabled: Some(true),
                ssh_host_key: None,
            },
        )
        .await
        .expect("insert test tunnel");

        let mgr = TunnelManager::new(pool, UiBroadcast::new(), "127.0.0.1:0".parse().unwrap());
        mgr.start_tunnel(tunnel.id).await;

        let connected = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(
                    mgr.tunnel_status(tunnel.id).await,
                    Some(shared::protocol::TunnelStatus::Connected)
                ) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(connected.is_ok(), "tunnel should reach Connected status");

        // Exercises run_connected_session's cancellation branch via the full
        // production call chain (start_tunnel -> run_reconnect_loop ->
        // run_tunnel_connection_attempt -> run_connected_session).
        mgr.stop_tunnel(tunnel.id).await;
        assert!(mgr.tunnel_status(tunnel.id).await.is_none());

        server.shutdown().await;
    }

    /// Regression: `run_reconnect_loop`'s `ConnectionOutcome::Retry` match arm
    /// (`backoff = next_backoff`) was never reachable by any test either,
    /// since it requires a real failed connection attempt followed by the
    /// loop actually running a second iteration.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn start_tunnel_retries_after_connection_refused(pool: sqlx::PgPool) {
        let _key_dir = TestSshKeyDir::setup();
        // Bind and immediately drop a listener: its port is guaranteed to
        // refuse connections for the rest of the test.
        let refused_port = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind throwaway listener");
            listener.local_addr().expect("listener addr").port()
        };
        let agent_id = insert_test_agent(&pool, "tunnel-retry-host").await;
        let tunnel = db::insert_tunnel(
            &pool,
            &db::NewSshTunnel {
                agent_id,
                ssh_host: "127.0.0.1".to_string(),
                ssh_user: "test".to_string(),
                ssh_port: Some(i32::from(refused_port)),
                tunnel_port: 0,
                enabled: Some(true),
                ssh_host_key: None,
            },
        )
        .await
        .expect("insert test tunnel");

        let ui_broadcast = UiBroadcast::new();
        let mut ui_events = ui_broadcast.subscribe();
        let mgr = TunnelManager::new(pool, ui_broadcast, "127.0.0.1:0".parse().unwrap());
        mgr.start_tunnel(tunnel.id).await;

        // The first `Reconnecting` status change comes from the initial
        // failed connect attempt; a second one only happens after
        // `run_reconnect_loop`'s `ConnectionOutcome::Retry` arm re-loops and
        // `connect_and_forward` fails again. Waiting for two proves the
        // Retry arm actually ran, instead of just inferring it from
        // wall-clock timing (which can't tell a broken Retry arm from a
        // slow-but-working one).
        let reconnecting_count = tokio::time::timeout(Duration::from_secs(5), async {
            let mut count = 0u32;
            while count < 2 {
                match ui_events.recv().await {
                    Ok(shared::protocol::ServerToUi::TunnelStatusChanged {
                        status: shared::protocol::TunnelStatus::Reconnecting,
                        ..
                    }) => count = count.saturating_add(1),
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            count
        })
        .await;
        assert_eq!(
            reconnecting_count,
            Ok(2),
            "retry loop should reach Reconnecting twice, proving the Retry arm ran"
        );

        mgr.stop_tunnel(tunnel.id).await;
        assert!(mgr.tunnel_status(tunnel.id).await.is_none());
    }
}

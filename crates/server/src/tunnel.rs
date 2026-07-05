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

pub struct TunnelSshHandler {
    pub server_addr: SocketAddr,
    pub ui_broadcast: UiBroadcast,
    pub agent_id: i64,
    pub expected_host_key: Option<String>,
}

impl client::Handler for TunnelSshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        let Some(expected) = &self.expected_host_key else {
            return Ok(true);
        };
        let actual = server_public_key.to_openssh().unwrap_or_default();
        if actual.trim() == expected.trim() {
            Ok(true)
        } else {
            tracing::error!("tunnel SSH host key mismatch: expected {expected}, got {actual}");
            Ok(false)
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
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

        Ok(())
    }
}

pub fn tunnel_ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        ..client::Config::default()
    })
}

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
    pub fn new(pool: PgPool, ui_broadcast: UiBroadcast, server_addr: SocketAddr) -> Self {
        Self {
            pool,
            ui_broadcast,
            server_addr,
            tunnels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

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
                    % 450
                    + 50,
            );
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            self.start_tunnel(tunnel.id).await;
        }
    }

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
                    completion: completion.clone(),
                },
            );
        }

        let manager = self.clone();
        tokio::spawn(async move {
            let _completion = TunnelTaskCompletion(completion);
            let mut backoff = Duration::from_secs(1);
            loop {
                let current = tokio::select! {
                    () = cancel.cancelled() => return,
                    result = db::get_tunnel_by_id(&manager.pool, tunnel_id) => result,
                };
                let current = match current {
                    Ok(t) => t,
                    Err(e) => {
                        error!(tunnel_id, "tunnel DB lookup failed: {e}");
                        return;
                    }
                };

                if !current.enabled {
                    manager
                        .set_status(tunnel_id, &hostname, TunnelStatus::Disconnected)
                        .await;
                    return;
                }

                let ssh_port = match u16::try_from(current.ssh_port) {
                    Ok(p) => p,
                    Err(_) => {
                        manager
                            .set_status(
                                tunnel_id,
                                &hostname,
                                TunnelStatus::Error {
                                    message: format!("invalid ssh_port: {}", current.ssh_port),
                                },
                            )
                            .await;
                        return;
                    }
                };

                let tunnel_port = match u32::try_from(current.tunnel_port) {
                    Ok(p) => p,
                    Err(_) => {
                        manager
                            .set_status(
                                tunnel_id,
                                &hostname,
                                TunnelStatus::Error {
                                    message: format!(
                                        "invalid tunnel_port: {}",
                                        current.tunnel_port
                                    ),
                                },
                            )
                            .await;
                        return;
                    }
                };

                let key = match crate::ssh::load_server_private_key_async().await {
                    Ok(k) => k,
                    Err(e) => {
                        manager
                            .set_status(
                                tunnel_id,
                                &hostname,
                                TunnelStatus::Error {
                                    message: e.to_string(),
                                },
                            )
                            .await;
                        return;
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

                let session = tokio::select! {
                    () = cancel.cancelled() => return,
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
                            .set_status(tunnel_id, &hostname, TunnelStatus::Reconnecting)
                            .await;
                        tokio::select! {
                            () = cancel.cancelled() => return,
                            () = tokio::time::sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(Duration::from_secs(120));
                        continue;
                    }
                };

                let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), None);
                let auth = tokio::select! {
                    () = cancel.cancelled() => return,
                    result = session.authenticate_publickey(&current.ssh_user, key_with_alg) => {
                        result
                    }
                };
                let auth = match auth {
                    Ok(a) => a,
                    Err(e) => {
                        warn!(tunnel_id, "tunnel auth error: {e}");
                        manager
                            .set_status(
                                tunnel_id,
                                &hostname,
                                TunnelStatus::Error {
                                    message: format!("auth error: {e}"),
                                },
                            )
                            .await;
                        return;
                    }
                };

                if !auth.success() {
                    manager
                        .set_status(
                            tunnel_id,
                            &hostname,
                            TunnelStatus::Error {
                                message: "public key authentication rejected".to_string(),
                            },
                        )
                        .await;
                    return;
                }

                let forward = tokio::select! {
                    () = cancel.cancelled() => return,
                    result = session.tcpip_forward("127.0.0.1", tunnel_port) => result,
                };
                match forward {
                    Ok(_bound_port) => {
                        manager
                            .set_status(tunnel_id, &hostname, TunnelStatus::Connected)
                            .await;
                        backoff = Duration::from_secs(1);
                    }
                    Err(e) => {
                        warn!(tunnel_id, "tcpip_forward failed: {e}");
                        manager
                            .set_status(tunnel_id, &hostname, TunnelStatus::Reconnecting)
                            .await;
                        tokio::select! {
                            () = cancel.cancelled() => return,
                            () = tokio::time::sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(Duration::from_secs(120));
                        continue;
                    }
                }

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
                                .set_status(tunnel_id, &hostname, TunnelStatus::Disconnected)
                                .await;
                            return;
                        }
                        () = tokio::time::sleep(Duration::from_secs(5)) => {
                            if session.is_closed() {
                                break;
                            }
                        }
                    }
                }

                manager
                    .set_status(tunnel_id, &hostname, TunnelStatus::Reconnecting)
                    .await;

                tokio::select! {
                    () = cancel.cancelled() => {
                        manager
                            .set_status(tunnel_id, &hostname, TunnelStatus::Disconnected)
                            .await;
                        return;
                    }
                    () = tokio::time::sleep(backoff) => {}
                }
                backoff = (backoff * 2).min(Duration::from_secs(120));
            }
        });
    }

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

    pub async fn tunnel_status(&self, tunnel_id: i64) -> Option<TunnelStatus> {
        self.tunnels
            .read()
            .await
            .get(&tunnel_id)
            .map(|s| s.status.clone())
    }

    pub async fn all_statuses(&self) -> Vec<(i64, TunnelStatus)> {
        self.tunnels
            .read()
            .await
            .iter()
            .map(|(id, s)| (*id, s.status.clone()))
            .collect()
    }

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
        let tunnel = match db::get_tunnel_by_agent_id(&self.pool, agent_id).await {
            Ok(t) => t,
            Err(_) => return true,
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
mod tests {
    use std::{
        net::SocketAddr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use russh::client::Handler;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::{
        TunnelManager, TunnelState, TunnelTaskCompletion, resolve_and_persist_host_key,
        tunnel_ssh_config, tunnel_target_addr,
    };
    use crate::ws::ui_broadcast::UiBroadcast;

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
        assert!(statuses.is_empty());
    }

    #[tokio::test]
    async fn stop_nonexistent_tunnel_is_no_op() {
        let mgr = dummy_manager();
        mgr.stop_tunnel(999).await;
        let statuses = mgr.all_statuses().await;
        assert!(statuses.is_empty());
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
                completion: completion.clone(),
            },
        );

        tokio::spawn({
            let task_finished = task_finished.clone();
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

    #[test]
    fn ssh_handler_accepts_when_keys_match() {
        let key_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAINwxkbeQjd0zydveueMhRPJE+cxoP0DNuUcYAwqmOs6S";
        let public = russh::keys::parse_public_key_base64(key_b64).unwrap();
        let expected = public.to_openssh().unwrap();

        let addr: SocketAddr = "127.0.0.1:2222".parse().unwrap();
        let mut handler = super::TunnelSshHandler {
            server_addr: addr,
            ui_broadcast: crate::ws::ui_broadcast::UiBroadcast::new(),
            agent_id: 1,
            expected_host_key: Some(expected),
        };

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.check_server_key(&public));
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

        let addr: SocketAddr = "127.0.0.1:2222".parse().unwrap();
        let mut handler = super::TunnelSshHandler {
            server_addr: addr,
            ui_broadcast: crate::ws::ui_broadcast::UiBroadcast::new(),
            agent_id: 1,
            expected_host_key: Some(expected),
        };

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.check_server_key(&public2));
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn ssh_handler_accepts_when_no_expected_key() {
        let key_b64 = "AAAAC3NzaC1lZDI1NTE5AAAAINwxkbeQjd0zydveueMhRPJE+cxoP0DNuUcYAwqmOs6S";
        let public = russh::keys::parse_public_key_base64(key_b64).unwrap();

        let addr: SocketAddr = "127.0.0.1:2222".parse().unwrap();
        let mut handler = super::TunnelSshHandler {
            server_addr: addr,
            ui_broadcast: crate::ws::ui_broadcast::UiBroadcast::new(),
            agent_id: 1,
            expected_host_key: None,
        };

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.check_server_key(&public));
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// Regression: stop_tunnel must release the write lock before awaiting
    /// task completion so that the task's cancellation path (which calls
    /// set_status and needs the write lock) can proceed without deadlocking.
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
                completion: completion.clone(),
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
}

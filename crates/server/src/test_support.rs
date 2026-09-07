// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Test-only helpers shared across this crate's unit test modules.

/// Generates a fresh ed25519 key pair for use in SSH-related tests.
/// `russh::keys::PrivateKey` (used in `tunnel.rs`'s tests) pins its own
/// `ssh-key`/`rand_core` versions internally, distinct from this crate's
/// direct `ssh-key` dependency used here and in `ssh.rs`'s tests - callers
/// that need a `russh` key round-trip this through an OpenSSH PEM instead of
/// converting it directly.
pub(crate) fn generate_ed25519_key() -> ssh_key::PrivateKey {
    ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
        .expect("generate test key")
}

/// An [`AppState`](crate::AppState) around `pool` for unit tests, with every
/// field a test never touches populated with an inert default.
///
/// Shared rather than rebuilt per module: the struct has two dozen fields, so
/// each private copy of this was a near-identical block that the duplicate-code
/// check counts as a cluster - and every new field on `AppState` meant editing
/// all of them. `key_material` is per module so two modules' encrypted
/// passphrases can never be read with one another's key by accident.
pub(crate) fn build_test_state(pool: sqlx::PgPool, key_material: &[u8]) -> crate::AppState {
    let ui_broadcast = crate::ws::ui_broadcast::UiBroadcast::new();
    let tunnel_manager = crate::tunnel::TunnelManager::new(
        pool.clone(),
        ui_broadcast.clone(),
        "127.0.0.1:0".parse().expect("valid socket address"),
    );

    crate::AppState {
        pool: pool.clone(),
        encryption_key: shared::crypto::derive_key(key_material).expect("derive test key"),
        registry: crate::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer: crate::log_buffer::LogBuffer::default(),
        notification_service: crate::notifications::NotificationService::new(pool),
        completion_bus: crate::ws::completion_bus::CompletionBus::new(),
        repo_op_tracker: crate::repo_op_tracker::RepoOpTracker::default(),
        background_task_tracker: crate::background_tasks::BackgroundTaskTracker::default(),
        repo_lock: crate::RepoLock::default(),
        import_tasks: crate::ImportTaskRegistry::default(),
        pending_dryruns: crate::new_pending_map(),
        pending_restores: crate::new_pending_map(),
        pending_vm_scans: crate::new_pending_map(),
        pending_vm_builds: crate::new_pending_map(),
        pending_migrations: crate::new_pending_map(),
        pending_deletes: crate::new_pending_map(),
        session_idle_timeout_minutes: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(480)),
        power_sessions: crate::power::PowerSessionTracker::default(),
        shutdown_token: tokio_util::sync::CancellationToken::new(),
        client_ip_resolver: crate::client_ip::ClientIpResolver::new(),
        task_registry: shared::task_registry::TaskRegistry::default(),
        user_rate_limiter: crate::rate_limit::UserRateLimiter::new(
            60,
            std::time::Duration::from_mins(1),
        ),
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Test-only helpers shared across this crate's unit test modules.

use std::str::FromStr;

use base64::Engine as _;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::TcpListener,
};

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

/// A [`NotificationService`](crate::notifications::NotificationService) around `pool` for unit
/// tests that never deliver to an email channel with a stored password.
pub(crate) fn test_notification_service(
    pool: sqlx::PgPool,
) -> crate::notifications::NotificationService {
    crate::notifications::NotificationService::new(
        pool,
        shared::crypto::derive_key(b"notification-service-test-key").expect("derive test key"),
    )
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

    let encryption_key = shared::crypto::derive_key(key_material).expect("derive test key");
    crate::AppState {
        pool: pool.clone(),
        encryption_key,
        registry: crate::ws::registry::AgentRegistry::new(),
        ui_broadcast,
        tunnel_manager,
        log_buffer: crate::log_buffer::LogBuffer::default(),
        notification_service: crate::notifications::NotificationService::new(pool, encryption_key),
        completion_bus: crate::ws::completion_bus::CompletionBus::new(),
        repo_op_tracker: crate::repo_op_tracker::RepoOpTracker::default(),
        background_task_tracker: crate::background_tasks::BackgroundTaskTracker::default(),
        repo_lock: crate::RepoLock::default(),
        import_tasks: crate::ImportTaskRegistry::default(),
        pending_dryruns: crate::new_pending_map(),
        pending_restores: crate::new_pending_map(),
        pending_vm_scans: crate::new_pending_map(),
        pending_vm_builds: crate::new_pending_map(),
        pending_vm_stages: crate::new_pending_map(),
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

/// Installs `script` as the borg binary until the returned guard is dropped; keep the
/// returned directory alive as long as the guard. Hold
/// [`acquire_test_binary_gate`](crate::borg::acquire_test_binary_gate) for the whole test.
pub(crate) async fn install_fake_borg(
    script: &str,
) -> (tempfile::TempDir, crate::borg::TestBinaryOverrideGuard) {
    use std::os::unix::fs::PermissionsExt as _;

    let tempdir = tempfile::tempdir().expect("create temp dir");
    let borg_path = tempdir.path().join("borg");
    tokio::fs::write(&borg_path, script)
        .await
        .expect("write fake borg");
    let mut permissions = tokio::fs::metadata(&borg_path)
        .await
        .expect("stat fake borg")
        .permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&borg_path, permissions)
        .await
        .expect("make fake borg executable");
    let guard = crate::borg::override_binary_for_tests(borg_path);
    (tempdir, guard)
}

/// One line from an SMTP client, as far as [`fake_smtp_server`] cares.
enum SmtpCommand {
    Hello,
    AuthPlain(String),
    Mail,
    Recipient,
    Data,
    EndOfData,
    Quit,
    Other,
}

impl FromStr for SmtpCommand {
    type Err = std::convert::Infallible;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut words = line.split_whitespace();
        let verb = words.next().unwrap_or_default().to_ascii_uppercase();
        Ok(match verb.as_str() {
            "EHLO" | "HELO" => Self::Hello,
            "AUTH" => Self::AuthPlain(words.nth(1).unwrap_or_default().to_owned()),
            "MAIL" => Self::Mail,
            "RCPT" => Self::Recipient,
            "DATA" => Self::Data,
            "." => Self::EndOfData,
            "QUIT" => Self::Quit,
            _ => Self::Other,
        })
    }
}

/// Accepts one connection, answers like a server offering AUTH PLAIN, and returns the
/// `user\0password` it was sent, if any.
pub(crate) async fn fake_smtp_server() -> (u16, tokio::task::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut credentials = None;
        let mut in_data = false;
        while let Ok(Some(line)) = lines.next_line().await {
            let command: SmtpCommand = line.parse().unwrap();
            let reply: &[u8] = match (in_data, command) {
                (true, SmtpCommand::EndOfData) => {
                    in_data = false;
                    b"250 queued\r\n"
                }
                (true, _) => continue,
                (false, SmtpCommand::Hello) => b"250-fake\r\n250 AUTH PLAIN\r\n",
                (false, SmtpCommand::AuthPlain(encoded)) => {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .unwrap();
                    credentials = Some(
                        String::from_utf8(decoded)
                            .unwrap()
                            .trim_start_matches('\0')
                            .to_owned(),
                    );
                    b"235 authenticated\r\n"
                }
                (false, SmtpCommand::Mail | SmtpCommand::Recipient) => b"250 ok\r\n",
                (false, SmtpCommand::Data) => {
                    in_data = true;
                    b"354 go ahead\r\n"
                }
                (false, SmtpCommand::Quit) => {
                    write.write_all(b"221 bye\r\n").await.unwrap();
                    break;
                }
                (false, SmtpCommand::EndOfData | SmtpCommand::Other) => b"500 unrecognised\r\n",
            };
            write.write_all(reply).await.unwrap();
        }
        credentials
    });
    (port, server)
}

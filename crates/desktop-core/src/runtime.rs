// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{path::PathBuf, sync::Arc, time::Duration};

use shared::agent_channel::ChannelPublicKey;

use crate::{
    paths::DesktopPaths,
    ports::free_loopback_port,
    postgres::{EmbeddedPostgres, PostgresConfig, PostgresError},
    process::{ChildSpec, EnvValue, ProcessError, Supervised},
    provision::{AdminSession, LocalServer, ProvisionError},
    secrets::{Secret, SecretError, SecretName, SecretStore, load_or_create},
    status::BackupStatusSource,
};

/// How long a child gets to stop after SIGTERM before it is killed. The
/// server waits for in-flight borg operations during its own shutdown, so
/// it gets longer than the agent.
const SERVER_STOP_GRACE: Duration = Duration::from_secs(30);
const AGENT_STOP_GRACE: Duration = Duration::from_secs(10);

/// Everything the desktop app needs to bring its processes up.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Where data, sockets and logs live.
    pub paths: DesktopPaths,
    /// The bundled `PostgreSQL` installation (contains `bin/`).
    pub postgres_install_dir: PathBuf,
    /// The bundled `server` binary.
    pub server_binary: PathBuf,
    /// The bundled `agent` binary.
    pub agent_binary: PathBuf,
    /// The built web UI the server serves (`ASSIMILATE_STATIC_DIR`).
    pub static_dir: Option<PathBuf>,
    /// The built docs site the server serves (`ASSIMILATE_DOCS_DIR`).
    pub docs_dir: Option<PathBuf>,
    /// The bundled `borg` binary, for both server and agent (`BORG_BINARY`).
    pub borg_binary: Option<PathBuf>,
    /// The name the local agent registers under.
    pub hostname: String,
    /// How long the server may take to answer `/api/health`.
    pub startup_timeout: Duration,
}

/// Why the desktop runtime couldn't start or stop cleanly.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The OS keychain failed.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The embedded database failed.
    #[error(transparent)]
    Postgres(#[from] PostgresError),
    /// A child process failed.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// Logging in or registering the agent failed.
    #[error(transparent)]
    Provision(#[from] ProvisionError),
    /// The agent channel key could not be derived from the server secret.
    #[error(transparent)]
    ChannelKey(#[from] shared::agent_channel::ChannelError),
    /// No free port, or a blocking task panicked.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The server process exited while the app was waiting for it.
    #[error("the server exited during startup with {0}; see its log in the app's logs folder")]
    ServerExited(std::process::ExitStatus),
}

/// What [`DesktopRuntime::start_server`] brought up on top of the database.
struct Started {
    server: Supervised,
    agent: Supervised,
    port: u16,
    session: AdminSession,
}

/// The running desktop stack: database, server and local agent.
pub struct DesktopRuntime {
    postgres: EmbeddedPostgres,
    server: Supervised,
    agent: Supervised,
    port: u16,
    session: AdminSession,
}

impl DesktopRuntime {
    /// Brings the stack up in dependency order: secrets, database, server,
    /// admin login, agent. Anything already started is stopped again if a
    /// later step fails.
    ///
    /// # Errors
    ///
    /// Fails with the first step that fails.
    pub async fn start(
        config: &RuntimeConfig,
        store: Arc<dyn SecretStore + Send + Sync>,
    ) -> Result<Self, RuntimeError> {
        let secret_key = load_secret(Arc::clone(&store), SecretName::ServerSecretKey).await?;
        let admin_password = load_secret(Arc::clone(&store), SecretName::AdminPassword).await?;
        let postgres_password = load_secret(store, SecretName::PostgresPassword).await?;

        let postgres = EmbeddedPostgres::start(
            &config.paths,
            &PostgresConfig {
                install_dir: config.postgres_install_dir.clone(),
                superuser_password: postgres_password,
                port: free_loopback_port()?,
            },
        )
        .await?;

        // From here on the database is running: every failure below must stop
        // it again, so all of it goes through one place.
        match Self::start_server(config, &postgres, &secret_key, &admin_password).await {
            Ok(started) => Ok(Self {
                postgres,
                server: started.server,
                agent: started.agent,
                port: started.port,
                session: started.session,
            }),
            Err(e) => {
                let _ = postgres
                    .stop()
                    .await
                    .inspect_err(|e| tracing::warn!(error = %e, "postgres did not stop"));
                Err(e)
            }
        }
    }

    /// Starts the server and, once it answers, signs in and starts the agent.
    /// A server that was spawned is stopped again if a later step fails.
    async fn start_server(
        config: &RuntimeConfig,
        postgres: &EmbeddedPostgres,
        secret_key: &Secret,
        admin_password: &Secret,
    ) -> Result<Started, RuntimeError> {
        let port = free_loopback_port()?;
        let database_url = postgres.database_url();
        // The server derives its channel key from the same secret, so the
        // local agent can pin it without asking the server for it.
        let server_key =
            shared::agent_channel::ChannelPrivateKey::derive(secret_key.expose().as_bytes())?
                .public_key()?;
        let spec = server_spec(config, port, &database_url, secret_key);
        let mut server = Supervised::spawn(&spec).await?;
        match Self::connect(&mut server, config, admin_password, port, server_key).await {
            Ok((agent, session)) => Ok(Started {
                server,
                agent,
                port,
                session,
            }),
            Err(e) => {
                stop_quietly(server, SERVER_STOP_GRACE).await;
                Err(e)
            }
        }
    }

    /// Waits for the server to answer (or exit), then signs in and starts the
    /// agent.
    async fn connect(
        server: &mut Supervised,
        config: &RuntimeConfig,
        admin_password: &Secret,
        port: u16,
        server_key: ChannelPublicKey,
    ) -> Result<(Supervised, AdminSession), RuntimeError> {
        let api = LocalServer::new(port)?;
        let healthy = api.wait_until_healthy(config.startup_timeout);
        tokio::select! {
            ready = healthy => ready?,
            exited = wait_for_exit(server) => {
                return Err(exited.map_or_else(RuntimeError::from, RuntimeError::ServerExited));
            }
        }
        Self::finish_start(&api, config, admin_password, port, server_key).await
    }

    async fn finish_start(
        api: &LocalServer,
        config: &RuntimeConfig,
        admin_password: &Secret,
        port: u16,
        server_key: ChannelPublicKey,
    ) -> Result<(Supervised, AdminSession), RuntimeError> {
        let session = api.admin_session(admin_password).await?;
        let agent = api.provision_agent(&session, &config.hostname).await?;
        let agent = Supervised::spawn(&agent_spec(
            config,
            port,
            &agent.hostname,
            agent.token,
            server_key,
        ))
        .await?;
        Ok((agent, session))
    }

    /// The URL the webview opens.
    #[must_use]
    pub fn server_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The admin session, for signing the webview in.
    #[must_use]
    pub fn session(&self) -> &AdminSession {
        &self.session
    }

    /// Asks this stack's server how its backups are doing, for the tray icon.
    ///
    /// # Errors
    ///
    /// Fails if the HTTP client can't be built.
    pub fn backup_status(&self) -> Result<BackupStatusSource, ProvisionError> {
        Ok(BackupStatusSource::new(
            LocalServer::new(self.port)?,
            self.session.clone(),
        ))
    }

    /// Stops agent, server and database, in that order: the agent depends on
    /// the server and the server on the database. Every step runs even if
    /// an earlier one fails; the first error is returned.
    ///
    /// # Errors
    ///
    /// The first step that failed.
    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        let agent = self.agent.terminate(AGENT_STOP_GRACE).await.map(drop);
        let server = self.server.terminate(SERVER_STOP_GRACE).await.map(drop);
        let postgres = self.postgres.stop().await;
        agent?;
        server?;
        postgres?;
        Ok(())
    }
}

async fn load_secret(
    store: Arc<dyn SecretStore + Send + Sync>,
    name: SecretName,
) -> Result<Secret, RuntimeError> {
    // The keychain may block (and on macOS may show a prompt).
    let secret = tokio::task::spawn_blocking(move || load_or_create(store.as_ref(), name))
        .await
        .map_err(std::io::Error::other)??;
    Ok(secret)
}

/// Resolves once the child has exited, with its exit status.
async fn wait_for_exit(child: &mut Supervised) -> Result<std::process::ExitStatus, ProcessError> {
    loop {
        if let Some(status) = child.exit_status()? {
            return Ok(status);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn stop_quietly(child: Supervised, grace: Duration) {
    let name = child.name();
    let _ = child
        .terminate(grace)
        .await
        .inspect_err(|e| tracing::warn!(name, error = %e, "child did not stop"));
}

fn plain(value: impl Into<String>) -> EnvValue {
    EnvValue::Plain(value.into())
}

fn path(value: &std::path::Path) -> EnvValue {
    EnvValue::Plain(value.to_string_lossy().into_owned())
}

fn server_spec(
    config: &RuntimeConfig,
    port: u16,
    database_url: &Secret,
    secret_key: &Secret,
) -> ChildSpec {
    let mut env = vec![
        ("DATABASE_URL", EnvValue::Secret(database_url.clone())),
        (
            "ASSIMILATE_SECRET_KEY",
            EnvValue::Secret(secret_key.clone()),
        ),
        ("ASSIMILATE_BIND_ADDR", plain(format!("127.0.0.1:{port}"))),
        ("ASSIMILATE_DEPLOYMENT_MODE", plain("desktop")),
        // Plain HTTP on loopback: a Secure cookie would never be sent back.
        ("ASSIMILATE_SECURE_COOKIES", plain("false")),
        ("SSH_KEY_DIR", path(&config.paths.ssh_keys())),
    ];
    env.extend(
        [
            ("ASSIMILATE_STATIC_DIR", config.static_dir.as_deref()),
            ("ASSIMILATE_DOCS_DIR", config.docs_dir.as_deref()),
            ("BORG_BINARY", config.borg_binary.as_deref()),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, path(value)))),
    );
    ChildSpec {
        name: "server",
        program: config.server_binary.clone(),
        env,
        log_file: config.paths.logs().join("server.log"),
    }
}

fn agent_spec(
    config: &RuntimeConfig,
    port: u16,
    hostname: &str,
    token: Secret,
    server_key: ChannelPublicKey,
) -> ChildSpec {
    let mut env = vec![
        ("BORG_SERVER_URL", plain(format!("ws://127.0.0.1:{port}"))),
        ("BORG_AGENT_TOKEN", EnvValue::Secret(token)),
        ("BORG_SERVER_KEY", plain(server_key.to_string())),
        ("BORG_HOSTNAME", plain(hostname)),
    ];
    env.extend(
        config
            .borg_binary
            .as_deref()
            .map(|borg| ("BORG_BINARY", path(borg))),
    );
    ChildSpec {
        name: "agent",
        program: config.agent_binary.clone(),
        env,
        log_file: config.paths.logs().join("agent.log"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn config() -> RuntimeConfig {
        RuntimeConfig {
            paths: DesktopPaths::at(PathBuf::from("/app")),
            postgres_install_dir: PathBuf::from("/bundle/postgres"),
            server_binary: PathBuf::from("/bundle/server"),
            agent_binary: PathBuf::from("/bundle/agent"),
            static_dir: Some(PathBuf::from("/bundle/static")),
            docs_dir: None,
            borg_binary: Some(PathBuf::from("/bundle/borg")),
            hostname: "laptop".to_owned(),
            startup_timeout: Duration::from_secs(5),
        }
    }

    fn value<'a>(spec: &'a ChildSpec, key: &str) -> Option<&'a str> {
        spec.env
            .iter()
            .find_map(|(name, value)| key.eq(*name).then(|| value.expose()))
    }

    fn is_secret(spec: &ChildSpec, key: &str) -> bool {
        spec.env
            .iter()
            .any(|(name, value)| key.eq(*name) && matches!(value, EnvValue::Secret(_)))
    }

    #[test]
    fn server_runs_in_desktop_mode_on_loopback_with_secrets_marked() {
        let spec = server_spec(
            &config(),
            4321,
            &Secret::from_stored("postgresql://u:p@x/db".to_owned()),
            &Secret::from_stored("key".to_owned()),
        );

        assert_eq!(spec.program, PathBuf::from("/bundle/server"));
        assert!(is_secret(&spec, "DATABASE_URL"));
        assert!(is_secret(&spec, "ASSIMILATE_SECRET_KEY"));
        assert_eq!(value(&spec, "ASSIMILATE_BIND_ADDR"), Some("127.0.0.1:4321"));
        assert_eq!(value(&spec, "ASSIMILATE_DEPLOYMENT_MODE"), Some("desktop"));
        assert_eq!(value(&spec, "ASSIMILATE_SECURE_COOKIES"), Some("false"));
        assert_eq!(
            value(&spec, "SSH_KEY_DIR").map(Path::new),
            Some(Path::new("/app").join("ssh").as_path())
        );
        assert_eq!(
            value(&spec, "ASSIMILATE_STATIC_DIR"),
            Some("/bundle/static")
        );
        assert_eq!(value(&spec, "BORG_BINARY"), Some("/bundle/borg"));
        assert_eq!(value(&spec, "ASSIMILATE_DOCS_DIR"), None);
        assert_eq!(spec.log_file, PathBuf::from("/app/logs/server.log"));
    }

    #[test]
    fn agent_connects_to_the_local_server_under_the_registered_hostname() {
        let secret = Secret::from_stored("server secret".to_owned());
        let key = shared::agent_channel::ChannelPrivateKey::derive(secret.expose().as_bytes())
            .unwrap()
            .public_key()
            .unwrap();
        let spec = agent_spec(
            &config(),
            4321,
            "laptop",
            Secret::from_stored("t".to_owned()),
            key,
        );

        assert_eq!(value(&spec, "BORG_SERVER_URL"), Some("ws://127.0.0.1:4321"));
        assert!(is_secret(&spec, "BORG_AGENT_TOKEN"));
        assert_eq!(
            value(&spec, "BORG_SERVER_KEY"),
            Some(key.to_string().as_str())
        );
        assert_eq!(value(&spec, "BORG_HOSTNAME"), Some("laptop"));
        assert_eq!(value(&spec, "BORG_BINARY"), Some("/bundle/borg"));
        assert_eq!(spec.log_file, PathBuf::from("/app/logs/agent.log"));
    }

    #[test]
    fn agent_without_a_bundled_borg_falls_back_to_path() {
        let config = RuntimeConfig {
            borg_binary: None,
            ..config()
        };
        let key = shared::agent_channel::ChannelPrivateKey::derive(b"s")
            .unwrap()
            .public_key()
            .unwrap();
        let spec = agent_spec(&config, 1, "h", Secret::from_stored("t".to_owned()), key);
        assert_eq!(value(&spec, "BORG_BINARY"), None);
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Running the agent as a Windows service: `service install` registers it
//! with the Service Control Manager and keeps its server URL and token in a
//! file only `LocalSystem` and administrators can read, `service uninstall`
//! removes both, and `service run` is what the SCM starts.
//!
//! The service restarts itself after it exits without reporting a stop -
//! which is how the dashboard's "Restart agent" works, as under systemd.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use shared::windows_acl::{Access, create_private_dir, create_private_file};
use tokio::{io::AsyncWriteExt as _, sync::Notify};
use tracing_subscriber::EnvFilter;
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use crate::{Args, systemd::RestartCapability};

/// The service's name, the same as the systemd unit's.
const SERVICE_NAME: &str = "assimilate-agent";
const DISPLAY_NAME: &str = "Assimilate backup agent";
const DESCRIPTION: &str = "Runs Assimilate backups on this machine.";

/// `ERROR_SERVICE_EXISTS`: `install` updates a service that is already there.
const ERROR_SERVICE_EXISTS: i32 = 1073;
/// `ERROR_SERVICE_DOES_NOT_EXIST`: `uninstall` of a service that isn't there.
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

/// How long the SCM waits after each restart before starting the agent again.
const RESTART_DELAY: Duration = Duration::from_secs(10);
/// How long a run of restarts counts before the SCM starts over.
const RESTART_RESET: Duration = Duration::from_hours(24);

/// `assimilate-agent service ...`.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Install, remove or run the agent as a Windows service.
    Service {
        #[command(subcommand)]
        action: Action,
    },
}

#[derive(clap::Subcommand)]
pub enum Action {
    /// Register the service, store its server URL and token, and start it.
    /// Run again to change either; the service is restarted.
    Install {
        #[arg(long, env = "BORG_SERVER_URL")]
        server_url: String,
        /// Prefer the environment variable: a command line can be read by
        /// other local users while the command runs.
        #[arg(long, env = "BORG_AGENT_TOKEN", hide_env_values = true)]
        token: String,
    },
    /// Stop and remove the service and its stored configuration.
    Uninstall,
    /// Run as the service. Only the Service Control Manager starts this.
    #[command(hide = true)]
    Run,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Windows service error: {0}")]
    Windows(#[from] windows_service::Error),
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("invalid service configuration {}: {source}", path.display())]
    Config {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("%ProgramData% is not set")]
    NoProgramData,
}

impl ServiceError {
    fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> Self {
        let context = context.into();
        move |source| Self::Io { context, source }
    }
}

/// What `install` stores for `run`.
#[derive(Serialize, Deserialize)]
struct StoredConfig {
    server_url: String,
    token: String,
}

/// Where the service keeps its configuration and log.
struct ServicePaths {
    dir: PathBuf,
}

impl ServicePaths {
    fn from_env() -> Result<Self, ServiceError> {
        let program_data = std::env::var_os("ProgramData").ok_or(ServiceError::NoProgramData)?;
        Ok(Self::at(&PathBuf::from(program_data)))
    }

    fn at(program_data: &Path) -> Self {
        Self {
            dir: program_data.join("Assimilate").join("agent"),
        }
    }

    fn config(&self) -> PathBuf {
        self.dir.join("agent.json")
    }

    fn log(&self) -> PathBuf {
        self.dir.join("agent.log")
    }
}

/// Runs one `service` subcommand to completion.
///
/// # Errors
///
/// Fails if the service can't be installed, removed or run, or its
/// configuration can't be written or read.
pub fn run(command: Command) -> Result<(), ServiceError> {
    let Command::Service { action } = command;
    if !matches!(action, Action::Run) {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("info"))
            .init();
    }
    match action {
        Action::Install { server_url, token } => {
            runtime()?.block_on(write_config(&StoredConfig { server_url, token }))?;
            install()
        }
        Action::Uninstall => {
            uninstall()?;
            runtime()?.block_on(remove_config())
        }
        Action::Run => Ok(service_dispatcher::start(SERVICE_NAME, ffi_service_main)?),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, ServiceError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServiceError::io("failed to start the async runtime"))
}

async fn write_config(config: &StoredConfig) -> Result<(), ServiceError> {
    let paths = ServicePaths::from_env()?;
    create_service_dir(&paths.dir).await?;
    let path = paths.config();
    let json = serde_json::to_vec_pretty(config).map_err(|source| ServiceError::Config {
        path: path.clone(),
        source,
    })?;
    let context = format!("writing {}", path.display());
    let file = create_private_file(&path, Access::SystemAndAdministrators)
        .await
        .map_err(ServiceError::io(context.clone()))?;
    let mut file = tokio::fs::File::from_std(file);
    file.write_all(&json)
        .await
        .map_err(ServiceError::io(context.clone()))?;
    file.flush().await.map_err(ServiceError::io(context))
}

async fn remove_config() -> Result<(), ServiceError> {
    let path = ServicePaths::from_env()?.config();
    match tokio::fs::remove_file(&path).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(ServiceError::io(format!("removing {}", path.display()))(e))
        }
        Ok(()) | Err(_) => Ok(()),
    }
}

async fn read_config(path: &Path) -> Result<StoredConfig, ServiceError> {
    let json = tokio::fs::read(path)
        .await
        .map_err(ServiceError::io(format!("reading {}", path.display())))?;
    serde_json::from_slice(&json).map_err(|source| ServiceError::Config {
        path: path.to_path_buf(),
        source,
    })
}

fn service_info() -> Result<ServiceInfo, ServiceError> {
    Ok(ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()
            .map_err(ServiceError::io("locating the agent executable"))?,
        launch_arguments: vec![OsString::from("service"), OsString::from("run")],
        dependencies: Vec::new(),
        // LocalSystem, so borg can read every file it is asked to back up.
        account_name: None,
        account_password: None,
    })
}

/// Restarts after every exit that wasn't a reported stop, including a clean
/// exit (the dashboard's restart), with a pause between attempts.
fn restart_policy() -> ServiceFailureActions {
    ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(RESTART_RESET),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: RESTART_DELAY,
            };
            3
        ]),
    }
}

fn install() -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = service_info()?;
    let access = ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::QUERY_STATUS;
    let service = match manager.create_service(&info, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(e))
            if e.raw_os_error() == Some(ERROR_SERVICE_EXISTS) =>
        {
            let service = manager.open_service(SERVICE_NAME, access)?;
            service.change_config(&info)?;
            stop_and_wait(&service)?;
            service
        }
        Err(e) => return Err(e.into()),
    };
    service.set_description(DESCRIPTION)?;
    service.update_failure_actions(restart_policy())?;
    service.set_failure_actions_on_non_crash_failures(true)?;
    service.start::<&str>(&[])?;
    tracing::info!("installed and started the {SERVICE_NAME} service");
    Ok(())
}

fn uninstall() -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let access = ServiceAccess::STOP | ServiceAccess::QUERY_STATUS | ServiceAccess::DELETE;
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(e))
            if e.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    stop_and_wait(&service)?;
    service.delete()?;
    Ok(())
}

/// Stops `service` if it runs, and waits for the agent's own shutdown, which
/// may let a running borg finish its checkpoint first.
fn stop_and_wait(service: &windows_service::service::Service) -> Result<(), ServiceError> {
    if service.query_status()?.current_state == ServiceState::Stopped {
        return Ok(());
    }
    let _ = service.stop();
    let started = std::time::Instant::now();
    let limit = stop_wait_hint().saturating_mul(2);
    while service.query_status()?.current_state != ServiceState::Stopped {
        if started.elapsed() > limit {
            return Err(ServiceError::Io {
                context: format!("stopping the {SERVICE_NAME} service"),
                source: std::io::Error::from(std::io::ErrorKind::TimedOut),
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// What the agent tells the SCM a stop may take: its whole shutdown deadline.
fn stop_wait_hint() -> Duration {
    crate::shutdown_deadline().saturating_add(crate::EXECUTOR_JOIN_GRACE)
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(arguments: Vec<OsString>) {
    let arguments: Vec<String> = arguments
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if let Err(e) = run_service(&arguments) {
        tracing::error!(error = %e, "the agent service failed");
        // Before the log file is open nothing above reaches anyone: a
        // service has no console, and the log may be what failed.
        record_startup_failure(&startup_failure_log(std::time::SystemTime::now()), &e);
    }
}

/// Where a failure at `at` the service couldn't log is written: the
/// system's temp folder, which exists even when `%ProgramData%` or the
/// agent's own folder doesn't, in a file named for the time.
fn startup_failure_log(at: std::time::SystemTime) -> PathBuf {
    let secs = at
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    std::env::temp_dir().join(format!("assimilate-agent-service-error-{secs}.log"))
}

/// Writes `error` to `path`, which must not exist yet: on older Windows
/// `LocalSystem`'s temp folder is `C:\Windows\Temp`, where any user can
/// create a file, so a file already there may be someone else's to read.
/// Best effort: there is nowhere left to report a failure to write it.
fn record_startup_failure(path: &Path, error: &ServiceError) {
    use std::io::Write as _;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        let _ = writeln!(file, "the agent service failed: {error}");
    }
}

fn run_service(arguments: &[String]) -> Result<(), ServiceError> {
    let paths = ServicePaths::from_env()?;
    let runtime = runtime()?;
    let log = runtime.block_on(open_log(&paths.log()))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log))
        .init();
    tracing::info!(
        ?arguments,
        version = crate::agent_version_string(),
        "service starting"
    );

    let stop = Arc::new(Notify::new());
    let handler_stop = Arc::clone(&stop);
    // `ServiceControl` is non-exhaustive, so this can't be an exhaustive
    // match: everything but a stop request or a status query is declined.
    let status = service_control_handler::register(SERVICE_NAME, move |control| {
        if matches!(control, ServiceControl::Stop | ServiceControl::Shutdown) {
            handler_stop.notify_one();
            ServiceControlHandlerResult::NoError
        } else if matches!(control, ServiceControl::Interrogate) {
            ServiceControlHandlerResult::NoError
        } else {
            ServiceControlHandlerResult::NotImplemented
        }
    })?;
    report(status, ServiceState::Running, ServiceExitCode::Win32(0))?;

    let fatal = runtime.block_on(async {
        let config = read_config(&paths.config()).await?;
        let args = Args {
            server_url: config.server_url,
            token: config.token,
        };
        let restart = RestartCapability {
            supported: true,
            unavailable_reason: None,
        };
        let stopping = {
            let stop = Arc::clone(&stop);
            async move {
                stop.notified().await;
                tracing::info!("stop requested by the Service Control Manager");
                let _ = report(status, ServiceState::StopPending, ServiceExitCode::Win32(0));
            }
        };
        Ok::<bool, ServiceError>(crate::run(args, stopping, restart).await)
    });
    // A fatal stop (say, a rejected token) is reported as a failure, which the
    // restart policy acts on, as systemd's Restart=on-failure does.
    let exit_code = match &fatal {
        Ok(false) => ServiceExitCode::Win32(0),
        Ok(true) | Err(_) => ServiceExitCode::ServiceSpecific(1),
    };
    report(status, ServiceState::Stopped, exit_code)?;
    fatal.map(|_| ())
}

fn report(
    status: ServiceStatusHandle,
    state: ServiceState,
    exit_code: ServiceExitCode,
) -> Result<(), ServiceError> {
    let controls_accepted = match state {
        ServiceState::Running => ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceState::Stopped
        | ServiceState::StartPending
        | ServiceState::StopPending
        | ServiceState::ContinuePending
        | ServiceState::PausePending
        | ServiceState::Paused => ServiceControlAccept::empty(),
    };
    status.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint: stop_wait_hint(),
        process_id: None,
    })?;
    Ok(())
}

/// Creates the service's folder so that only `LocalSystem` and administrators
/// can open it or anything in it: the config and the log alike.
async fn create_service_dir(dir: &Path) -> Result<(), ServiceError> {
    let context = format!("creating {}", dir.display());
    if let Some(parent) = dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(ServiceError::io(context.clone()))?;
    }
    create_private_dir(dir, Access::SystemAndAdministrators)
        .await
        .map_err(ServiceError::io(context))
}

async fn open_log(path: &Path) -> Result<std::fs::File, ServiceError> {
    let context = format!("opening {}", path.display());
    if let Some(dir) = path.parent() {
        create_service_dir(dir).await?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(ServiceError::io(context))?;
    Ok(file.into_std().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failure_before_logging_is_recorded_in_a_new_fallback_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("error.log");

        record_startup_failure(&path, &ServiceError::NoProgramData);
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "the agent service failed: %ProgramData% is not set\n"
        );

        // A file already there is left alone, not written through.
        let planted = dir.path().join("planted.log");
        tokio::fs::write(&planted, b"planted").await.unwrap();
        record_startup_failure(&planted, &ServiceError::NoProgramData);
        assert_eq!(tokio::fs::read(&planted).await.unwrap(), b"planted");

        let at = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            startup_failure_log(at),
            std::env::temp_dir().join("assimilate-agent-service-error-1700000000.log")
        );
    }

    #[test]
    fn files_live_under_program_data() {
        let paths = ServicePaths::at(Path::new(r"C:\ProgramData"));
        assert_eq!(
            paths.config(),
            Path::new(r"C:\ProgramData\Assimilate\agent\agent.json")
        );
        assert_eq!(
            paths.log(),
            Path::new(r"C:\ProgramData\Assimilate\agent\agent.log")
        );
    }

    #[test]
    fn the_service_runs_itself_with_the_run_subcommand() {
        let info = service_info().unwrap();
        assert_eq!(info.launch_arguments, ["service", "run"]);
        assert_eq!(info.start_type, ServiceStartType::AutoStart);
        assert!(info.account_name.is_none(), "runs as LocalSystem");
    }

    #[test]
    fn every_failure_action_restarts_after_a_pause() {
        let actions = restart_policy().actions.unwrap();
        assert_eq!(actions.len(), 3);
        assert!(actions.iter().all(|action| {
            action.action_type == ServiceActionType::Restart && action.delay == RESTART_DELAY
        }));
    }

    #[tokio::test]
    async fn the_stored_config_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.json");
        let json = serde_json::to_vec(&StoredConfig {
            server_url: "wss://backup.example".to_owned(),
            token: "t0ken".to_owned(),
        })
        .unwrap();
        tokio::fs::write(&path, json).await.unwrap();

        let config = read_config(&path).await.unwrap();

        assert_eq!(config.server_url, "wss://backup.example");
        assert_eq!(config.token, "t0ken");
    }

    #[tokio::test]
    async fn a_malformed_config_names_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.json");
        tokio::fs::write(&path, b"{").await.unwrap();

        let error = read_config(&path).await.err().unwrap();

        assert!(error.to_string().contains("agent.json"), "{error}");
    }
}

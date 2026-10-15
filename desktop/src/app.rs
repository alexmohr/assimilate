// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use desktop_core::{
    paths::{DesktopPaths, PathsError},
    runtime::{DesktopRuntime, RuntimeConfig, RuntimeError},
    secrets::{KeychainStore, SecretStore},
};
use tauri::{
    AppHandle, Manager, RunEvent, Url, WebviewWindow, WindowEvent,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

use crate::{
    layout::BundleLayout,
    lifecycle::{Lifecycle, QuitDecision, Started, finish_or_stop},
    session::session_cookie,
};

const MAIN_WINDOW: &str = "main";
/// Puts all app data under this directory instead of the platform default.
const DATA_DIR_VAR: &str = "ASSIMILATE_DESKTOP_DATA_DIR";
/// Debug builds only: keep secrets in memory instead of the OS keychain, so
/// a development run never touches the developer's real keychain. Data
/// encrypted in such a run can't be read by the next one.
#[cfg(debug_assertions)]
const EPHEMERAL_SECRETS_VAR: &str = "ASSIMILATE_DESKTOP_EPHEMERAL_SECRETS";
const EXIT_OK: i32 = 0;
const EXIT_STOP_FAILED: i32 = 1;
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_mins(2);

/// Why the app couldn't start at all.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Tauri failed to build or run the app.
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

/// Why the background stack couldn't be brought up.
#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("the main window is missing")]
    NoWindow,
    #[error("the local server URL is invalid: {0}")]
    Url(String),
}

/// App-wide state: the running stack, once it is up, and the quit state.
type Desktop = Lifecycle<DesktopRuntime>;

/// Where the app keeps its data: [`DATA_DIR_VAR`] if set, otherwise the
/// platform default.
///
/// # Errors
///
/// Fails if the platform default can't be determined.
pub fn data_paths() -> Result<DesktopPaths, PathsError> {
    std::env::var_os(DATA_DIR_VAR).map_or_else(DesktopPaths::from_env, |dir| {
        Ok(DesktopPaths::at(PathBuf::from(dir)))
    })
}

/// Builds and runs the app until the user quits from the tray.
///
/// # Errors
///
/// Fails if Tauri can't build the app.
pub fn run(paths: Result<DesktopPaths, PathsError>) -> Result<(), AppError> {
    let app = tauri::Builder::default()
        .manage(Desktop::default())
        .setup(move |app| {
            build_tray(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { start(handle, paths).await });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps backups running; quit is in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(e) = window.hide() {
                    tracing::warn!(error = %e, "failed to hide the window");
                }
            }
        })
        .build(tauri::generate_context!())?;

    app.run(|handle, event| match event {
        RunEvent::ExitRequested { api, .. } => request_exit(handle, &api),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(handle),
        _ => {}
    });
    Ok(())
}

/// An exit requested by the OS goes through [`quit`]; only the exit that
/// follows a finished shutdown is let through.
fn request_exit(handle: &AppHandle, api: &tauri::ExitRequestApi) {
    match quit(handle) {
        QuitDecision::Exit => {}
        QuitDecision::StartShutdown | QuitDecision::AlreadyShuttingDown => api.prevent_exit(),
    }
}

/// The app's single way out: the first request stops the stack in order
/// (agent, server, database) and then exits; later requests wait for it.
fn quit(handle: &AppHandle) -> QuitDecision {
    let decision = handle.state::<Desktop>().request_quit();
    if decision == QuitDecision::StartShutdown {
        let handle = handle.clone();
        tauri::async_runtime::spawn(async move { shutdown_and_exit(handle).await });
    }
    decision
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Assimilate", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Assimilate", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit_item])?;
    let open_id = open.id().clone();
    let quit_id = quit_item.id().clone();
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Assimilate")
        .menu(&menu)
        .on_menu_event(move |app, event| {
            if *event.id() == open_id {
                show_main_window(app);
            } else if *event.id() == quit_id {
                quit(app);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let shown = window
        .show()
        .and_then(|()| window.unminimize())
        .and_then(|()| window.set_focus());
    if let Err(e) = shown {
        tracing::warn!(error = %e, "failed to show the window");
    }
}

async fn start(app: AppHandle, paths: Result<DesktopPaths, PathsError>) {
    let paths = match paths {
        Ok(paths) => paths,
        Err(e) => return report_startup_error(&app, &e, None),
    };
    // A quit during startup waits for this to finish and then stops
    // whatever came up; see `Lifecycle::start`.
    match app.state::<Desktop>().start(bring_up(&app, &paths)).await {
        Ok(Started::Running) => {}
        Ok(Started::Skipped) => tracing::info!("quitting before startup; not starting the stack"),
        Err(e) => report_startup_error(&app, &e, Some(&paths.logs())),
    }
}

async fn bring_up(app: &AppHandle, paths: &DesktopPaths) -> Result<DesktopRuntime, StartError> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    // Look the window up first, so a missing window never leaves a started
    // stack behind.
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or(StartError::NoWindow)?;
    let resource_dir = app.path().resource_dir()?;
    let layout = BundleLayout::resolve(&exe_dir, &resource_dir, |var| {
        std::env::var_os(var).map(PathBuf::from)
    })
    .await;
    let config = RuntimeConfig {
        paths: paths.clone(),
        postgres_install_dir: layout.postgres_dir,
        server_binary: layout.server_binary,
        agent_binary: layout.agent_binary,
        static_dir: Some(layout.static_dir),
        docs_dir: layout.docs_dir,
        borg_binary: layout.borg_binary,
        hostname: gethostname::gethostname().to_string_lossy().into_owned(),
        startup_timeout: SERVER_STARTUP_TIMEOUT,
    };
    let runtime = DesktopRuntime::start(&config, secret_store()).await?;
    finish_or_stop(runtime, |runtime| sign_in(&window, runtime)).await
}

fn secret_store() -> Arc<dyn SecretStore + Send + Sync> {
    #[cfg(debug_assertions)]
    if std::env::var_os(EPHEMERAL_SECRETS_VAR).is_some() {
        tracing::warn!("secrets are kept in memory only for this run");
        return Arc::new(desktop_core::secrets::InMemoryStore::default());
    }
    Arc::new(KeychainStore::default())
}

fn sign_in(window: &WebviewWindow, runtime: &DesktopRuntime) -> Result<(), StartError> {
    window.set_cookie(session_cookie(runtime.session().cookie_value().expose()))?;
    let url = Url::parse(&runtime.server_url()).map_err(|e| StartError::Url(e.to_string()))?;
    window.navigate(url)?;
    Ok(())
}

fn report_startup_error(app: &AppHandle, error: &dyn std::fmt::Display, log_dir: Option<&Path>) {
    tracing::error!(%error, "the desktop stack failed to start");
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let log_dir = log_dir.map_or_else(
        || "the app's data folder".to_owned(),
        |dir| dir.display().to_string(),
    );
    // Both values are JSON-encoded, so they reach the page as strings, never
    // as markup or code.
    let script = format!(
        "window.showStartupError({}, {})",
        serde_json::Value::from(error.to_string()),
        serde_json::Value::from(log_dir),
    );
    if let Err(e) = window.eval(&script) {
        tracing::warn!(error = %e, "failed to show the startup error");
    }
}

/// Waits for a start in progress, stops the stack and exits. Only [`quit`]
/// calls this, once.
async fn shutdown_and_exit(app: AppHandle) {
    let stopped = app.state::<Desktop>().stop().await;
    if let Err(e) = &stopped {
        tracing::warn!(error = %e, "the desktop stack did not stop cleanly");
    }
    app.exit(exit_code(&stopped));
}

/// The process exit code after a shutdown: non-zero if the stack did not
/// stop cleanly, so a supervisor or script can tell.
fn exit_code<E>(stopped: &Result<(), E>) -> i32 {
    if stopped.is_ok() {
        EXIT_OK
    } else {
        EXIT_STOP_FAILED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_stop_exits_with_zero() {
        assert_eq!(exit_code(&Ok::<(), std::io::Error>(())), 0);
    }

    #[test]
    fn a_failed_stop_exits_non_zero() {
        let failed = Err(std::io::Error::other("postgres did not stop"));
        assert_ne!(exit_code(&failed), 0);
    }
}

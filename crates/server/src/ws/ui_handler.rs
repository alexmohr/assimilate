// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code},
    },
    http::StatusCode,
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use shared::protocol::ServerToUi;

use super::ui_audience::UiAudience;
use crate::{
    AppState,
    api::auth::{SessionUse, authenticate_session},
    cookies,
    error::ApiError,
};

/// The path the UI socket is served on, for the password-change gate in
/// [`authenticate_session`].
const UI_SOCKET_PATH: &str = "/ws/ui";

/// How often an open UI socket re-checks its session and reloads what its
/// user may see. A socket whose session was logged out, revoked, expired or
/// went idle closes within this long, and a change to the user's roles or to
/// an agent's or repository's visibility reaches it within this long.
const SESSION_RECHECK_INTERVAL: Duration = Duration::from_secs(30);

/// WebSocket upgrade handler for browser UI clients.
///
/// Authenticates the session cookie with every check the REST API applies to
/// a session (expiry, idle timeout, pending TOTP, forced password change),
/// then upgrades the connection to a UI broadcast receiver that only
/// delivers events for the agents and repositories the user can see.
pub async fn ui_ws_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    upgrade_ui_socket(ws, &headers, state, SESSION_RECHECK_INTERVAL).await
}

/// [`ui_ws_handler`] with the session re-check interval as a parameter, so
/// tests can watch a socket close without waiting the production interval.
async fn upgrade_ui_socket(
    ws: WebSocketUpgrade,
    headers: &axum::http::HeaderMap,
    state: AppState,
    recheck_every: Duration,
) -> axum::response::Response {
    let Some(session_id) = cookies::session_cookie(headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match load_audience(&state, session_id.clone(), SessionUse::Request).await {
        Ok(audience) => {
            let session = UiSession {
                session_id,
                audience,
                recheck_every,
            };
            ws.on_upgrade(|socket| handle_ui_socket(socket, state, session))
                .into_response()
        }
        Err(e) => e.into_response(),
    }
}

/// Authenticates `session_id` and loads what its user may see.
async fn load_audience(
    state: &AppState,
    session_id: String,
    session_use: SessionUse,
) -> Result<UiAudience, ApiError> {
    let user = authenticate_session(state, session_id, UI_SOCKET_PATH, session_use).await?;
    UiAudience::load(&state.pool, user.user_id).await
}

/// The signed-in session one UI socket serves.
struct UiSession {
    /// The session cookie value, re-checked every `recheck_every`.
    session_id: String,
    /// What the session's user may see, reloaded on every re-check.
    audience: UiAudience,
    /// How often to re-check the session.
    recheck_every: Duration,
}

impl UiSession {
    /// Re-checks the session and reloads its audience. `false` means the
    /// session is no longer valid (or could not be checked) and the socket
    /// must close.
    async fn recheck(&mut self, state: &AppState) -> bool {
        match load_audience(state, self.session_id.clone(), SessionUse::Recheck).await {
            Ok(audience) => {
                self.audience = audience;
                true
            }
            Err(e) => {
                tracing::debug!(error = %e, "closing UI socket: session ended or unverifiable");
                false
            }
        }
    }
}

type UiSink = SplitSink<WebSocket, Message>;

/// An event could not be sent: the client end of the socket went away (or,
/// in principle, the event did not serialize). Either way the socket closes,
/// and the browser reconnects and refetches what it shows.
struct SocketGone;

/// Sends `event` if `audience` may see it.
async fn deliver(
    sink: &mut UiSink,
    audience: &UiAudience,
    event: &ServerToUi,
) -> Result<(), SocketGone> {
    if audience.may_see(event) {
        send(sink, event).await?;
    }
    Ok(())
}

/// Sends `event` unfiltered.
async fn send(sink: &mut UiSink, event: &ServerToUi) -> Result<(), SocketGone> {
    let json = serde_json::to_string(event).map_err(|_| SocketGone)?;
    sink.send(Message::Text(json.into()))
        .await
        .map_err(|_| SocketGone)
}

/// Replays the in-progress imports and backups `audience` may see, so a
/// client that connects mid-operation shows it without waiting for its
/// next event.
async fn replay_snapshots(
    sink: &mut UiSink,
    state: &AppState,
    audience: &UiAudience,
) -> Result<(), SocketGone> {
    for (repo_id, snap) in state.ui_broadcast.current_import_snapshots() {
        let event = ServerToUi::ImportProgress {
            repo_id,
            progress: snap.progress,
            total: snap.total,
            message: snap.message,
        };
        deliver(sink, audience, &event).await?;
    }

    for snap in state.ui_broadcast.current_active_backups() {
        if !audience.may_see_backup(&snap.hostname, snap.repo_id) {
            continue;
        }
        let started = ServerToUi::BackupStarted {
            hostname: snap.hostname.clone(),
            target_name: snap.target_name.clone(),
            archive_name: snap.archive_name.clone(),
            schedule_id: snap.schedule_id,
            started_at: snap.started_at,
        };
        send(sink, &started).await?;
        if let Some(line) = snap.progress_line {
            let progress = ServerToUi::BackupLog {
                hostname: snap.hostname,
                schedule_id: snap.schedule_id,
                repo_id: snap.repo_id,
                line,
            };
            send(sink, &progress).await?;
        }
    }
    Ok(())
}

async fn handle_ui_socket(socket: WebSocket, state: AppState, mut session: UiSession) {
    let mut rx = state.ui_broadcast.subscribe();
    let (mut sink, mut stream) = socket.split();

    let recv_task = async {
        while let Some(Ok(msg)) = stream.next().await {
            if matches!(msg, Message::Close(_)) {
                return;
            }
        }
    };

    tokio::select! {
        _ = stream_events(&mut sink, &mut rx, &state, &mut session) => {}
        () = recv_task => {}
    }
}

/// The next broadcast event, or `None` once the broadcast has closed.
///
/// A burst of events (e.g. `borg create --list` logging one line per file) can
/// outrun a single client's send loop faster than the broadcast channel's
/// fixed capacity. Dropping the connection then would be worse than the lag
/// itself: a `BackupStarted`/`BackupCompleted` lost in the gap has no other way
/// to reach the client, since nothing re-polls for it after page load. So a
/// lagged receiver skips past the gap and keeps receiving.
async fn next_event<T: Clone>(rx: &mut tokio::sync::broadcast::Receiver<T>) -> Option<T> {
    loop {
        match rx.recv().await {
            Ok(event) => return Some(event),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Replays the running operations, then forwards broadcast events `session`
/// may see until the server shuts down, the session ends or the client goes
/// away.
async fn stream_events(
    sink: &mut UiSink,
    rx: &mut tokio::sync::broadcast::Receiver<ServerToUi>,
    state: &AppState,
    session: &mut UiSession,
) -> Result<(), SocketGone> {
    replay_snapshots(sink, state, &session.audience).await?;
    let mut recheck = tokio::time::interval(session.recheck_every);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick completes at once; the session was just checked by
    // the upgrade, so the first re-check is one interval from now.
    recheck.tick().await;
    loop {
        tokio::select! {
            // Once the broadcast closes this branch stops matching, and the
            // socket stays open until the session ends or the server stops.
            Some(event) = next_event(rx) => {
                deliver(sink, &session.audience, &event).await?;
            }
            _ = recheck.tick() => {
                if !session.recheck(state).await {
                    // Best effort: the socket closes either way once this
                    // returns.
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: close_code::POLICY,
                            reason: "session ended".into(),
                        })))
                        .await;
                    return Ok(());
                }
            }
            () = state.shutdown_token.cancelled() => {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::StreamExt as _;
    use serde_json::json;
    use shared::{protocol::ServerToUi, time::Timestamp};
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};
    use tokio_util::sync::CancellationToken;

    use super::UiAudience;
    use crate::{api::tokens::hash_token, db, test_support::insert_auth_user};

    /// Serves `/ws/ui` on a loopback port, re-checking sessions every
    /// `recheck_every`, and returns the server's address.
    async fn serve_ui_socket(
        state: crate::AppState,
        recheck_every: Duration,
    ) -> std::net::SocketAddr {
        let app =
            axum::Router::new()
                .route(
                    "/ws/ui",
                    axum::routing::get(
                        move |ws: axum::extract::WebSocketUpgrade,
                              headers: axum::http::HeaderMap,
                              axum::extract::State(state): axum::extract::State<
                            crate::AppState,
                        >| async move {
                            super::upgrade_ui_socket(ws, &headers, state, recheck_every).await
                        },
                    ),
                )
                .with_state(state);
        crate::test_support::serve_on_loopback(app).await
    }

    /// Inserts a session for `user_id` and returns its cookie value.
    async fn insert_session(pool: &sqlx::PgPool, user_id: i64, pending_totp: bool) -> String {
        let cookie = format!("ui-handler-session-{user_id}-{pending_totp}");
        db::insert_session(
            pool,
            &hash_token(&cookie),
            user_id,
            Timestamp::now()
                .checked_add(shared::time::minutes(60))
                .expect("expiry in range"),
            false,
            pending_totp,
        )
        .await
        .expect("insert session");
        cookie
    }

    async fn connect(
        addr: std::net::SocketAddr,
        cookie: &str,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Box<tungstenite::Error>,
    > {
        let mut request = format!("ws://{addr}/ws/ui")
            .into_client_request()
            .expect("request");
        let cookie = format!("{}={cookie}", shared::cookies::SESSION_COOKIE);
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            cookie.parse().expect("cookie header"),
        );
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .map_err(Box::new)
    }

    fn refused_with(
        result: &Result<impl std::fmt::Debug, Box<tungstenite::Error>>,
        status: axum::http::StatusCode,
    ) -> bool {
        result.as_ref().err().is_some_and(|error| {
            matches!(&**error, tungstenite::Error::Http(response) if response.status() == status)
        })
    }

    /// A password-only login of a TOTP-protected account gets a session that
    /// is still waiting for the second factor. It must not open the live
    /// event stream, the same as it cannot call any other API.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_session_pending_totp_is_refused(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let user = insert_auth_user(&pool, "totp-pending").await;
        let cookie = insert_session(&pool, user.user_id, true).await;
        let addr = serve_ui_socket(state, Duration::from_secs(30)).await;

        let refused = connect(addr, &cookie).await;

        assert!(
            refused_with(&refused, axum::http::StatusCode::UNAUTHORIZED),
            "{refused:?}"
        );
    }

    /// A user who still has to change their password can reach only the
    /// password-change endpoints, not the live event stream.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_user_who_must_change_their_password_is_refused(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let user = insert_auth_user(&pool, "must-change").await;
        sqlx::query!(
            "UPDATE users SET must_change_password = true WHERE id = $1",
            user.user_id
        )
        .execute(&pool)
        .await
        .expect("require password change");
        let cookie = insert_session(&pool, user.user_id, false).await;
        let addr = serve_ui_socket(state, Duration::from_secs(30)).await;

        let refused = connect(addr, &cookie).await;

        assert!(
            refused_with(&refused, axum::http::StatusCode::FORBIDDEN),
            "{refused:?}"
        );
    }

    /// Logging out (or revoking or expiring the session) must end a socket
    /// that was opened with it, not leave it streaming for as long as the
    /// browser keeps it open.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_socket_closes_once_its_session_is_gone(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let user = insert_auth_user(&pool, "logs-out").await;
        let cookie = insert_session(&pool, user.user_id, false).await;
        let addr = serve_ui_socket(state, Duration::from_millis(50)).await;
        let socket = connect(addr, &cookie).await.expect("upgrade");

        db::delete_session(&pool, &hash_token(&cookie))
            .await
            .expect("log out");

        // Reads until the socket closes; no event is sent, so the only frame
        // is the close.
        let closed = tokio::time::timeout(Duration::from_secs(5), socket.collect::<Vec<_>>()).await;
        assert!(
            closed.is_ok(),
            "socket still open after its session was deleted"
        );
    }

    /// A user who cannot see an agent receives none of its live events (its
    /// backup log lines name the files it is backing up), while events for
    /// their own agent still arrive.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_user_receives_only_events_for_agents_they_can_see(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let viewer = insert_auth_user(&pool, "viewer").await;
        let other = insert_auth_user(&pool, "other").await;
        let visible = db::insert_agent(
            &pool,
            "viewer-host",
            None,
            "token-a",
            Some(viewer.user_id),
            None,
        )
        .await
        .expect("insert visible agent");
        let viewer_repo = insert_repo(&pool, "viewer-agent-repo", None).await;
        crate::test_support::let_user_see_agent(&pool, viewer.user_id, visible.id, viewer_repo)
            .await;
        db::insert_agent(
            &pool,
            "other-host",
            None,
            "token-b",
            Some(other.user_id),
            None,
        )
        .await
        .expect("insert hidden agent");
        let cookie = insert_session(&pool, viewer.user_id, false).await;
        let addr = serve_ui_socket(state.clone(), Duration::from_secs(30)).await;
        let mut socket = connect(addr, &cookie).await.expect("upgrade");

        state.ui_broadcast.send(ServerToUi::AgentConnected {
            hostname: "other-host".to_owned(),
        });
        state.ui_broadcast.send(ServerToUi::AgentConnected {
            hostname: "viewer-host".to_owned(),
        });

        assert_eq!(
            read_event(&mut socket).await,
            agent_connected("viewer-host")
        );
    }

    /// The JSON of an `AgentConnected` event for `hostname`.
    fn agent_connected(hostname: &str) -> serde_json::Value {
        json!({ "type": "AgentConnected", "payload": { "hostname": hostname } })
    }

    /// Reads the next text frame from `socket` as a UI event's JSON.
    async fn read_event(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> serde_json::Value {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("an event arrives")
            .expect("socket open")
            .expect("frame");
        serde_json::from_str(&frame.into_text().expect("a text frame")).expect("event json")
    }

    /// A browser that connects while a backup is running is shown the
    /// backup's start and latest progress line, but only for backups it may
    /// see: another user's host's file paths stay hidden.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connecting_user_is_replayed_only_their_own_running_backups(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let viewer = insert_auth_user(&pool, "replay-viewer").await;
        let other = insert_auth_user(&pool, "replay-other").await;
        let visible = db::insert_agent(
            &pool,
            "viewer-host",
            None,
            "token-a",
            Some(viewer.user_id),
            None,
        )
        .await
        .expect("insert visible agent");
        let own_repo = insert_repo(&pool, "viewer-repo", Some(viewer.user_id)).await;
        let hidden_repo = insert_repo(&pool, "other-repo", Some(other.user_id)).await;
        crate::test_support::let_user_see_agent(&pool, viewer.user_id, visible.id, own_repo).await;
        for (hostname, repo_id) in [("other-host", hidden_repo), ("viewer-host", own_repo)] {
            state.ui_broadcast.send(ServerToUi::BackupLog {
                hostname: hostname.to_owned(),
                schedule_id: None,
                repo_id,
                line: progress_line(hostname),
            });
        }
        let cookie = insert_session(&pool, viewer.user_id, false).await;
        let addr = serve_ui_socket(state, Duration::from_secs(30)).await;

        let mut socket = connect(addr, &cookie).await.expect("upgrade");

        let started = read_event(&mut socket).await;
        assert_eq!(started.pointer("/type"), Some(&json!("BackupStarted")));
        assert_eq!(
            started.pointer("/payload/hostname"),
            Some(&json!("viewer-host"))
        );
        let progress = read_event(&mut socket).await;
        assert_eq!(progress.pointer("/type"), Some(&json!("BackupLog")));
        assert_eq!(
            progress.pointer("/payload/line"),
            Some(&json!(progress_line("viewer-host")))
        );
    }

    /// An agent becoming visible to a user reaches a socket they already
    /// have open at its next re-check, without reconnecting.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_open_socket_picks_up_a_newly_visible_agent(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let viewer = insert_auth_user(&pool, "late-viewer").await;
        let cookie = insert_session(&pool, viewer.user_id, false).await;
        let addr = serve_ui_socket(state.clone(), Duration::from_millis(50)).await;
        let mut socket = connect(addr, &cookie).await.expect("upgrade");

        let late = db::insert_agent(
            &pool,
            "late-host",
            None,
            "token-late",
            Some(viewer.user_id),
            None,
        )
        .await
        .expect("insert agent");
        // A repository grant and a schedule backing the agent up are what
        // make it visible.
        let repo = insert_repo(&pool, "late-repo", None).await;
        crate::test_support::let_user_see_agent(&pool, viewer.user_id, late.id, repo).await;
        // Keep announcing until a re-check has reloaded the audience.
        let event = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                state.ui_broadcast.send(ServerToUi::AgentConnected {
                    hostname: "late-host".to_owned(),
                });
                if let Ok(event) =
                    tokio::time::timeout(Duration::from_millis(100), read_event(&mut socket)).await
                {
                    return event;
                }
            }
        })
        .await
        .expect("the newly visible agent's event arrives");

        assert_eq!(event, agent_connected("late-host"));
    }

    /// A complete `archive_progress` line naming a path on `hostname`, as
    /// borg reports it while a backup runs.
    fn progress_line(hostname: &str) -> String {
        format!(
            r#"{{"type":"archive_progress","nfiles":1,"original_size":1,"path":"/srv/{hostname}"}}"#
        )
    }

    /// Stopping the server ends every open UI socket.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_socket_closes_when_the_server_shuts_down(pool: sqlx::PgPool) {
        let state =
            crate::test_support::build_test_state(pool.clone(), b"ui-handler-test-key-material");
        let user = insert_auth_user(&pool, "shutdown-viewer").await;
        let cookie = insert_session(&pool, user.user_id, false).await;
        let addr = serve_ui_socket(state.clone(), Duration::from_secs(30)).await;
        let socket = connect(addr, &cookie).await.expect("upgrade");

        state.shutdown_token.cancel();

        let closed = tokio::time::timeout(Duration::from_secs(5), socket.collect::<Vec<_>>()).await;
        assert!(closed.is_ok(), "socket still open after shutdown");
    }

    /// A receiver that fell behind the broadcast skips the events it missed
    /// and goes on with the newest, instead of ending the socket; a closed
    /// broadcast ends it.
    #[tokio::test]
    async fn next_event_skips_a_lag_and_ends_on_close() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<i32>(2);
        for i in 0..5 {
            tx.send(i).expect("send");
        }
        drop(tx);

        assert_eq!(super::next_event(&mut rx).await, Some(3));
        assert_eq!(super::next_event(&mut rx).await, Some(4));
        assert_eq!(super::next_event(&mut rx).await, None);
    }

    /// Hiding an agent takes it out of a restricted user's live view, even
    /// though it still backs up to a repository they may read.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_hidden_agent_leaves_a_viewers_audience(pool: sqlx::PgPool) {
        let viewer = insert_auth_user(&pool, "hidden-viewer").await;
        let agent = db::insert_agent(&pool, "hidden-host", None, "token-h", None, None)
            .await
            .expect("insert agent");
        let repo = insert_repo(&pool, "hidden-host-repo", None).await;
        crate::test_support::let_user_see_agent(&pool, viewer.user_id, agent.id, repo).await;
        let audience = UiAudience::load(&pool, viewer.user_id)
            .await
            .expect("audience");
        assert!(audience.may_see_backup("hidden-host", repo));

        db::set_agent_hidden(&pool, agent.id, true)
            .await
            .expect("hide agent");
        let audience = UiAudience::load(&pool, viewer.user_id)
            .await
            .expect("audience");

        assert!(!audience.may_see_backup("hidden-host", repo));
        assert!(
            !crate::api::read_scope::visible_agents(&pool, viewer.user_id)
                .await
                .expect("scope")
                .allows(agent.id)
        );
    }

    /// Inserts a repository named `name` owned by `owner_id` and returns its id.
    async fn insert_repo(pool: &sqlx::PgPool, name: &str, owner_id: Option<i64>) -> i64 {
        db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name,
                repo_path: &format!("/backups/{name}"),
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"unused",
                compression: "lz4",
                encryption: "repokey",
                owner_id,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo")
        .id
    }

    /// A session cookie that names no session is refused before the upgrade,
    /// the same as no cookie at all. e2e only reaches this when a browser
    /// still holds a cookie for a session that has since ended, which made
    /// the line's coverage depend on test timing.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_cookie_for_no_session_is_refused(pool: sqlx::PgPool) {
        let state = crate::test_support::build_test_state(pool, b"ui-handler-test-key-material");
        let app = axum::Router::new()
            .route("/ws/ui", axum::routing::get(super::ui_ws_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("address");
        tokio::spawn(async move { axum::serve(listener, app).await });

        let mut request = format!("ws://{addr}/ws/ui")
            .into_client_request()
            .expect("request");
        let cookie = format!("{}=no-such-session", shared::cookies::SESSION_COOKIE);
        request.headers_mut().insert(
            axum::http::header::COOKIE,
            cookie.parse().expect("cookie header"),
        );

        let refused = tokio_tungstenite::connect_async(request).await;

        assert!(
            matches!(
                &refused,
                Err(tungstenite::Error::Http(response))
                    if response.status() == axum::http::StatusCode::UNAUTHORIZED
            ),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn send_task_exits_when_shutdown_token_cancelled() {
        let token = CancellationToken::new();
        let (tx, _rx) = tokio::sync::broadcast::channel::<&str>(2);

        let mut rx = tx.subscribe();

        let token_clone = token.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = rx.recv() => {}
                    () = token_clone.cancelled() => {
                        return;
                    }
                }
            }
        });

        token.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;

        assert!(
            result.is_ok(),
            "send_task did not exit within 5s after token cancellation"
        );
    }

    // Mirrors send_task's own recv-error handling: a lagged receiver must
    // keep consuming later messages instead of dropping the connection, since
    // a burst of BackupLog lines outrunning a slow client would otherwise
    // silently disconnect it mid-run and lose whatever it hasn't seen yet
    // (there is no other path that redelivers a missed BackupStarted).
    #[tokio::test]
    async fn lagged_receiver_keeps_going_instead_of_exiting() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<i32>(2);

        // Overflow the receiver's buffer (capacity 2) before it reads anything.
        for i in 0..5 {
            tx.send(i).unwrap();
        }

        let mut received = Vec::new();
        loop {
            match rx.recv().await {
                Ok(event) => received.push(event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
            if received.len() == 2 {
                break;
            }
        }

        // The oldest two of the five sent values were evicted by the lag; the
        // receiver must still have read past the gap to the newest ones
        // rather than exiting for good on the first Lagged.
        assert_eq!(received, vec![3, 4]);
    }
}

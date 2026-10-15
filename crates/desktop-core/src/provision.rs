// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use reqwest::{StatusCode, header};
use serde::{Deserialize, Serialize};
use shared::cookies::{SESSION_COOKIE, cookie_value};

use crate::secrets::Secret;

/// The built-in user every fresh server creates.
const ADMIN_USERNAME: &str = "admin";
/// The built-in user's password until it is first changed.
const BOOTSTRAP_ADMIN_PASSWORD: &str = "admin";
/// How often to poll `/api/health` while waiting for the server.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Why the local server couldn't be reached or set up.
#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// The HTTP request itself failed.
    #[error("request to the local server failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The server didn't answer `/api/health` in time.
    #[error("the local server did not become healthy within {0:?}")]
    NotHealthy(Duration),
    /// The server answered with a status this flow doesn't expect.
    #[error("{action} failed with HTTP {status}")]
    UnexpectedStatus {
        /// What was being attempted.
        action: &'static str,
        /// The status the server returned.
        status: StatusCode,
    },
    /// A login succeeded but carried no session cookie.
    #[error("the login response carried no session cookie")]
    NoSessionCookie,
    /// The admin account has two-factor authentication turned on, so a
    /// password alone never yields a session.
    #[error(
        "the admin account has two-factor authentication turned on, which the desktop app can't \
         sign in with"
    )]
    TwoFactorEnabled,
    /// The server is refusing admin logins for a while after too many
    /// failed attempts, e.g. mistyped passwords on the login page.
    #[error(
        "the server is refusing admin sign-ins after too many failed login attempts; try again in \
         15 minutes"
    )]
    LoginRateLimited,
    /// Neither the stored nor the bootstrap admin password was accepted.
    /// The usual cause is the app's keychain entries being lost or replaced
    /// while its data folder was kept, so the stored password is a new one
    /// the database never saw.
    #[error(
        "the admin account accepts neither the stored password nor the bootstrap one; this \
         happens when the app's `assimilate-desktop` keychain entries were removed or replaced \
         but its data folder was kept. Restore those keychain entries, or move the data folder \
         aside to start over"
    )]
    AdminLockedOut,
    /// The local server's address isn't a usable base URL.
    #[error("invalid local server URL {0}")]
    InvalidBaseUrl(String),
}

/// A logged-in admin session on the local server.
#[derive(Debug, Clone)]
pub struct AdminSession {
    cookie: Secret,
}

impl AdminSession {
    /// The session cookie's value, for handing to the webview.
    #[must_use]
    pub fn cookie_value(&self) -> &Secret {
        &self.cookie
    }

    fn header(&self) -> String {
        format!("{SESSION_COOKIE}={}", self.cookie.expose())
    }
}

/// The local agent's identity, as the server registered it.
#[derive(Debug, Clone)]
pub struct AgentCredentials {
    /// The hostname the agent must report (`BORG_HOSTNAME`).
    pub hostname: String,
    /// The freshly issued token (`BORG_AGENT_TOKEN`).
    pub token: Secret,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Serialize)]
struct ChangePasswordRequest<'a> {
    new_password: &'a str,
}

#[derive(Serialize)]
struct CreateAgentRequest<'a> {
    hostname: &'a str,
    display_name: &'a str,
}

/// The part of a successful login's body that explains a missing session.
#[derive(Deserialize)]
struct LoginOutcome {
    #[serde(default)]
    totp_required: bool,
}

#[derive(Deserialize)]
struct AgentTokenResponse {
    token: String,
}

/// Talks to the local server over loopback HTTP.
#[derive(Debug, Clone)]
pub struct LocalServer {
    base: reqwest::Url,
    http: reqwest::Client,
}

impl LocalServer {
    /// A client for the server listening on `127.0.0.1:port`.
    ///
    /// # Errors
    ///
    /// Fails if the HTTP client can't be built.
    pub fn new(port: u16) -> Result<Self, ProvisionError> {
        Self::with_base_url(format!("http://127.0.0.1:{port}"))
    }

    /// A client for an explicit base URL, for tests.
    ///
    /// # Errors
    ///
    /// Fails if the HTTP client can't be built.
    pub fn with_base_url(base_url: String) -> Result<Self, ProvisionError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let base =
            reqwest::Url::parse(&base_url).map_err(|_| ProvisionError::InvalidBaseUrl(base_url))?;
        Ok(Self { base, http })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base.as_str().trim_end_matches('/'))
    }

    /// `/api/agents/<hostname>/<action>`. The hostname is percent-encoded as a
    /// single path segment, so a `/` or `?` in it can't change which route
    /// is hit.
    fn agent_url(&self, hostname: &str, action: &str) -> Result<reqwest::Url, ProvisionError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| ProvisionError::InvalidBaseUrl(self.base.to_string()))?
            .pop_if_empty()
            .extend(["api", "agents", hostname, action]);
        Ok(url)
    }

    /// Polls `/api/health` until it answers 200, or `timeout` passes.
    ///
    /// # Errors
    ///
    /// Fails with [`ProvisionError::NotHealthy`] when time runs out.
    pub async fn wait_until_healthy(&self, timeout: Duration) -> Result<(), ProvisionError> {
        let poll = async {
            loop {
                let healthy = self
                    .http
                    .get(self.url("/api/health"))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success());
                if healthy {
                    return;
                }
                tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
            }
        };
        tokio::time::timeout(timeout, poll)
            .await
            .map_err(|_| ProvisionError::NotHealthy(timeout))
    }

    /// Logs in as the built-in admin, rotating its bootstrap password on
    /// first run.
    ///
    /// `password` must already be in the keychain before this is called, so
    /// a crash between rotating it here and storing it can't lock the app
    /// out: the next start tries the stored password first, then the
    /// bootstrap one. That costs at most one failed login, far below the
    /// server's lockout threshold.
    ///
    /// # Errors
    ///
    /// Fails if neither password works or the server misbehaves.
    pub async fn admin_session(&self, password: &Secret) -> Result<AdminSession, ProvisionError> {
        if let Some(session) = self.login(ADMIN_USERNAME, password.expose()).await? {
            return Ok(session);
        }
        let Some(bootstrap) = self.login(ADMIN_USERNAME, BOOTSTRAP_ADMIN_PASSWORD).await? else {
            return Err(ProvisionError::AdminLockedOut);
        };
        self.change_password(&bootstrap, password).await?;
        tracing::info!("rotated the built-in admin's bootstrap password");
        self.login(ADMIN_USERNAME, password.expose())
            .await?
            .ok_or(ProvisionError::AdminLockedOut)
    }

    /// Returns the session, or `None` if the credentials were rejected.
    async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<AdminSession>, ProvisionError> {
        let response = self
            .http
            .post(self.url("/api/auth/login"))
            .json(&LoginRequest { username, password })
            .send()
            .await?;
        match response.status() {
            StatusCode::UNAUTHORIZED => Ok(None),
            StatusCode::TOO_MANY_REQUESTS => Err(ProvisionError::LoginRateLimited),
            status if status.is_success() => match session_from(&response) {
                Err(ProvisionError::NoSessionCookie) => Err(missing_session_cause(response).await),
                session => session.map(Some),
            },
            status => Err(ProvisionError::UnexpectedStatus {
                action: "login",
                status,
            }),
        }
    }

    async fn change_password(
        &self,
        session: &AdminSession,
        new_password: &Secret,
    ) -> Result<(), ProvisionError> {
        let response = self
            .http
            .post(self.url("/api/auth/change-password"))
            .header(header::COOKIE, session.header())
            .json(&ChangePasswordRequest {
                new_password: new_password.expose(),
            })
            .send()
            .await?;
        expect_success("change-password", response.status())
    }

    /// Registers the local agent, or issues it a fresh token if it already
    /// exists. A new token every start means a token leaked from an earlier
    /// session is useless.
    ///
    /// # Errors
    ///
    /// Fails if the server rejects either request.
    pub async fn provision_agent(
        &self,
        session: &AdminSession,
        hostname: &str,
    ) -> Result<AgentCredentials, ProvisionError> {
        let regenerate = self
            .http
            .post(self.agent_url(hostname, "regenerate-token")?)
            .header(header::COOKIE, session.header())
            .send()
            .await?;
        let response = match regenerate.status() {
            StatusCode::NOT_FOUND => {
                let created = self
                    .http
                    .post(self.url("/api/agents"))
                    .header(header::COOKIE, session.header())
                    .json(&CreateAgentRequest {
                        hostname,
                        display_name: "This computer",
                    })
                    .send()
                    .await?;
                expect_success("create agent", created.status())?;
                created
            }
            status => {
                expect_success("regenerate agent token", status)?;
                regenerate
            }
        };
        let body: AgentTokenResponse = response.json().await?;
        Ok(AgentCredentials {
            hostname: hostname.to_owned(),
            token: Secret::from_stored(body.token),
        })
    }
}

fn expect_success(action: &'static str, status: StatusCode) -> Result<(), ProvisionError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(ProvisionError::UnexpectedStatus { action, status })
    }
}

fn session_from(response: &reqwest::Response) -> Result<AdminSession, ProvisionError> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(session_cookie_value)
        .map(|value| AdminSession {
            cookie: Secret::from_stored(value.to_owned()),
        })
        .ok_or(ProvisionError::NoSessionCookie)
}

/// Why a successful login carried no session: a second login step is
/// pending when the account uses two-factor authentication.
async fn missing_session_cause(response: reqwest::Response) -> ProvisionError {
    match response.json::<LoginOutcome>().await {
        Ok(LoginOutcome {
            totp_required: true,
        }) => ProvisionError::TwoFactorEnabled,
        _ => ProvisionError::NoSessionCookie,
    }
}

/// Extracts the session value from one `Set-Cookie` header.
fn session_cookie_value(set_cookie: &str) -> Option<&str> {
    cookie_value(set_cookie, SESSION_COOKIE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_value_reads_only_the_session_cookie() {
        assert_eq!(
            session_cookie_value("session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=60"),
            Some("abc")
        );
        assert_eq!(session_cookie_value("session=abc"), Some("abc"));
        assert_eq!(session_cookie_value("other=abc; Path=/"), None);
        assert_eq!(session_cookie_value("session=; Max-Age=0"), None);
        assert_eq!(session_cookie_value("garbage"), None);
    }

    #[test]
    fn admin_session_header_carries_the_cookie() {
        let session = AdminSession {
            cookie: Secret::from_stored("abc".to_owned()),
        };
        assert_eq!(session.header(), "session=abc");
        assert!(!format!("{session:?}").contains("abc"));
    }

    use std::collections::HashMap;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    /// A canned response: status, extra header lines, body.
    type Reply = (u16, &'static str, &'static str);

    /// Serves `routes` (request path to reply) over loopback HTTP/1.1 for
    /// exactly `requests` requests, one connection each; unknown paths get a
    /// 404. Await the handle to know every request was answered.
    async fn fake_server(
        requests: usize,
        routes: &[(&'static str, Reply)],
    ) -> (LocalServer, tokio::task::JoinHandle<()>) {
        let routes: HashMap<&'static str, Reply> = routes.iter().copied().collect();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let serve = tokio::spawn(async move {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let (status, headers, body) = routes.get(path).copied().unwrap_or((404, "", ""));
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: \
                     application/json\r\nConnection: close\r\n{headers}\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (LocalServer::new(port).unwrap(), serve)
    }

    /// Reads one whole request, headers and body, so the client never sees
    /// its connection closed mid-send.
    async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut eof = false;
        while !eof && !request_complete(&buf) {
            let read = stream.read(&mut chunk).await.unwrap();
            eof = read == 0;
            buf.extend_from_slice(chunk.get(..read).unwrap());
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn request_complete(buf: &[u8]) -> bool {
        let text = String::from_utf8_lossy(buf).to_ascii_lowercase();
        let length = text
            .split("content-length:")
            .nth(1)
            .and_then(|rest| rest.lines().next());
        let length = length
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let end = text.find("\r\n\r\n").map(|end| end.saturating_add(4));
        end.is_some_and(|end| buf.len() >= end.saturating_add(length))
    }

    fn error_text<T>(result: Result<T, ProvisionError>) -> Option<String> {
        result.err().map(|e| e.to_string())
    }

    #[tokio::test]
    async fn admin_session_fails_when_neither_password_is_accepted() {
        let (server, served) = fake_server(2, &[("/api/auth/login", (401, "", ""))]).await;

        let result = server.admin_session(&Secret::generate()).await;

        assert!(matches!(result, Err(ProvisionError::AdminLockedOut)));
        served.await.unwrap();
    }

    #[tokio::test]
    async fn a_rate_limited_login_is_reported_as_such() {
        let (server, served) = fake_server(1, &[("/api/auth/login", (429, "", ""))]).await;

        let result = server.admin_session(&Secret::generate()).await;

        assert!(matches!(result, Err(ProvisionError::LoginRateLimited)));
        served.await.unwrap();
    }

    #[tokio::test]
    async fn a_login_waiting_for_a_second_factor_is_reported_as_such() {
        let body = r#"{"totp_required":true,"temp_token":"pending"}"#;
        let (server, served) = fake_server(1, &[("/api/auth/login", (200, "", body))]).await;

        let result = server.admin_session(&Secret::generate()).await;

        assert!(matches!(result, Err(ProvisionError::TwoFactorEnabled)));
        served.await.unwrap();
    }

    #[tokio::test]
    async fn an_unexpected_login_status_is_reported_with_its_status() {
        let (server, served) = fake_server(1, &[("/api/auth/login", (500, "", ""))]).await;

        let result = server.admin_session(&Secret::generate()).await;

        assert_eq!(
            error_text(result).as_deref(),
            Some("login failed with HTTP 500 Internal Server Error")
        );
        served.await.unwrap();
    }

    #[tokio::test]
    async fn a_login_without_a_session_cookie_is_rejected() {
        let (server, served) = fake_server(1, &[("/api/auth/login", (200, "", "{}"))]).await;

        let result = server.admin_session(&Secret::generate()).await;

        assert!(matches!(result, Err(ProvisionError::NoSessionCookie)));
        served.await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_agent_registration_is_reported() {
        let (server, served) = fake_server(2, &[("/api/agents", (403, "", ""))]).await;
        let session = AdminSession {
            cookie: Secret::from_stored("abc".to_owned()),
        };

        let result = server.provision_agent(&session, "laptop").await;

        assert_eq!(
            error_text(result).as_deref(),
            Some("create agent failed with HTTP 403 Forbidden")
        );
        served.await.unwrap();
    }

    #[test]
    fn agent_urls_encode_the_hostname_as_one_path_segment() {
        let server = LocalServer::new(4321).unwrap();

        let url = server.agent_url("odd/host?x", "regenerate-token").unwrap();

        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:4321/api/agents/odd%2Fhost%3Fx/regenerate-token"
        );
    }

    #[test]
    fn an_invalid_base_url_is_rejected_up_front() {
        let result = LocalServer::with_base_url("not a url".to_owned());

        assert!(matches!(result, Err(ProvisionError::InvalidBaseUrl(url)) if url.eq("not a url")));
    }

    #[tokio::test]
    async fn wait_until_healthy_times_out_when_nothing_listens() {
        let port = crate::ports::free_loopback_port().unwrap();
        let server = LocalServer::new(port).unwrap();

        let result = server.wait_until_healthy(Duration::from_millis(300)).await;

        assert!(matches!(result, Err(ProvisionError::NotHealthy(_))));
    }
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A stand-in Kubernetes API server for tests: it lists pods and answers
//! exec requests over the `v5.channel.k8s.io` WebSocket subprotocol, so
//! [`crate::database::KubeExec`] can be exercised end to end without a cluster.
//! An exec that attaches stdin reads it to its end, which v5 marks with a
//! close frame for the stdin channel, before it answers.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use futures_util::{SinkExt as _, StreamExt as _};
use k8s_openapi::api::core::v1::Pod;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, handshake::derive_accept_key, protocol::Role},
};

use super::fake_server;

/// Channels of the Kubernetes exec stream.
const STDIN: u8 = 0;
const STDOUT: u8 = 1;
const STDERR: u8 = 2;
const STATUS: u8 = 3;

/// How one command run through the fake exec endpoint ends.
#[derive(Debug, Clone)]
pub enum Reply {
    /// Writes `stdout` and `stderr`, then reports success.
    Success { stdout: Vec<u8>, stderr: String },
    /// Writes `stdout` and `stderr`, then reports a non-zero exit.
    Failure { stdout: Vec<u8>, stderr: String },
    /// Writes `stdout`, then closes without a status, as a cut connection
    /// does.
    Cut { stdout: Vec<u8> },
    /// Writes `stdout`, then, once `received` is notified, resets the
    /// connection without a status, as a peer that drops its socket does: the
    /// client sees a transport error, not a close. Windows discards what the
    /// client hasn't read yet when the reset arrives, so the test notifies
    /// `received` once `stdout` has come through.
    Reset {
        stdout: Vec<u8>,
        received: Arc<tokio::sync::Notify>,
    },
}

impl Reply {
    pub fn ok(stdout: &[u8]) -> Self {
        Self::Success {
            stdout: stdout.to_vec(),
            stderr: String::new(),
        }
    }
}

#[derive(Default)]
struct State {
    pods: Vec<Pod>,
    /// Keyed by `<namespace>/<pod> <container> <last argument>`.
    replies: HashMap<String, Reply>,
    /// Status to answer the pod list with instead of the pods.
    list_status: Option<u16>,
    /// Every exec request seen, by the same key as `replies`.
    calls: Vec<String>,
    /// What each exec that attached stdin read on it, in order.
    inputs: Vec<Vec<u8>>,
}

/// A running fake API server.
pub struct FakeKube {
    state: Arc<Mutex<State>>,
    url: String,
}

impl FakeKube {
    pub async fn start(pods: Vec<Pod>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = fake_server::spawn(
            listener,
            State {
                pods,
                ..State::default()
            },
            serve,
        );
        Self { state, url }
    }

    /// Answers the exec of a command ending in `last_arg` in `pod`.
    pub fn reply(&self, namespace: &str, pod: &str, container: &str, last_arg: &str, reply: Reply) {
        self.state
            .lock()
            .unwrap()
            .replies
            .insert(format!("{namespace}/{pod} {container} {last_arg}"), reply);
    }

    /// Makes the pod list fail with `status`.
    pub fn fail_list(&self, status: u16) {
        self.state.lock().unwrap().list_status = Some(status);
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    /// What each exec that attached stdin read on it, in order.
    pub fn inputs(&self) -> Vec<Vec<u8>> {
        self.state.lock().unwrap().inputs.clone()
    }

    /// Writes a kubeconfig pointing at this server into `dir`.
    pub async fn kubeconfig(&self, dir: &Path) -> PathBuf {
        write_kubeconfig(dir, &serde_json::json!({ "server": self.url })).await
    }

    /// Writes a kubeconfig that reaches this server through the HTTP proxy
    /// at `proxy` into `dir`.
    pub async fn kubeconfig_via(&self, dir: &Path, proxy: &str) -> PathBuf {
        write_kubeconfig(
            dir,
            &serde_json::json!({ "server": self.url, "proxy-url": proxy }),
        )
        .await
    }
}

async fn write_kubeconfig(dir: &Path, cluster: &serde_json::Value) -> PathBuf {
    let config = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{ "name": "fake", "cluster": cluster }],
        "users": [{ "name": "fake", "user": {} }],
        "contexts": [{ "name": "fake", "context": { "cluster": "fake", "user": "fake" } }],
        "current-context": "fake",
    });
    let path = dir.join("kubeconfig");
    tokio::fs::write(&path, config.to_string()).await.unwrap();
    path
}

/// A forwarding proxy that tunnels each `CONNECT` it receives, remembering
/// the targets it was asked for.
pub struct ConnectProxy {
    pub url: String,
    targets: Arc<Mutex<Vec<String>>>,
}

impl ConnectProxy {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let targets = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&targets);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(tunnel(stream, Arc::clone(&seen)));
            }
        });
        Self { url, targets }
    }

    /// The `host:port` of every tunnel opened so far.
    pub fn targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }
}

async fn tunnel(stream: TcpStream, targets: Arc<Mutex<Vec<String>>>) {
    let mut client = BufReader::new(stream);
    let mut request = String::new();
    client.read_line(&mut request).await.unwrap();
    let target = request.split_whitespace().nth(1).unwrap().to_owned();
    let mut line = String::new();
    while client.read_line(&mut line).await.unwrap() > 2 {
        line.clear();
    }
    targets.lock().unwrap().push(target.clone());
    let mut upstream = TcpStream::connect(&target).await.unwrap();
    client
        .get_mut()
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .unwrap();
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

async fn serve(stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut ws_key = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("sec-websocket-key")
        {
            ws_key = Some(value.trim().to_owned());
        }
    }
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let stream = reader.into_inner();
    match ws_key {
        Some(key) => exec(stream, &key, path, query, &state).await,
        None => list(stream, &state).await,
    }
}

async fn list(mut stream: TcpStream, state: &Arc<Mutex<State>>) {
    let (status, body) = {
        let state = state.lock().unwrap();
        match state.list_status {
            Some(status) => (
                status,
                serde_json::json!({
                    "kind": "Status", "apiVersion": "v1", "status": "Failure",
                    "message": "pods is forbidden", "reason": "Forbidden", "code": status,
                }),
            ),
            None => (
                200,
                serde_json::json!({
                    "kind": "PodList", "apiVersion": "v1", "metadata": {},
                    "items": state.pods,
                }),
            ),
        }
    };
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
}

async fn exec(
    mut stream: TcpStream,
    key: &str,
    path: &str,
    query: &str,
    state: &Arc<Mutex<State>>,
) {
    // /api/v1/namespaces/<ns>/pods/<pod>/exec
    let parts: Vec<&str> = path.split('/').collect();
    let namespace = parts.get(4).copied().unwrap_or_default();
    let pod = parts.get(6).copied().unwrap_or_default();
    let params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect();
    let container = params
        .iter()
        .find(|(k, _)| k == "container")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let reads_stdin = params.iter().any(|(k, v)| k == "stdin" && v == "true");
    let last_arg = params
        .iter()
        .rfind(|(k, _)| k == "command")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let call = format!("{namespace}/{pod} {container} {last_arg}");
    let reply = {
        let mut state = state.lock().unwrap();
        state.calls.push(call.clone());
        state.replies.get(&call).cloned()
    };

    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: \
         Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: v5.channel.k8s.io\r\n\r\n",
        derive_accept_key(key.as_bytes())
    );
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let mut ws = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    if reads_stdin {
        // Until the close frame for stdin, `[255, 0]`, or the connection's end.
        let mut input = Vec::new();
        while let Some(Ok(Message::Binary(bytes))) = ws.next().await {
            match bytes.split_first() {
                Some((&STDIN, data)) => input.extend_from_slice(data),
                _ => break,
            }
        }
        state.lock().unwrap().inputs.push(input);
    }
    let frame = |channel: u8, data: &[u8]| {
        let mut bytes = vec![channel];
        bytes.extend_from_slice(data);
        Message::Binary(bytes.into())
    };
    let status = |success: bool| {
        let status = if success {
            serde_json::json!({ "metadata": {}, "status": "Success" })
        } else {
            serde_json::json!({
                "metadata": {}, "status": "Failure",
                "message": "command terminated with non-zero exit code: exit status 1",
                "reason": "NonZeroExitCode",
            })
        };
        frame(STATUS, status.to_string().as_bytes())
    };
    let reset = match &reply {
        Some(Reply::Reset { received, .. }) => Some(Arc::clone(received)),
        _ => None,
    };
    let frames = match reply {
        Some(Reply::Success { stdout, stderr }) => vec![
            frame(STDOUT, &stdout),
            frame(STDERR, stderr.as_bytes()),
            status(true),
        ],
        Some(Reply::Failure { stdout, stderr }) => vec![
            frame(STDOUT, &stdout),
            frame(STDERR, stderr.as_bytes()),
            status(false),
        ],
        Some(Reply::Cut { stdout } | Reply::Reset { stdout, .. }) => {
            vec![frame(STDOUT, &stdout)]
        }
        None => vec![
            frame(STDERR, format!("no reply scripted for {call}").as_bytes()),
            status(false),
        ],
    };
    for message in frames {
        if ws.send(message).await.is_err() {
            return;
        }
    }
    if let Some(received) = reset {
        received.notified().await;
        // A zero linger makes dropping the socket send a reset, not a FIN.
        let _ = ws.get_ref().set_zero_linger();
        return;
    }
    let _ = ws.close(None).await;
}

/// Decodes one `application/x-www-form-urlencoded` component.
pub(crate) fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(i.saturating_add(1)..i.saturating_add(3))
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(value) = hex {
                    out.push(value);
                    i = i.saturating_add(2);
                } else {
                    out.push(byte);
                }
            }
            other => out.push(other),
        }
        i = i.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

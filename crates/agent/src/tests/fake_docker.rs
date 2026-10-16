// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A stand-in Docker engine for tests: it lists containers and runs exec
//! requests over a Unix socket the way the engine's API does, so
//! [`crate::database::DockerExec`] can be exercised end to end without one.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bollard::models::ContainerSummary;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
    net::{UnixListener, UnixStream},
};

use super::{fake_kube::decode, fake_server};

/// How one command run through the fake exec endpoint ends.
#[derive(Debug, Clone)]
pub struct ExecReply {
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// The exit code the engine reports, `None` for none at all.
    pub exit_code: Option<i64>,
    /// How many times the exec is still reported running once its output
    /// has ended.
    pub running_polls: u32,
}

impl ExecReply {
    pub fn exit(code: i64, stdout: &[u8], stderr: &str) -> Self {
        Self {
            stdout: stdout.to_vec(),
            stderr: stderr.to_owned(),
            exit_code: Some(code),
            running_polls: 0,
        }
    }
}

#[derive(Default)]
struct State {
    containers: Vec<ContainerSummary>,
    /// Keyed by `<container id> <last argument>`.
    replies: HashMap<String, ExecReply>,
    /// The reply of each created exec, by its id.
    execs: HashMap<String, ExecReply>,
    calls: Vec<String>,
    list_query: String,
    inspections: u32,
    failure: Option<u16>,
}

/// A running fake engine.
pub struct FakeDocker {
    state: Arc<Mutex<State>>,
    _dir: tempfile::TempDir,
    socket: String,
}

impl FakeDocker {
    pub fn start(containers: Vec<ContainerSummary>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("docker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let state = fake_server::spawn(
            listener,
            State {
                containers,
                ..State::default()
            },
            serve,
        );
        Self {
            state,
            _dir: dir,
            socket: socket.to_string_lossy().into_owned(),
        }
    }

    pub fn socket(&self) -> String {
        self.socket.clone()
    }

    pub fn reply(&self, container: &str, last_arg: &str, reply: ExecReply) {
        self.state
            .lock()
            .unwrap()
            .replies
            .insert(format!("{container} {last_arg}"), reply);
    }

    /// Answers every request with `status` from now on.
    pub fn fail_requests(&self, status: u16) {
        self.state.lock().unwrap().failure = Some(status);
    }

    /// Every exec created, as `<container id> <command>`.
    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    /// The decoded query string of the last container list.
    pub fn list_query(&self) -> String {
        self.state.lock().unwrap().list_query.clone()
    }

    /// How many times an exec was inspected.
    pub fn inspections(&self) -> u32 {
        self.state.lock().unwrap().inspections
    }
}

/// One HTTP request: its method, path segments after the API version, query
/// and body.
struct Request {
    method: String,
    segments: Vec<String>,
    query: String,
    body: Vec<u8>,
}

async fn read_request(reader: &mut BufReader<UnixStream>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.ok()?;
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .skip_while(|segment| segment.starts_with('v'))
        .map(str::to_owned)
        .collect();
    Some(Request {
        method,
        segments,
        query: decode(query),
        body,
    })
}

async fn respond(stream: &mut BufReader<UnixStream>, status: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.get_mut().write_all(head.as_bytes()).await;
    let _ = stream.get_mut().write_all(body.as_bytes()).await;
}

async fn serve(stream: UnixStream, state: Arc<Mutex<State>>) {
    let mut stream = BufReader::new(stream);
    while let Some(request) = read_request(&mut stream).await {
        let failure = state.lock().unwrap().failure;
        if let Some(status) = failure {
            respond(&mut stream, status, r#"{"message":"engine failure"}"#).await;
            continue;
        }
        let segments: Vec<&str> = request.segments.iter().map(String::as_str).collect();
        match (request.method.as_str(), segments.as_slice()) {
            ("GET", ["containers", "json"]) => {
                let body = {
                    let mut state = state.lock().unwrap();
                    state.list_query = request.query.clone();
                    serde_json::to_string(&state.containers).unwrap()
                };
                respond(&mut stream, 200, &body).await;
            }
            ("POST", ["containers", container, "exec"]) => {
                let body = create_exec(&state, container, &request.body);
                respond(&mut stream, 201, &body).await;
            }
            ("POST", ["exec", id, "start"]) => {
                let reply = state.lock().unwrap().execs.get(*id).cloned();
                stream_exec(stream, reply.unwrap_or_else(unknown)).await;
                return;
            }
            ("GET", ["exec", id, "json"]) => {
                let body = inspect_exec(&state, id);
                respond(&mut stream, 200, &body).await;
            }
            _ => respond(&mut stream, 404, r#"{"message":"no such route"}"#).await,
        }
    }
}

fn unknown() -> ExecReply {
    ExecReply::exit(127, b"", "no such command in the script")
}

fn create_exec(state: &Arc<Mutex<State>>, container: &str, body: &[u8]) -> String {
    let config: serde_json::Value = serde_json::from_slice(body).unwrap();
    let command: Vec<String> = config
        .get("Cmd")
        .and_then(serde_json::Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut state = state.lock().unwrap();
    let key = format!(
        "{container} {}",
        command.last().cloned().unwrap_or_default()
    );
    let reply = state.replies.get(&key).cloned().unwrap_or_else(unknown);
    let id = format!("exec-{}", state.execs.len());
    state.execs.insert(id.clone(), reply);
    state
        .calls
        .push(format!("{container} {}", command.join(" ")));
    serde_json::json!({ "Id": id }).to_string()
}

/// Switches the connection to the raw stream the engine multiplexes stdout
/// and stderr on, writes the reply's output, and hangs up.
async fn stream_exec(mut stream: BufReader<UnixStream>, reply: ExecReply) {
    let head = "HTTP/1.1 101 UPGRADED\r\nContent-Type: \
                application/vnd.docker.multiplexed-stream\r\nConnection: Upgrade\r\nUpgrade: \
                tcp\r\n\r\n";
    let socket = stream.get_mut();
    let _ = socket.write_all(head.as_bytes()).await;
    for (channel, bytes) in [
        (1u8, reply.stdout.as_slice()),
        (2u8, reply.stderr.as_bytes()),
    ] {
        if bytes.is_empty() {
            continue;
        }
        let mut frame = vec![channel, 0, 0, 0];
        frame.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        frame.extend_from_slice(bytes);
        let _ = socket.write_all(&frame).await;
    }
    let _ = socket.shutdown().await;
}

fn inspect_exec(state: &Arc<Mutex<State>>, id: &str) -> String {
    let mut state = state.lock().unwrap();
    state.inspections = state.inspections.saturating_add(1);
    let reply = state.execs.get_mut(id).unwrap();
    if reply.running_polls > 0 {
        reply.running_polls = reply.running_polls.saturating_sub(1);
        return serde_json::json!({ "Running": true, "ExitCode": null }).to_string();
    }
    serde_json::json!({ "Running": false, "ExitCode": reply.exit_code }).to_string()
}

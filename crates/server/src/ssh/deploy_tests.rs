// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A whole deploy against a stand-in SSH host that records what it is sent
//! instead of running it: every command line the server runs there, what each
//! command reads on standard input, and every file uploaded over SFTP. Every
//! user on a real host can read the command lines, so the agent token and the
//! sudo password must never be in one.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use russh::{
    Channel, ChannelId,
    server::{Auth, Handler, Msg, Server, Session},
};
use russh_sftp::protocol::{FileAttributes, Handle, OpenFlags, Status, StatusCode, Version};
use tokio::net::TcpListener;

use super::{DeployAgentParams, deploy_agent, install_unit_cmd, sudo_input};

/// What the stand-in host was sent.
#[derive(Default)]
struct Recorded {
    /// Every command line run, in order.
    commands: Vec<String>,
    /// Each command that read standard input, with what it read.
    inputs: Vec<(String, Vec<u8>)>,
    /// Every file uploaded over SFTP, with its contents and the permissions
    /// it was created with.
    files: HashMap<String, (Vec<u8>, Option<u32>)>,
}

type Shared = Arc<Mutex<Recorded>>;

/// One SSH connection to the stand-in host.
struct Connection {
    recorded: Shared,
    sessions: HashMap<ChannelId, Channel<Msg>>,
    /// Commands waiting for the end of their standard input before they
    /// answer, with what they have read so far.
    reading: HashMap<ChannelId, (String, Vec<u8>)>,
}

impl Connection {
    /// Answers `command` as the real host would for a deploy: `uname -m`
    /// names the architecture, and everything else succeeds.
    fn answer(
        session: &mut Session,
        channel: ChannelId,
        command: &str,
    ) -> Result<(), russh::Error> {
        if command.starts_with("uname") {
            session.data(channel, b"x86_64\n".to_vec())?;
        }
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)
    }
}

impl Handler for Connection {
    type Error = russh::Error;

    fn auth_password(
        &mut self,
        _user: &str,
        _password: &str,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        std::future::ready(Ok(Auth::Accept))
    }

    fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.sessions.insert(channel.id(), channel);
        std::future::ready(Ok(true))
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(sftp) = self.sessions.remove(&channel) {
            let files = Files {
                recorded: Arc::clone(&self.recorded),
            };
            russh_sftp::server::run(sftp.into_stream(), files).await;
        }
        session.channel_success(channel)
    }

    fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.exec(channel, data, session))
    }

    fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        if let Some((_, input)) = self.reading.get_mut(&channel) {
            input.extend_from_slice(data);
        }
        std::future::ready(Ok(()))
    }

    fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.end_of_input(channel, session))
    }
}

impl Connection {
    /// Records `data` as a command and answers it, or, for one that reads
    /// standard input, waits for the end of it.
    fn exec(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.recorded
            .lock()
            .expect("recorded")
            .commands
            .push(command.clone());
        session.channel_success(channel)?;
        // Only sudo given a password reads standard input; it answers once
        // the input has ended.
        if command.starts_with("sudo -S") {
            self.reading.insert(channel, (command, Vec::new()));
            return Ok(());
        }
        Self::answer(session, channel, &command)
    }

    /// Records what the command on `channel` read, and answers it.
    fn end_of_input(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        if let Some((command, input)) = self.reading.remove(&channel) {
            self.recorded
                .lock()
                .expect("recorded")
                .inputs
                .push((command.clone(), input));
            Self::answer(session, channel, &command)?;
        }
        Ok(())
    }
}

/// The stand-in host's SFTP server, keeping uploads in memory.
struct Files {
    recorded: Shared,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_owned(),
        language_tag: "en-US".to_owned(),
    }
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> impl Future<Output = Result<Version, Self::Error>> + Send {
        std::future::ready(Ok(Version::new()))
    }

    fn open(
        &mut self,
        id: u32,
        filename: String,
        _flags: OpenFlags,
        attrs: FileAttributes,
    ) -> impl Future<Output = Result<Handle, Self::Error>> + Send {
        self.recorded
            .lock()
            .expect("recorded")
            .files
            .insert(filename.clone(), (Vec::new(), attrs.permissions));
        std::future::ready(Ok(Handle {
            id,
            handle: filename,
        }))
    }

    fn write(
        &mut self,
        id: u32,
        handle: String,
        _offset: u64,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        if let Some((contents, _)) = self
            .recorded
            .lock()
            .expect("recorded")
            .files
            .get_mut(&handle)
        {
            contents.extend_from_slice(&data);
        }
        std::future::ready(Ok(ok(id)))
    }

    fn close(
        &mut self,
        id: u32,
        _handle: String,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        std::future::ready(Ok(ok(id)))
    }
}

struct StandIn {
    recorded: Shared,
}

impl Server for StandIn {
    type Handler = Connection;

    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Connection {
        Connection {
            recorded: Arc::clone(&self.recorded),
            sessions: HashMap::new(),
            reading: HashMap::new(),
        }
    }
}

/// Starts the stand-in host on a loopback port, returning the port and what
/// it records.
async fn stand_in_host() -> (u16, Shared) {
    let recorded = Shared::default();
    let config = Arc::new(russh::server::Config {
        keys: vec![crate::test_support::generate_test_key()],
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("address").port();
    let mut server = StandIn {
        recorded: Arc::clone(&recorded),
    };
    tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    (port, recorded)
}

#[tokio::test]
async fn a_deploy_keeps_the_token_and_the_sudo_password_off_every_command_line() {
    let (port, recorded) = stand_in_host().await;
    let binaries = tempfile::tempdir().expect("tempdir");
    tokio::fs::write(binaries.path().join("agent-x86_64"), b"agent")
        .await
        .expect("write agent binary");

    deploy_agent(&DeployAgentParams {
        host: "127.0.0.1",
        user: "deployer",
        port,
        binary_dir: binaries.path(),
        remote_path: "/usr/local/bin/assimilate-agent",
        server_url: "wss://backup.example",
        token: "t0ken-s3cret",
        server_key: "S3rverKey=",
        password: Some("passw0rd-s3cret"),
        systemd_service_content: None,
    })
    .await
    .expect("deploy");

    let recorded = recorded.lock().expect("recorded");
    for command in &recorded.commands {
        assert!(!command.contains("t0ken-s3cret"), "{command}");
        assert!(!command.contains("passw0rd-s3cret"), "{command}");
    }
    // Every command run as root got the password on its standard input.
    let sudo_commands = recorded
        .commands
        .iter()
        .filter(|command| command.starts_with("sudo"))
        .count();
    assert_eq!(recorded.inputs.len(), sudo_commands);
    assert!(
        recorded
            .inputs
            .iter()
            .all(|(_, input)| *input == sudo_input("passw0rd-s3cret"))
    );
    // The unit holding the token was uploaded, readable by its owner only,
    // and installed from there.
    let (unit_path, (unit, permissions)) = recorded
        .files
        .iter()
        .find(|(path, _)| path.starts_with("/tmp/assimilate-agent-unit-"))
        .expect("the unit is uploaded");
    assert!(
        String::from_utf8_lossy(unit).contains("BORG_AGENT_TOKEN=t0ken-s3cret"),
        "{}",
        String::from_utf8_lossy(unit)
    );
    assert_eq!(*permissions, Some(0o600));
    let install = install_unit_cmd(unit_path);
    assert!(
        recorded.commands.iter().any(|command| command
            .contains(&super::shell_escape(&format!("exec </dev/null; {install}")))),
        "{:?}",
        recorded.commands
    );
}

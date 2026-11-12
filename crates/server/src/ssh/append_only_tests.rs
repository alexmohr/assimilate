// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Setting append-only up on a stand-in repository host that behaves the way
//! sshd does where it matters: a key is accepted only while its line is in
//! `~/.ssh/authorized_keys`, and a key forced to a command gets that command
//! instead of the SFTP subsystem. The order of the setup steps is what keeps
//! the server from locking itself out, so it is tested against a host that can
//! actually lock it out.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use russh::{
    Channel, ChannelId,
    server::{Auth, ChannelOpenHandle, Handler, Msg, Server, Session},
};
use russh_sftp::protocol::{
    Attrs, Data, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use ssh_key::PublicKey;
use tokio::net::TcpListener;

use super::{
    TEST_KEY_DIR,
    append_only::{Access, BACKUP_COPY, Login, SetupError, check, disable, enable},
    authorized_keys::{self, MAINTENANCE_COMMENT, Restriction},
    keys,
};

const AUTHORIZED_KEYS: &str = ".ssh/authorized_keys";
const PASSWORD: &str = "s3cret";

/// What the stand-in host holds: its files, as sshd's user would see them.
#[derive(Default)]
struct HostState {
    files: HashMap<String, Vec<u8>>,
    /// How many times a key that sshd accepted was then given the SFTP
    /// subsystem, and how many times it was refused it.
    sftp_granted: usize,
    sftp_refused: usize,
    /// Directories that exist, so making one again fails as it does on sshd.
    dirs: HashSet<String>,
    /// The commands run over `exec`, answered with success.
    commands: Vec<String>,
    faults: Faults,
}

/// Ways the host misbehaves on purpose, set per test.
#[derive(Default)]
pub(crate) struct Faults {
    /// A path whose `stat` is denied rather than answered.
    pub(crate) deny_stat: Option<String>,
    /// A path whose writes fail once this many have succeeded.
    pub(crate) fail_writes_to: Option<(String, usize)>,
    writes_seen: HashMap<String, usize>,
    /// The agents' key is refused while the file holds it this way.
    pub(crate) refuse_backup_when: Option<Restriction>,
    /// A password login is accepted but gets no SFTP.
    pub(crate) password_without_sftp: bool,
    /// A write is acknowledged but not kept, as a full disk or a misbehaving
    /// host might.
    pub(crate) drop_writes: bool,
    /// The same, for the one write to `authorized_keys` with this index (the
    /// first is 0): later writes are kept again.
    pub(crate) drop_keys_write: Option<usize>,
}

type Shared = Arc<Mutex<HostState>>;

impl HostState {
    fn authorized_keys(&self) -> String {
        self.files
            .get(AUTHORIZED_KEYS)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default()
    }
}

struct Connection {
    state: Shared,
    sessions: HashMap<ChannelId, Channel<Msg>>,
    /// Whether this connection may open a shell or SFTP: not for a key forced
    /// to a command.
    full_access: bool,
    /// Keys the host never accepts, to stand in for one it ignores.
    refused_keys: Arc<Vec<PublicKey>>,
    /// The key the agents use, for the faults that single it out.
    backup: PublicKey,
}

impl Handler for Connection {
    type Error = russh::Error;

    fn auth_password(
        &mut self,
        _user: &str,
        password: &str,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        let accepted = password == PASSWORD;
        let sftp = !self
            .state
            .lock()
            .expect("state")
            .faults
            .password_without_sftp;
        self.full_access = accepted && sftp;
        std::future::ready(Ok(if accepted {
            Auth::Accept
        } else {
            Auth::reject()
        }))
    }

    fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: russh::server::VerifiedPublicKey<'_>,
    ) -> impl Future<Output = Result<Auth, Self::Error>> + Send {
        let Ok(key) =
            PublicKey::from_openssh(&public_key.public_key.to_openssh().unwrap_or_default())
        else {
            return std::future::ready(Ok(Auth::reject()));
        };
        if self
            .refused_keys
            .iter()
            .any(|k| k.key_data() == key.key_data())
        {
            return std::future::ready(Ok(Auth::reject()));
        }
        let (text, refuse_when) = {
            let state = self.state.lock().expect("state");
            (
                state.authorized_keys(),
                state.faults.refuse_backup_when.clone(),
            )
        };
        let restriction = authorized_keys::restriction_of(&text, &key);
        if key.key_data() == self.backup.key_data() && refuse_when.as_ref() == Some(&restriction) {
            return std::future::ready(Ok(Auth::reject()));
        }
        let auth = match restriction {
            Restriction::Absent => Auth::reject(),
            Restriction::Unrestricted => {
                self.full_access = true;
                Auth::Accept
            }
            Restriction::AppendOnly | Restriction::Other => {
                self.full_access = false;
                Auth::Accept
            }
        };
        std::future::ready(Ok(auth))
    }

    keep_offered_channels!();

    fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        std::future::ready(self.exec(channel, data, session))
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let granted = self.full_access;
        {
            let mut state = self.state.lock().expect("state");
            if granted {
                state.sftp_granted = state.sftp_granted.saturating_add(1);
            } else {
                state.sftp_refused = state.sftp_refused.saturating_add(1);
            }
        }
        if !granted {
            // sshd runs the forced command instead; there is no SFTP.
            return session.channel_failure(channel);
        }
        if let Some(sftp) = self.sessions.remove(&channel) {
            session.channel_success(channel)?;
            russh_sftp::server::run(
                sftp.into_stream(),
                Files {
                    state: Arc::clone(&self.state),
                    listed: HashSet::new(),
                },
            )
            .await;
        }
        Ok(())
    }
}

impl Connection {
    /// Runs a command the way a shell login would: recorded, and answered with
    /// success, but not for a key forced to a command.
    fn exec(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        if !self.full_access {
            return session.channel_failure(channel);
        }
        self.state
            .lock()
            .expect("state")
            .commands
            .push(String::from_utf8_lossy(data).into_owned());
        session.channel_success(channel)?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)
    }
}

/// The stand-in host's files over SFTP.
struct Files {
    state: Shared,
    /// Directories whose single page of entries was already handed out.
    listed: HashSet<String>,
}

fn status(id: u32, code: StatusCode) -> Status {
    Status {
        id,
        status_code: code,
        error_message: format!("{code:?}"),
        language_tag: "en-US".to_owned(),
    }
}

impl russh_sftp::server::Handler for Files {
    sftp_handler_basics!();

    fn realpath(
        &mut self,
        id: u32,
        path: String,
    ) -> impl Future<Output = Result<Name, Self::Error>> + Send {
        std::future::ready(Ok(Name {
            id,
            files: vec![russh_sftp::protocol::File::dummy(path)],
        }))
    }

    fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        _attrs: FileAttributes,
    ) -> impl Future<Output = Result<Handle, Self::Error>> + Send {
        let mut state = self.state.lock().expect("state");
        let exists = state.files.contains_key(&filename);
        let result = if !exists && !flags.contains(OpenFlags::CREATE) {
            Err(StatusCode::NoSuchFile)
        } else {
            if !exists || flags.contains(OpenFlags::TRUNCATE) {
                state.files.insert(filename.clone(), Vec::new());
            }
            Ok(Handle {
                id,
                handle: filename,
            })
        };
        std::future::ready(result)
    }

    fn close(
        &mut self,
        id: u32,
        _handle: String,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        std::future::ready(Ok(status(id, StatusCode::Ok)))
    }

    fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> impl Future<Output = Result<Data, Self::Error>> + Send {
        let state = self.state.lock().expect("state");
        let bytes = state.files.get(&handle).cloned().unwrap_or_default();
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start
            .saturating_add(usize::try_from(len).unwrap_or(usize::MAX))
            .min(bytes.len());
        let result = match bytes.get(start..end) {
            Some(chunk) if !chunk.is_empty() => Ok(Data {
                id,
                data: chunk.to_vec(),
            }),
            _ => Err(StatusCode::Eof),
        };
        std::future::ready(result)
    }

    fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        let mut state = self.state.lock().expect("state");
        if state.faults.drop_writes {
            return std::future::ready(Ok(status(id, StatusCode::Ok)));
        }
        if let Some(dropped) = state.faults.drop_keys_write
            && handle == AUTHORIZED_KEYS
        {
            let seen = state.faults.writes_seen.entry(handle.clone()).or_default();
            let this = *seen;
            *seen = seen.saturating_add(1);
            if this == dropped {
                return std::future::ready(Ok(status(id, StatusCode::Ok)));
            }
        }
        if let Some((path, allowed)) = state.faults.fail_writes_to.clone()
            && path == handle
        {
            let seen = state.faults.writes_seen.entry(handle.clone()).or_default();
            if *seen >= allowed {
                return std::future::ready(Ok(status(id, StatusCode::Failure)));
            }
            *seen = seen.saturating_add(1);
        }
        let file = state.files.entry(handle).or_default();
        // As on a real host, a write replaces only the bytes it covers: what
        // lies beyond them stays, unless opening the file truncated it (see
        // `open`). That is what tells `create` from a plain write.
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(data.len());
        if file.len() < end {
            file.resize(end, 0);
        }
        if let Some(target) = file.get_mut(start..end) {
            target.copy_from_slice(&data);
        }
        std::future::ready(Ok(status(id, StatusCode::Ok)))
    }

    fn stat(
        &mut self,
        id: u32,
        path: String,
    ) -> impl Future<Output = Result<Attrs, Self::Error>> + Send {
        let state = self.state.lock().expect("state");
        if state.faults.deny_stat.as_deref() == Some(path.as_str()) {
            return std::future::ready(Err(StatusCode::PermissionDenied));
        }
        let exists = state.files.contains_key(&path);
        drop(state);
        std::future::ready(if exists {
            Ok(Attrs {
                id,
                attrs: FileAttributes::default(),
            })
        } else {
            Err(StatusCode::NoSuchFile)
        })
    }

    fn opendir(
        &mut self,
        id: u32,
        path: String,
    ) -> impl Future<Output = Result<Handle, Self::Error>> + Send {
        std::future::ready(Ok(Handle { id, handle: path }))
    }

    fn readdir(
        &mut self,
        id: u32,
        handle: String,
    ) -> impl Future<Output = Result<Name, Self::Error>> + Send {
        // Every directory holds one entry, handed out once, then the end.
        std::future::ready(if self.listed.insert(handle) {
            Ok(Name {
                id,
                files: vec![russh_sftp::protocol::File::dummy("repo")],
            })
        } else {
            Err(StatusCode::Eof)
        })
    }

    fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> impl Future<Output = Result<Status, Self::Error>> + Send {
        let created = self.state.lock().expect("state").dirs.insert(path);
        std::future::ready(if created {
            Ok(status(id, StatusCode::Ok))
        } else {
            Err(StatusCode::Failure)
        })
    }
}

struct StandIn {
    state: Shared,
    refused_keys: Arc<Vec<PublicKey>>,
    backup: PublicKey,
}

impl Server for StandIn {
    type Handler = Connection;

    fn new_client(&mut self, _peer: Option<std::net::SocketAddr>) -> Connection {
        Connection {
            state: Arc::clone(&self.state),
            sessions: HashMap::new(),
            full_access: false,
            refused_keys: Arc::clone(&self.refused_keys),
            backup: self.backup.clone(),
        }
    }
}

/// A stand-in host, the server's key directory, and what the host holds.
pub(crate) struct Fixture {
    pub(crate) port: u16,
    state: Shared,
    keys: tempfile::TempDir,
    backup: PublicKey,
}

pub(crate) struct HostOptions {
    /// What `authorized_keys` holds before anything is set up.
    pub(crate) authorized_keys: String,
    /// Whether the server's own key is in `authorized_keys` to begin with. A
    /// host that does not have it refuses the key until it is added.
    pub(crate) server_key_authorized: bool,
    /// Whether the host ignores the maintenance key.
    pub(crate) ignore_maintenance_key: bool,
    /// Whether the login has no `~/.ssh/authorized_keys` at all yet.
    pub(crate) no_authorized_keys: bool,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            authorized_keys: String::new(),
            server_key_authorized: true,
            ignore_maintenance_key: false,
            no_authorized_keys: false,
        }
    }
}

impl Fixture {
    /// Starts the host. The server's own key is in `authorized_keys` unless
    /// `options.authorized_keys` already has it.
    pub(crate) async fn start(options: HostOptions) -> Self {
        let keys = tempfile::tempdir().expect("key dir");
        let backup_private = crate::test_support::generate_ed25519_key();
        let mut backup = backup_private.public_key().clone();
        backup.set_comment("assimilate-server");
        tokio::fs::write(
            keys.path().join("id_ed25519"),
            backup_private
                .to_openssh(ssh_key::LineEnding::LF)
                .expect("encode"),
        )
        .await
        .expect("write key");
        tokio::fs::write(
            keys.path().join("id_ed25519.pub"),
            format!("{}\n", backup.to_openssh().expect("encode")),
        )
        .await
        .expect("write public key");

        let mut refused = Vec::new();
        if options.ignore_maintenance_key {
            let maintenance = TEST_KEY_DIR
                .scope(keys.path().to_owned(), keys::ensure_maintenance_key())
                .await
                .expect("maintenance key");
            refused.push(maintenance.public_key().clone());
        }

        let state = Shared::default();
        let text = if options.authorized_keys.is_empty() {
            if options.server_key_authorized {
                format!("{}\n", backup.to_openssh().expect("encode"))
            } else {
                String::new()
            }
        } else {
            options
                .authorized_keys
                .replace("{backup}", &backup.to_openssh().expect("encode"))
        };
        {
            let mut held = state.lock().expect("state");
            if !options.no_authorized_keys {
                held.files
                    .insert(AUTHORIZED_KEYS.to_owned(), text.into_bytes());
                held.dirs.insert(".ssh".to_owned());
            }
        }

        let config = Arc::new(russh::server::Config {
            keys: vec![crate::test_support::generate_test_key()],
            auth_rejection_time: std::time::Duration::from_millis(1),
            auth_rejection_time_initial: Some(std::time::Duration::from_millis(1)),
            ..Default::default()
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let mut server = StandIn {
            state: Arc::clone(&state),
            refused_keys: Arc::new(refused),
            backup: backup.clone(),
        };
        tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });
        Self {
            port,
            state,
            keys,
            backup,
        }
    }

    pub(crate) fn login(&self) -> Login<'static> {
        Login {
            host: "127.0.0.1",
            user: "borg",
            port: self.port,
            host_key: None,
        }
    }

    /// Makes the host misbehave from now on.
    pub(crate) fn fault(&self, set: impl FnOnce(&mut Faults)) {
        set(&mut self.state.lock().expect("state").faults);
    }

    pub(crate) fn key_dir(&self) -> PathBuf {
        self.keys.path().to_owned()
    }

    pub(crate) fn authorized_keys(&self) -> String {
        self.state.lock().expect("state").authorized_keys()
    }

    /// How many writes to `authorized_keys` were counted since
    /// `drop_keys_write` was set.
    fn keys_writes(&self) -> usize {
        let state = self.state.lock().expect("state");
        state
            .faults
            .writes_seen
            .get(AUTHORIZED_KEYS)
            .copied()
            .unwrap_or(0)
    }

    /// The commands run on the host over `exec`.
    pub(crate) fn commands(&self) -> Vec<String> {
        self.state.lock().expect("state").commands.clone()
    }

    fn file(&self, path: &str) -> Option<String> {
        self.state
            .lock()
            .expect("state")
            .files
            .get(path)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn sftp_refused(&self) -> usize {
        self.state.lock().expect("state").sftp_refused
    }

    fn set_authorized_keys(&self, text: &str) {
        self.state
            .lock()
            .expect("state")
            .files
            .insert(AUTHORIZED_KEYS.to_owned(), text.as_bytes().to_vec());
    }

    async fn maintenance(&self) -> PublicKey {
        TEST_KEY_DIR
            .scope(self.key_dir(), keys::ensure_maintenance_key())
            .await
            .expect("maintenance key")
            .public_key()
            .clone()
    }

    /// Runs `future` with the server's key directory pointing at this fixture.
    pub(crate) async fn scoped<T>(&self, future: impl Future<Output = T>) -> T {
        TEST_KEY_DIR.scope(self.key_dir(), future).await
    }
}

fn paths(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

fn lines_with(text: &str, needle: &str) -> usize {
    text.lines().filter(|l| l.contains(needle)).count()
}

#[tokio::test]
async fn turning_it_on_restricts_the_servers_key_and_lets_the_maintenance_key_in() {
    let host = Fixture::start(HostOptions::default()).await;

    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");

    let text = host.authorized_keys();
    let maintenance = host.maintenance().await;
    assert_eq!(
        authorized_keys::restriction_of(&text, &host.backup),
        Restriction::AppendOnly
    );
    assert_eq!(
        authorized_keys::restriction_of(&text, &maintenance),
        Restriction::Unrestricted,
        "the maintenance key keeps full access"
    );
    assert!(
        text.contains("--restrict-to-repository '/srv/borg/a'"),
        "{text}"
    );
    assert_eq!(lines_with(&text, MAINTENANCE_COMMENT), 1);
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[tokio::test]
async fn the_file_as_it_was_is_kept_once_and_other_keys_survive() {
    let original = "# deploy keys\ncommand=\"/bin/true\",restrict ssh-ed25519 \
                    AAAAC3NzaC1lZDI1NTE5AAAAIOtherKeyOtherKeyOtherKeyOtherKeyOtherKeyOtherKey0 \
                    alice@laptop\n{backup}\n";
    let host = Fixture::start(HostOptions {
        authorized_keys: original.to_owned(),
        ..HostOptions::default()
    })
    .await;
    let before = host.authorized_keys();

    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");

    let text = host.authorized_keys();
    assert!(
        text.starts_with("# deploy keys\ncommand=\"/bin/true\",restrict ssh-ed25519 "),
        "{text}"
    );
    assert!(text.contains("alice@laptop"));
    assert_eq!(
        host.file(BACKUP_COPY),
        Some(before.clone()),
        "the first state is kept"
    );

    // A second run does not replace the copy with the first run's result.
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a", "/srv/borg/b"]),
        Access::ServerKey,
    ))
    .await
    .expect("reapply");
    assert_eq!(host.file(BACKUP_COPY), Some(before));
}

#[tokio::test]
async fn a_new_repository_is_let_in_through_the_maintenance_key() {
    let host = Fixture::start(HostOptions::default()).await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");
    // The server's own key is restricted now, so only the maintenance key can
    // edit the file: this is what lets a second repository be added.
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a", "/srv/borg/b"]),
        Access::ServerKey,
    ))
    .await
    .expect("reapply through the maintenance key");
    let text = host.authorized_keys();
    assert!(
        text.contains("'/srv/borg/a'") && text.contains("'/srv/borg/b'"),
        "{text}"
    );
    assert_eq!(text.lines().count(), 2, "{text}");
}

#[tokio::test]
async fn a_host_that_ignores_the_maintenance_key_is_left_as_it_was() {
    let host = Fixture::start(HostOptions {
        ignore_maintenance_key: true,
        ..HostOptions::default()
    })
    .await;
    let before = host.authorized_keys();

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::MaintenanceKeyRejected)),
        "{result:?}"
    );
    assert_eq!(
        host.authorized_keys(),
        before,
        "the server's key was never restricted"
    );
    // The agents' key still works as before.
    assert_eq!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::Unrestricted
    );
}

#[tokio::test]
async fn the_servers_key_is_never_restricted_before_the_maintenance_key_is_proven() {
    // If the order were the other way round this host would be locked: the
    // server key restricted, the maintenance key not accepted.
    let host = Fixture::start(HostOptions {
        ignore_maintenance_key: true,
        ..HostOptions::default()
    })
    .await;

    let _ = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert_ne!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::AppendOnly
    );
}

#[tokio::test]
async fn a_host_that_refuses_the_servers_key_asks_for_the_password_once() {
    let host = Fixture::start(HostOptions {
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;
    assert!(
        matches!(result, Err(SetupError::NeedsPassword)),
        "{result:?}"
    );

    // With the password it works; the key is accepted for the agents again
    // afterwards because the host is told to accept it, restricted.
    let host_with_key_in_file = host;
    host_with_key_in_file
        .scoped(enable(
            host_with_key_in_file.login(),
            &paths(&["/srv/borg/a"]),
            Access::Password(PASSWORD),
        ))
        .await
        .expect("enable with the password");
    assert_eq!(
        authorized_keys::restriction_of(
            &host_with_key_in_file.authorized_keys(),
            &host_with_key_in_file.backup
        ),
        Restriction::AppendOnly
    );
}

#[tokio::test]
async fn a_wrong_password_is_reported_and_changes_nothing() {
    let host = Fixture::start(HostOptions {
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;
    let before = host.authorized_keys();

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::Password("wrong"),
        ))
        .await;

    assert!(matches!(result, Err(SetupError::Ssh(_))), "{result:?}");
    assert_eq!(host.authorized_keys(), before);
}

#[tokio::test]
async fn a_host_that_does_not_answer_is_not_mistaken_for_a_refused_key() {
    let host = Fixture::start(HostOptions::default()).await;
    let closed = Login {
        port: 1,
        ..host.login()
    };

    let result = host
        .scoped(enable(closed, &paths(&["/srv/borg/a"]), Access::ServerKey))
        .await;

    assert!(matches!(result, Err(SetupError::Ssh(_))), "{result:?}");
}

#[tokio::test]
async fn a_path_that_cannot_go_in_a_forced_command_is_refused_before_the_host_is_touched() {
    let host = Fixture::start(HostOptions::default()).await;
    let before = host.authorized_keys();

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/bo\"rg"]),
            Access::ServerKey,
        ))
        .await;

    assert!(matches!(result, Err(SetupError::Keys(_))), "{result:?}");
    assert_eq!(host.authorized_keys(), before);
    assert!(host.file(BACKUP_COPY).is_none());
}

#[tokio::test]
async fn turning_it_off_restores_the_plain_key_and_removes_the_maintenance_key() {
    let host = Fixture::start(HostOptions {
        authorized_keys: "ssh-ed25519 \
             AAAAC3NzaC1lZDI1NTE5AAAAIOtherKeyOtherKeyOtherKeyOtherKeyOtherKeyOtherKey0 \
             bob\n{backup}\n"
            .to_owned(),
        ..HostOptions::default()
    })
    .await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");

    host.scoped(disable(host.login(), Access::ServerKey))
        .await
        .expect("disable");

    let text = host.authorized_keys();
    assert_eq!(
        authorized_keys::restriction_of(&text, &host.backup),
        Restriction::Unrestricted
    );
    assert!(!text.contains(MAINTENANCE_COMMENT), "{text}");
    assert!(text.contains("bob"), "{text}");
    assert!(!text.contains("command="), "{text}");
}

#[tokio::test]
async fn the_check_reads_the_file_and_tries_both_keys() {
    let host = Fixture::start(HostOptions::default()).await;
    let repo = paths(&["/srv/borg/a"]);
    host.scoped(enable(host.login(), &repo, Access::ServerKey))
        .await
        .expect("enable");

    let result = host.scoped(check(host.login(), &repo)).await;

    assert!(result.backup_key_ok);
    assert!(result.maintenance_key_ok);
    assert!(result.restriction_in_place);
    assert_eq!(result.error, None);
}

#[tokio::test]
async fn the_check_notices_that_the_restriction_was_removed_by_hand() {
    let host = Fixture::start(HostOptions::default()).await;
    let repo = paths(&["/srv/borg/a"]);
    host.scoped(enable(host.login(), &repo, Access::ServerKey))
        .await
        .expect("enable");
    let maintenance = host.maintenance().await;
    // Someone put the agents' key back as a plain line.
    host.set_authorized_keys(&format!(
        "{}\n{}\n",
        host.backup.to_openssh().expect("encode"),
        authorized_keys::maintenance_line(&maintenance)
    ));

    let result = host.scoped(check(host.login(), &repo)).await;

    assert!(result.backup_key_ok && result.maintenance_key_ok);
    assert!(!result.restriction_in_place);
}

#[tokio::test]
async fn the_check_notices_a_repository_the_restriction_does_not_cover() {
    let host = Fixture::start(HostOptions::default()).await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");

    let result = host
        .scoped(check(
            host.login(),
            &paths(&["/srv/borg/a", "/srv/borg/new"]),
        ))
        .await;

    assert!(!result.restriction_in_place);
}

#[tokio::test]
async fn the_check_reports_a_maintenance_key_the_host_no_longer_accepts() {
    let host = Fixture::start(HostOptions::default()).await;
    let repo = paths(&["/srv/borg/a"]);
    host.scoped(enable(host.login(), &repo, Access::ServerKey))
        .await
        .expect("enable");
    // The maintenance line was deleted from the file.
    host.set_authorized_keys("");

    let result = host.scoped(check(host.login(), &repo)).await;

    assert!(!result.maintenance_key_ok);
    assert!(!result.restriction_in_place);
    assert!(result.error.is_some());
}

#[tokio::test]
async fn a_restricted_key_is_not_given_the_sftp_subsystem_by_the_stand_in() {
    // Guards the stand-in itself: the tests above are only meaningful if a
    // restricted key really cannot use SFTP, as on a real host.
    let host = Fixture::start(HostOptions::default()).await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");
    let before = host.sftp_refused();

    let session = host
        .scoped(super::connect_as(
            super::KeyRole::Backup,
            "127.0.0.1",
            "borg",
            host.port,
            None,
        ))
        .await
        .expect("the restricted key still authenticates");
    let sftp = super::open_sftp(&session).await;

    assert!(
        sftp.is_err(),
        "a key forced to borg serve must not get SFTP"
    );
    assert_eq!(host.sftp_refused(), before + 1);
    let _ = Path::new(AUTHORIZED_KEYS);
}

#[tokio::test]
async fn a_server_key_that_is_already_restricted_asks_for_the_password() {
    // The login was restricted earlier and this process does not know it yet,
    // so it tries the server's key, which authenticates but gets no SFTP.
    let host = Fixture::start(HostOptions {
        authorized_keys: "command=\"borg serve --append-only --restrict-to-repository \
                          '/srv/borg/a'\",restrict {backup}\n"
            .to_owned(),
        ..HostOptions::default()
    })
    .await;
    let before = host.authorized_keys();

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::NeedsPassword)),
        "{result:?}"
    );
    assert_eq!(host.authorized_keys(), before, "nothing was changed");
}

#[tokio::test]
async fn a_host_that_does_not_keep_what_is_written_is_reported() {
    let host = Fixture::start(HostOptions::default()).await;
    host.fault(|f| f.drop_writes = true);

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(matches!(result, Err(SetupError::NotWritten)), "{result:?}");
    assert!(
        !host.authorized_keys().contains("--append-only"),
        "the server's key is never restricted on the strength of a write that did not stick"
    );
}

#[tokio::test]
async fn a_maintenance_key_that_does_not_stick_is_reported_and_the_file_is_put_back() {
    let host = Fixture::start(HostOptions::default()).await;
    let before = host.authorized_keys();
    // The very first write to the file is the one adding the maintenance key;
    // dropping it leaves the file empty until it is put back.
    host.fault(|f| f.drop_keys_write = Some(0));

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(matches!(result, Err(SetupError::NotWritten)), "{result:?}");
    assert_eq!(
        host.authorized_keys(),
        before,
        "the host's file is as it was, not left empty"
    );
}

#[tokio::test]
async fn a_restriction_that_does_not_stick_is_reported_and_the_file_is_put_back() {
    let host = Fixture::start(HostOptions::default()).await;
    // The first write adds the maintenance key; the one restricting the key is
    // dropped, which leaves the file empty until it is put back.
    host.fault(|f| f.drop_keys_write = Some(1));

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(matches!(result, Err(SetupError::NotWritten)), "{result:?}");
    assert_eq!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::Unrestricted,
        "the server's key is plain again, not gone"
    );
}

#[tokio::test]
async fn turning_it_off_with_a_write_that_does_not_stick_puts_the_file_back() {
    let host = Fixture::start(HostOptions::default()).await;
    // Count the writes to the file without dropping any.
    host.fault(|f| f.drop_keys_write = Some(usize::MAX));
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");
    let restricted = host.authorized_keys();
    // The next write to the file is the one turning it off.
    let next = host.keys_writes();
    host.fault(|f| f.drop_keys_write = Some(next));

    let result = host.scoped(disable(host.login(), Access::ServerKey)).await;

    assert!(matches!(result, Err(SetupError::NotWritten)), "{result:?}");
    assert_eq!(host.authorized_keys(), restricted);
}

#[tokio::test]
async fn a_login_without_an_authorized_keys_file_gets_one() {
    let host = Fixture::start(HostOptions {
        no_authorized_keys: true,
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;

    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::Password(PASSWORD),
    ))
    .await
    .expect("enable from nothing");

    assert_eq!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::AppendOnly
    );
}

#[tokio::test]
async fn an_authorized_keys_file_that_cannot_be_looked_up_is_an_error() {
    let host = Fixture::start(HostOptions::default()).await;
    host.fault(|f| f.deny_stat = Some(AUTHORIZED_KEYS.to_owned()));

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::Ssh(super::SshError::Sftp(_)))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_copy_that_cannot_be_kept_does_not_stop_the_setup() {
    for fault in ["stat", "write"] {
        let host = Fixture::start(HostOptions::default()).await;
        host.fault(|f| {
            if fault == "stat" {
                f.deny_stat = Some(BACKUP_COPY.to_owned());
            } else {
                f.fail_writes_to = Some((BACKUP_COPY.to_owned(), 0));
            }
        });

        host.scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await
        .unwrap_or_else(|e| panic!("{fault}: {e:?}"));

        assert_eq!(
            authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
            Restriction::AppendOnly,
            "{fault}"
        );
    }
}

#[tokio::test]
async fn a_password_login_without_sftp_is_an_ssh_error_not_a_request_for_the_password() {
    let host = Fixture::start(HostOptions {
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;
    host.fault(|f| f.password_without_sftp = true);

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::Password(PASSWORD),
        ))
        .await;

    assert!(matches!(result, Err(SetupError::Ssh(_))), "{result:?}");
}

#[tokio::test]
async fn a_rejected_maintenance_key_is_taken_back_out_even_if_putting_the_file_back_fails() {
    let host = Fixture::start(HostOptions {
        ignore_maintenance_key: true,
        ..HostOptions::default()
    })
    .await;
    let before = host.authorized_keys();
    // The first write adds the maintenance key; putting the file back fails.
    host.fault(|f| f.fail_writes_to = Some((AUTHORIZED_KEYS.to_owned(), 1)));

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::MaintenanceKeyRejected)),
        "{result:?}"
    );
    assert_ne!(
        host.authorized_keys(),
        before,
        "the host was left to the admin"
    );
}

#[tokio::test]
async fn a_host_that_refuses_the_restricted_key_is_put_back() {
    let host = Fixture::start(HostOptions::default()).await;
    host.fault(|f| f.refuse_backup_when = Some(Restriction::AppendOnly));

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::BackupKeyRejected)),
        "{result:?}"
    );
    assert_eq!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::Unrestricted,
        "the agents' key works again"
    );
}

#[tokio::test]
async fn a_host_that_refuses_the_restricted_key_and_the_undo_is_reported_as_refused() {
    let host = Fixture::start(HostOptions::default()).await;
    host.fault(|f| {
        f.refuse_backup_when = Some(Restriction::AppendOnly);
        // The maintenance key and the restriction go in; the undo does not.
        f.fail_writes_to = Some((AUTHORIZED_KEYS.to_owned(), 2));
    });

    let result = host
        .scoped(enable(
            host.login(),
            &paths(&["/srv/borg/a"]),
            Access::ServerKey,
        ))
        .await;

    assert!(
        matches!(result, Err(SetupError::BackupKeyRejected)),
        "{result:?}"
    );
}

#[tokio::test]
async fn turning_it_off_puts_the_restriction_back_when_the_host_refuses_the_plain_key() {
    let host = Fixture::start(HostOptions::default()).await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");
    let enabled = host.authorized_keys();
    host.fault(|f| f.refuse_backup_when = Some(Restriction::Unrestricted));

    let result = host.scoped(disable(host.login(), Access::ServerKey)).await;

    assert!(
        matches!(result, Err(SetupError::BackupKeyRejected)),
        "{result:?}"
    );
    assert_eq!(
        host.authorized_keys(),
        enabled,
        "the host still enforces what the database says it enforces"
    );
}

#[tokio::test]
async fn turning_it_off_reports_a_host_that_refuses_the_plain_key_and_the_undo() {
    let host = Fixture::start(HostOptions::default()).await;
    host.scoped(enable(
        host.login(),
        &paths(&["/srv/borg/a"]),
        Access::ServerKey,
    ))
    .await
    .expect("enable");
    host.fault(|f| {
        f.refuse_backup_when = Some(Restriction::Unrestricted);
        // Turning it off writes once; putting the file back would be the
        // second, which fails.
        f.fail_writes_to = Some((AUTHORIZED_KEYS.to_owned(), 1));
    });

    let result = host.scoped(disable(host.login(), Access::ServerKey)).await;

    assert!(
        matches!(result, Err(SetupError::BackupKeyRejected)),
        "{result:?}"
    );
}

#[tokio::test]
async fn the_stand_in_answers_an_sftp_request_it_does_not_implement_with_an_error() {
    let host = Fixture::start(HostOptions::default()).await;
    let session = host
        .scoped(super::connect_as(
            super::KeyRole::Backup,
            "127.0.0.1",
            "borg",
            host.port,
            None,
        ))
        .await
        .expect("the unrestricted key authenticates");
    let sftp = super::open_sftp(&session).await.expect("sftp");

    assert!(sftp.remove_file("anything").await.is_err());
}

fn deploy_request(port: u16) -> super::DeployKeyRequest {
    super::DeployKeyRequest {
        ssh_host: "127.0.0.1".to_owned(),
        ssh_user: "borg".to_owned(),
        ssh_port: Some(port),
        password: PASSWORD.to_owned(),
        use_sftp: true,
    }
}

#[tokio::test]
async fn deploying_the_servers_key_puts_it_in_authorized_keys_and_proves_it_works() {
    let host = Fixture::start(HostOptions {
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;

    let response = host
        .scoped(super::deploy_key(&deploy_request(host.port)))
        .await;

    assert!(response.success, "{:?}", response.error);
    assert!(!response.already_deployed);
    assert_eq!(
        authorized_keys::restriction_of(&host.authorized_keys(), &host.backup),
        Restriction::Unrestricted
    );
}

#[tokio::test]
async fn a_key_that_is_already_deployed_is_left_alone() {
    let host = Fixture::start(HostOptions::default()).await;
    let before = host.authorized_keys();

    let response = host
        .scoped(super::deploy_key(&deploy_request(host.port)))
        .await;

    assert!(response.success && response.already_deployed);
    assert_eq!(host.authorized_keys(), before);
}

#[tokio::test]
async fn a_key_the_host_then_refuses_is_reported_as_uploaded_but_unproven() {
    let host = Fixture::start(HostOptions {
        server_key_authorized: false,
        ..HostOptions::default()
    })
    .await;
    host.fault(|f| f.refuse_backup_when = Some(Restriction::Unrestricted));

    let response = host
        .scoped(super::deploy_key(&deploy_request(host.port)))
        .await;

    assert!(!response.success);
    assert!(
        response
            .error
            .is_some_and(|e| e.contains("verification failed")),
    );
}

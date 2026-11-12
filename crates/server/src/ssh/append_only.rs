// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Setting append-only access up on a repository host, checking it, and taking
//! it away again, by editing the login's `~/.ssh/authorized_keys` over SFTP.
//!
//! The order of the steps keeps the server from locking itself out: the
//! maintenance key is added and proven first, and only then is the key the
//! agents use restricted, through the maintenance key. SFTP cannot replace a
//! file atomically, so the file as Assimilate first found it is copied to
//! [`BACKUP_COPY`] before anything changes.

use std::{future::Future, time::Duration};

use russh_sftp::client::SftpSession;
use ssh_key::PublicKey;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use super::{
    KeyRole, SshClientHandler, SshError,
    authorized_keys::{self, AuthorizedKeysError, Restriction},
    connect_as, connect_with_password_pinned, keys, open_sftp,
};

const AUTHORIZED_KEYS: &str = ".ssh/authorized_keys";

/// Where the file is kept as Assimilate first found it.
pub const BACKUP_COPY: &str = ".ssh/authorized_keys.assimilate-backup";

/// How long to wait for a host before giving up on it.
const HOST_TIMEOUT: Duration = Duration::from_secs(30);

/// Why append-only could not be set up, checked or removed.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// Connecting, authenticating or an SFTP operation failed.
    #[error(transparent)]
    Ssh(#[from] SshError),
    /// A line for the host could not be built.
    #[error(transparent)]
    Keys(#[from] AuthorizedKeysError),
    /// The host did not answer in time.
    #[error("the host did not answer within {} seconds", HOST_TIMEOUT.as_secs())]
    Timeout,
    /// The maintenance key was added but the host does not accept it. The
    /// file was put back as it was.
    #[error(
        "the host did not accept the maintenance key after it was added; authorized_keys was put \
         back as it was"
    )]
    MaintenanceKeyRejected,
    /// What was written is not what is on the host when read back.
    #[error("authorized_keys on the host does not hold what was written")]
    NotWritten,
    /// The agents' key stopped authenticating after the change, so it was
    /// undone.
    #[error("the key the agents use stopped working after the change, so it was undone")]
    BackupKeyRejected,
    /// The server's own key is not on the host and no password was given.
    #[error(
        "the server's key is not accepted by the host, or gets no SFTP session there; the SSH \
         password is needed once"
    )]
    NeedsPassword,
}

/// The SSH login a setting covers.
#[derive(Debug, Clone, Copy)]
pub struct Login<'a> {
    /// Hostname the repositories use.
    pub host: &'a str,
    /// SSH user the repositories log in as.
    pub user: &'a str,
    /// SSH port.
    pub port: u16,
    /// The pinned host key, if the host has one.
    pub host_key: Option<&'a str>,
}

/// How to get in when the maintenance key is not on the host yet.
#[derive(Clone, Copy)]
pub enum Access<'a> {
    /// The server's own key, which most hosts already accept.
    ServerKey,
    /// The login's SSH password, used once and not stored.
    Password(&'a str),
}

impl std::fmt::Debug for Access<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServerKey => f.write_str("ServerKey"),
            // The password is a secret: never shown, even in a debug print.
            Self::Password(_) => f.write_str("Password([REDACTED])"),
        }
    }
}

/// What a connection check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// Whether the key the agents use is accepted.
    pub backup_key_ok: bool,
    /// Whether the maintenance key is accepted.
    pub maintenance_key_ok: bool,
    /// Whether the host's `authorized_keys` carries the restriction for
    /// exactly the repositories asked about.
    pub restriction_in_place: bool,
    /// What went wrong, if something did.
    pub error: Option<String>,
}

async fn timed<T, E>(fut: impl Future<Output = Result<T, E>>) -> Result<T, SetupError>
where
    SetupError: From<E>,
{
    tokio::time::timeout(HOST_TIMEOUT, fut)
        .await
        .map_err(|_| SetupError::Timeout)?
        .map_err(SetupError::from)
}

async fn connect_role(
    role: KeyRole,
    login: Login<'_>,
) -> Result<russh::client::Handle<SshClientHandler>, SetupError> {
    timed(connect_as(
        role,
        login.host,
        login.user,
        login.port,
        login.host_key.map(str::to_owned),
    ))
    .await
}

async fn read_authorized_keys(sftp: &SftpSession) -> Result<String, SetupError> {
    match sftp.try_exists(AUTHORIZED_KEYS).await {
        Ok(true) => {}
        Ok(false) => return Ok(String::new()),
        Err(e) => {
            return Err(SshError::Sftp(format!("failed to look for authorized_keys: {e}")).into());
        }
    }
    let bytes = sftp
        .read(AUTHORIZED_KEYS)
        .await
        .map_err(|e| SshError::Sftp(format!("failed to read authorized_keys: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Replaces the file at `path` with `bytes`, creating it if need be.
///
/// `SftpSession::write` opens a file for writing only: it neither creates a
/// missing file nor truncates an existing one, so a shorter text would leave
/// the end of the old one behind. `create` does both.
async fn put(sftp: &SftpSession, path: &str, bytes: &[u8]) -> Result<(), SshError> {
    let mut file = sftp
        .create(path)
        .await
        .map_err(|e| SshError::Sftp(format!("failed to open {path}: {e}")))?;
    file.write_all(bytes)
        .await
        .map_err(|e| SshError::Sftp(format!("failed to write {path}: {e}")))?;
    file.shutdown()
        .await
        .map_err(|e| SshError::Sftp(format!("failed to close {path}: {e}")))
}

async fn write_authorized_keys(sftp: &SftpSession, text: &str) -> Result<(), SetupError> {
    if let Err(e) = sftp.create_dir(".ssh").await {
        tracing::debug!(error = %e, "sftp create_dir .ssh failed (may already exist)");
    }
    put(sftp, AUTHORIZED_KEYS, text.as_bytes()).await?;
    let written = read_authorized_keys(sftp).await?;
    if written == text {
        Ok(())
    } else {
        Err(SetupError::NotWritten)
    }
}

/// Writes `text` back after a write failed, which can leave the file empty:
/// SFTP truncates it before writing. A failure here is logged, since the
/// first state of the file is in [`BACKUP_COPY`].
async fn put_back(sftp: &SftpSession, text: &str, why: &str) {
    if let Err(e) = write_authorized_keys(sftp, text).await {
        warn!(error = %e, "could not put authorized_keys back after {why}");
    }
}

/// Keeps the first state of the file Assimilate saw. A later change never
/// overwrites it, so it stays the way back to what the admin had.
async fn keep_first_copy(sftp: &SftpSession, original: &str) {
    match sftp.try_exists(BACKUP_COPY).await {
        Ok(false) => {
            if let Err(e) = put(sftp, BACKUP_COPY, original.as_bytes()).await {
                warn!(error = %e, "could not keep a copy of authorized_keys");
            }
        }
        Ok(true) => {}
        Err(e) => warn!(error = %e, "could not look for the authorized_keys copy"),
    }
}

/// A refused key means the password is needed; any other failure is kept.
fn needs_password_if_refused(error: SetupError) -> SetupError {
    match error {
        SetupError::Ssh(SshError::Auth(_)) => SetupError::NeedsPassword,
        other => other,
    }
}

/// Why a key could not be used after the file was written: `rejected` when the
/// host refused it, the failure itself when the host could not be reached to
/// say. The file is put back either way, since access cannot be confirmed.
fn rejected_if_refused(error: SetupError, rejected: SetupError) -> SetupError {
    match error {
        SetupError::Ssh(SshError::Auth(_)) => rejected,
        other => other,
    }
}

/// The session and SFTP channel for editing the file: the maintenance key's
/// when the host already accepts it, otherwise `access`.
async fn editor(
    login: Login<'_>,
    access: Access<'_>,
) -> Result<(russh::client::Handle<SshClientHandler>, SftpSession, bool), SetupError> {
    match connect_role(KeyRole::Maintenance, login).await {
        Ok(session) => {
            let sftp = timed(open_sftp(&session)).await?;
            return Ok((session, sftp, true));
        }
        // Only a refused key means it is not set up yet; a host that does not
        // answer is reported as that, not as a missing password.
        Err(SetupError::Ssh(SshError::Auth(_))) => {}
        Err(other) => return Err(other),
    }
    let session = match access {
        // Only a refused key means the password is needed; a host that does
        // not answer is a different problem and is reported as such.
        Access::ServerKey => connect_role(KeyRole::Backup, login)
            .await
            .map_err(needs_password_if_refused)?,
        Access::Password(password) => {
            timed(connect_with_password_pinned(
                login.host,
                login.user,
                login.port,
                password,
                login.host_key.map(str::to_owned),
            ))
            .await?
        }
    };
    let sftp = timed(open_sftp(&session))
        .await
        .map_err(|e| match (access, e) {
            // A restricted server key authenticates but gets the forced command
            // instead of the SFTP subsystem.
            (Access::ServerKey, SetupError::Ssh(SshError::Sftp(_))) => SetupError::NeedsPassword,
            (_, e) => e,
        })?;
    Ok((session, sftp, false))
}

/// Makes the login append-only for the repositories at `repo_paths`.
///
/// Safe to run again: it brings the host's file in line with the current set
/// of repositories, which is how a newly added repository is let in.
///
/// # Errors
///
/// Returns a [`SetupError`] naming the step that failed. A key the host turns
/// out to refuse is taken back out, so the host is not left unusable. So is a
/// write the host does not keep ([`SetupError::NotWritten`]) after the
/// maintenance key is proven. If putting the file back fails as well,
/// [`BACKUP_COPY`] holds it as it first was.
pub async fn enable(
    login: Login<'_>,
    repo_paths: &[String],
    access: Access<'_>,
) -> Result<(), SetupError> {
    // The steps hold russh's connection state across many awaits, which makes
    // the future large; boxing it keeps every caller's future small.
    Box::pin(enable_steps(login, repo_paths, access)).await
}

async fn enable_steps(
    login: Login<'_>,
    repo_paths: &[String],
    access: Access<'_>,
) -> Result<(), SetupError> {
    let maintenance = keys::ensure_maintenance_key().await?.public_key().clone();
    let backup = keys::read_backup_public_key().await?;
    // Refuse a path that cannot be put in a forced command before touching
    // the host.
    let restricted_line = authorized_keys::restricted_line(&backup, repo_paths)?;

    let (entry, entry_sftp, entered_with_maintenance_key) = editor(login, access).await?;

    let original = read_authorized_keys(&entry_sftp).await?;
    keep_first_copy(&entry_sftp, &original).await;

    if !entered_with_maintenance_key {
        if let Err(e) = write_authorized_keys(
            &entry_sftp,
            &authorized_keys::with_maintenance_key(&original, &maintenance),
        )
        .await
        {
            put_back(
                &entry_sftp,
                &original,
                "the maintenance key was not written",
            )
            .await;
            return Err(e);
        }
        if let Err(e) = connect_role(KeyRole::Maintenance, login).await {
            put_back(&entry_sftp, &original, "the maintenance key was rejected").await;
            return Err(rejected_if_refused(e, SetupError::MaintenanceKeyRejected));
        }
    }
    drop(entry_sftp);
    drop(entry);

    // From here on the maintenance key does the editing.
    let session = connect_role(KeyRole::Maintenance, login).await?;
    let sftp = timed(open_sftp(&session)).await?;
    let current = read_authorized_keys(&sftp).await?;
    let restricted = authorized_keys::with_restricted_line(
        &authorized_keys::with_maintenance_key(&current, &maintenance),
        &backup,
        &restricted_line,
    );
    if let Err(e) = write_authorized_keys(&sftp, &restricted).await {
        put_back(&sftp, &current, "the restriction was not written").await;
        return Err(e);
    }

    if let Err(e) = connect_role(KeyRole::Backup, login).await {
        put_back(&sftp, &current, "the backup key was rejected").await;
        return Err(rejected_if_refused(e, SetupError::BackupKeyRejected));
    }
    info!(
        host = login.host,
        user = login.user,
        "append-only set up on the host"
    );
    Ok(())
}

/// Takes append-only away: the agents' key is a plain line again and the
/// maintenance key is removed from the host.
///
/// # Errors
///
/// Returns a [`SetupError`] naming the step that failed. A key the host
/// refuses, or a write it does not keep, is undone by putting the file back as
/// it was; if that fails as well, [`BACKUP_COPY`] holds its first state.
pub async fn disable(login: Login<'_>, access: Access<'_>) -> Result<(), SetupError> {
    // The steps hold russh's connection state across many awaits, which makes
    // the future large; boxing it keeps every caller's future small.
    Box::pin(disable_steps(login, access)).await
}

async fn disable_steps(login: Login<'_>, access: Access<'_>) -> Result<(), SetupError> {
    let maintenance = keys::ensure_maintenance_key().await?.public_key().clone();
    let backup = keys::read_backup_public_key().await?;

    let (session, sftp, _) = editor(login, access).await?;
    let current = read_authorized_keys(&sftp).await?;
    keep_first_copy(&sftp, &current).await;
    if let Err(e) = write_authorized_keys(
        &sftp,
        &authorized_keys::without_append_only(&current, &backup, &maintenance),
    )
    .await
    {
        put_back(&sftp, &current, "the plain key was not written").await;
        return Err(e);
    }

    // The session that did the editing is still open, so a plain key the
    // host turns out to refuse can be put back as it was: leaving it would
    // drop the restriction while the database still says it is in place.
    if let Err(e) = connect_role(KeyRole::Backup, login).await {
        put_back(&sftp, &current, "the plain key was rejected").await;
        return Err(rejected_if_refused(e, SetupError::BackupKeyRejected));
    }
    drop(sftp);
    drop(session);
    info!(
        host = login.host,
        user = login.user,
        "append-only removed from the host"
    );
    Ok(())
}

/// Tries both keys against the host and reads its `authorized_keys`.
///
/// This cannot prove that borg refuses deletions - borg offers no way to ask -
/// but the file shows whether the restriction is installed.
pub async fn check(login: Login<'_>, repo_paths: &[String]) -> CheckResult {
    // Boxed for the same reason as `enable`: the connection state makes it large.
    Box::pin(check_steps(login, repo_paths)).await
}

async fn check_steps(login: Login<'_>, repo_paths: &[String]) -> CheckResult {
    let backup_key_ok = connect_role(KeyRole::Backup, login).await.is_ok();

    let (maintenance_key_ok, restriction_in_place, error) = match inspect(login, repo_paths).await {
        Ok(in_place) => (true, in_place, None),
        Err(e) => (false, false, Some(e.to_string())),
    };
    let error = error.or_else(|| {
        (!backup_key_ok).then(|| "the key the agents use is not accepted by the host".to_owned())
    });

    CheckResult {
        backup_key_ok,
        maintenance_key_ok,
        restriction_in_place,
        error,
    }
}

async fn inspect(login: Login<'_>, repo_paths: &[String]) -> Result<bool, SetupError> {
    let backup: PublicKey = keys::read_backup_public_key().await?;
    let session = connect_role(KeyRole::Maintenance, login).await?;
    let sftp = timed(open_sftp(&session)).await?;
    let text = read_authorized_keys(&sftp).await?;
    Ok(restriction_covers(&text, &backup, repo_paths))
}

/// Whether the file restricts the agents' key to append-only access to
/// exactly `repo_paths`.
#[must_use]
pub fn restriction_covers(text: &str, backup: &PublicKey, repo_paths: &[String]) -> bool {
    authorized_keys::restriction_of(text, backup) == Restriction::AppendOnly
        && authorized_keys::restricted_line(backup, repo_paths)
            .is_ok_and(|line| text.lines().any(|l| l.trim() == line))
}

#[cfg(test)]
mod tests {
    use ssh_key::{Algorithm, PrivateKey, rand_core::OsRng};

    use super::*;

    #[test]
    fn a_host_that_could_not_be_reached_is_not_reported_as_refusing_a_key() {
        assert!(matches!(
            rejected_if_refused(
                SetupError::Ssh(SshError::Auth("no".to_owned())),
                SetupError::BackupKeyRejected
            ),
            SetupError::BackupKeyRejected
        ));
        assert!(matches!(
            rejected_if_refused(SetupError::Timeout, SetupError::BackupKeyRejected),
            SetupError::Timeout
        ));
    }

    #[test]
    fn only_a_refused_key_turns_into_a_request_for_the_password() {
        assert!(matches!(
            needs_password_if_refused(SetupError::Ssh(SshError::Auth("no".to_owned()))),
            SetupError::NeedsPassword
        ));
        assert!(matches!(
            needs_password_if_refused(SetupError::Timeout),
            SetupError::Timeout
        ));
        assert!(matches!(
            needs_password_if_refused(SetupError::Ssh(SshError::Connection("down".to_owned()))),
            SetupError::Ssh(SshError::Connection(_))
        ));
    }

    fn key() -> PublicKey {
        let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone();
        key.set_comment("assimilate-server");
        key
    }

    #[test]
    fn the_password_is_not_in_a_debug_print() {
        let printed = format!("{:?}", Access::Password("hunter2"));
        assert!(!printed.contains("hunter2"), "{printed}");
        assert_eq!(format!("{:?}", Access::ServerKey), "ServerKey");
    }

    #[test]
    fn the_restriction_covers_the_repositories_it_was_written_for() {
        let backup = key();
        let paths = vec!["/srv/a".to_owned(), "/srv/b".to_owned()];
        let text = authorized_keys::with_restricted_backup_key("", &backup, &paths).unwrap();
        assert!(restriction_covers(&text, &backup, &paths));
    }

    #[test]
    fn a_restriction_for_other_repositories_does_not_count() {
        let backup = key();
        let text = authorized_keys::with_restricted_backup_key("", &backup, &["/srv/a".to_owned()])
            .unwrap();
        assert!(!restriction_covers(
            &text,
            &backup,
            &["/srv/a".to_owned(), "/srv/b".to_owned()]
        ));
    }

    #[test]
    fn a_plain_key_is_not_a_restriction() {
        let backup = key();
        let text = format!("{}\n", backup.to_openssh().unwrap());
        assert!(!restriction_covers(&text, &backup, &["/srv/a".to_owned()]));
    }

    #[test]
    fn an_unusable_path_is_never_covered() {
        let backup = key();
        let text = authorized_keys::with_restricted_backup_key("", &backup, &["/srv/a".to_owned()])
            .unwrap();
        assert!(!restriction_covers(
            &text,
            &backup,
            &["/srv/\"a".to_owned()]
        ));
    }
}

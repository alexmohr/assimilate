// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Restoring a database dump from an archive into the database server its
//! source runs on now.
//!
//! `borg extract --stdout` streams the dump straight into the restore client's
//! stdin. When borg fails, the client must not take the end of what borg
//! wrote for the end of the dump: the stream ends in an error instead, and
//! the client is stopped before its stdin is closed. A restore has no time
//! limit of its own, since a large database takes as long as it takes; borg
//! waits at most ten minutes for the repository's lock.

use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
};

use shared::{
    borg::GracefulChild,
    database::{
        DatabaseRestore, DatabaseRestoreTarget, DatabaseRuntime, DatabaseRuntimes,
        DatabaseSourceId, MAX_DATABASE_NAME_BYTES, is_valid_new_database_name,
    },
    protocol::AgentToServer,
    types::RepoId,
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    process::ChildStdout,
    sync::{mpsc, oneshot},
};
use tracing::{error, info, warn};

use super::{
    Executor, FreeTaskContext, RepoOperationKey, RestoreDatabaseRequest, send_operation_failed,
    send_outbound, transport::setup_ssh_forward,
};
use crate::{
    backup::BackupTarget,
    borg::{Borg, StreamingBorg},
    database::{RuntimeClients, SourceCandidate, find_all, read_tail, restore_dump},
};

/// What a queued database restore runs with.
pub(super) struct DatabaseRestoreTaskParams {
    pub(super) repo_id: RepoId,
    pub(super) target: BackupTarget,
    pub(super) archive_name: String,
    pub(super) restore: DatabaseRestore,
    /// Where the host searches for databases, from its current settings.
    pub(super) runtimes: DatabaseRuntimes,
    pub(super) request_id: String,
}

impl Executor {
    /// Queues a database restore behind the repository's other operations.
    /// A restore that is invalid on its face is refused at once.
    pub(super) async fn restore_database(
        &self,
        request: RestoreDatabaseRequest,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let RestoreDatabaseRequest {
            request_id,
            repo_id,
            archive_name,
            restore,
        } = request;
        if let Some(reason) = rejection(&restore) {
            warn!(repo_id = ?repo_id, "refusing a database restore: {reason}");
            send_operation_failed(outbound_tx, request_id, reason).await;
            return;
        }
        let Some((target, hostname)) = self
            .request_target(repo_id, &request_id, "database restore", outbound_tx)
            .await
        else {
            return;
        };
        let config = Arc::clone(&self.current_config);
        let repo_key = RepoOperationKey::from_backup_target(&target);
        let ctx = self.owned_task_context(hostname, outbound_tx);

        info!(repo_id = ?repo_id, repo = %repo_key.repo_url(), "queued database restore");

        self.spawn_queued(repo_id, repo_key, async move {
            // Read when the restore starts rather than when it was queued, so
            // runtime settings changed while it waited for the repository
            // are the ones it uses.
            let runtimes = config
                .lock()
                .await
                .as_ref()
                .map_or_else(DatabaseRuntimes::default, |config| {
                    config.database_backup.runtimes.clone()
                });
            run_database_restore_task(
                DatabaseRestoreTaskParams {
                    repo_id,
                    target,
                    archive_name,
                    restore,
                    runtimes,
                    request_id,
                },
                ctx.borrowed(),
                &ctx.borg,
            )
            .await;
        })
        .await;
    }
}

/// Why `restore` is refused before anything runs, if it is. The server
/// validates the name too; this is the agent's own check.
fn rejection(restore: &DatabaseRestore) -> Option<String> {
    match &restore.target {
        DatabaseRestoreTarget::NewDatabase { name } if !is_valid_new_database_name(name) => {
            Some(format!(
                "cannot restore into a database named {name:?}: a new database's name is 1 to \
                 {MAX_DATABASE_NAME_BYTES} letters, digits, '_' or '-', and starts with neither \
                 '-' nor 'pg_'"
            ))
        }
        DatabaseRestoreTarget::NewDatabase { .. } | DatabaseRestoreTarget::Original => None,
    }
}

/// Restores the dump, reporting `RestoreCompleted`, or `OperationFailed`
/// when the restore cannot start: the source is not found on this host, or
/// borg cannot be started.
pub(super) async fn run_database_restore_task(
    params: DatabaseRestoreTaskParams,
    ctx: FreeTaskContext<'_>,
    borg: &Borg,
) {
    let DatabaseRestoreTaskParams {
        repo_id,
        mut target,
        archive_name,
        restore,
        runtimes,
        request_id,
    } = params;
    let FreeTaskContext {
        hostname,
        server_url,
        token,
        outbound_tx,
    } = ctx;

    let (clients, source) = match locate(&runtimes, &restore.source).await {
        Ok(found) => found,
        Err(reason) => {
            warn!(repo_id = ?repo_id, "cannot restore a database: {reason}");
            send_operation_failed(outbound_tx, request_id, reason).await;
            return;
        }
    };

    let _ssh_forward = setup_ssh_forward(&mut target, hostname, server_url, token).await;
    let env_vars = crate::backup::borg_env(&target);
    let args = extract_dump_args(&archive_name, &restore.archive_path);
    info!(
        repo_id = ?repo_id,
        archive = %archive_name,
        source = %restore.source,
        "running borg extract of a database dump"
    );
    let StreamingBorg {
        child,
        stdout,
        stderr,
    } = match borg.spawn(&args, &env_vars) {
        Ok(spawned) => spawned,
        Err(e) => {
            send_operation_failed(
                outbound_tx,
                request_id,
                format!("failed to execute borg: {e}"),
            )
            .await;
            return;
        }
    };

    let (exit_tx, exit_rx) = oneshot::channel();
    let mut dump = DumpStream::new(stdout, child, exit_tx);
    let restoring = async {
        let restored = restore_dump(&clients, &source, &restore, &mut dump).await;
        // Stops borg when the restore ended before reading all of it.
        drop(dump);
        restored
    };
    let (restored, borg_stderr) = tokio::join!(restoring, read_tail(stderr));
    // No exit code: borg was stopped before it finished, because the restore
    // had already failed.
    let exit_code = exit_rx.await.ok();
    let error = match (exit_code, restored) {
        (Some(code), _) if !extract_succeeded(code) => Some(format!(
            "borg extract failed (exit {code}): {}",
            borg_stderr.trim()
        )),
        (_, Err(e)) => Some(e.to_string()),
        (_, Ok(())) => None,
    };
    // Borg's warnings (exit 1) do not fail the restore, but are logged the
    // way a file restore logs them.
    if let Some(code @ 1) = exit_code
        && error.is_none()
    {
        let warning = crate::backup::warning_status_log("extract", code, &borg_stderr);
        warn!(repo_id = ?repo_id, "{warning}");
    }
    if let Some(message) = &error {
        error!(repo_id = ?repo_id, "database restore failed: {message}");
    } else {
        let database = restore.target_database();
        info!(repo_id = ?repo_id, database, "database restored");
    }
    let success = error.is_none();
    let msg = AgentToServer::RestoreCompleted {
        request_id,
        success,
        files_restored: u64::from(success),
        error_message: error,
    };
    send_outbound(outbound_tx, msg).await;
}

/// borg's arguments for streaming the dump at `archive_path` to stdout.
pub(super) fn extract_dump_args(archive_name: &str, archive_path: &str) -> Vec<String> {
    let archive_spec = format!("::{archive_name}");
    Borg::args_with_positional(
        &["extract", "--stdout", "--lock-wait", "600", &archive_spec],
        &[archive_path],
    )
}

/// Whether borg's exit code means the extract worked; 1 is a warning.
const fn extract_succeeded(code: i32) -> bool {
    matches!(code, 0 | 1)
}

/// Where `source` runs now, and a client for its runtime. Only the source's
/// own runtime is searched, so one that cannot be reached elsewhere on the
/// host does not stand in the way.
async fn locate(
    runtimes: &DatabaseRuntimes,
    source: &DatabaseSourceId,
) -> Result<(RuntimeClients, SourceCandidate), String> {
    let only = only_runtime(runtimes, source.runtime).ok_or_else(|| {
        format!(
            "cannot restore into {source}: this host does not search {} for databases",
            source.runtime
        )
    })?;
    let found = find_all(&only).await;
    if let Some(failure) = found.failure() {
        return Err(format!("{failure}; cannot restore into {source}"));
    }
    let candidate = found
        .sources
        .into_iter()
        .find(|candidate| candidate.id == *source)
        .ok_or_else(|| format!("{source} is not running on this host"))?;
    Ok((found.clients, candidate))
}

/// `runtimes` with only `runtime` turned on, or `None` when it is off.
fn only_runtime(runtimes: &DatabaseRuntimes, runtime: DatabaseRuntime) -> Option<DatabaseRuntimes> {
    runtimes.searches(runtime).then(|| DatabaseRuntimes {
        kubernetes: runtime == DatabaseRuntime::Kubernetes,
        docker: runtime == DatabaseRuntime::Docker,
        host: runtime == DatabaseRuntime::Host,
        ..runtimes.clone()
    })
}

/// How borg ended, once its stdout has.
type BorgEnd = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

/// What `borg extract --stdout` writes, ending in an error rather than an end
/// of file when borg fails, so the restore never takes a cut-off dump for a
/// whole one. Dropping it stops borg.
struct DumpStream {
    stdout: ChildStdout,
    /// Waits for borg once its stdout ended; `None` once it has.
    end: Option<BorgEnd>,
}

impl DumpStream {
    /// Reads `stdout` of `child`, and sends borg's exit code to `exit` once
    /// it ended; `-1` when it was killed or could not be waited for.
    fn new(stdout: ChildStdout, mut child: GracefulChild, exit: oneshot::Sender<i32>) -> Self {
        let end = Box::pin(async move {
            let code = child
                .wait()
                .await
                .map_or(-1, |status| status.code().unwrap_or(-1));
            // The restore task may be gone already; then nobody asks.
            let _ = exit.send(code);
            if extract_succeeded(code) {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "borg extract exited with code {code}"
                )))
            }
        });
        Self {
            stdout,
            end: Some(end),
        }
    }
}

impl AsyncRead for DumpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let filled = buf.filled().len();
        ready!(Pin::new(&mut self.stdout).poll_read(cx, buf))?;
        if buf.filled().len() > filled {
            return Poll::Ready(Ok(()));
        }
        let Some(end) = self.end.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let ended = ready!(end.as_mut().poll(cx));
        self.end = None;
        Poll::Ready(ended)
    }
}

#[cfg(test)]
mod tests;

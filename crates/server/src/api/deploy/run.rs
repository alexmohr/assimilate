// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Running an agent deploy on the server, after the request that started it
//! has answered: the SSH steps, then waiting for the restarted agent to
//! connect back, with every step reported to the deploy's job.

use std::{path::PathBuf, time::Duration};

use shared::responses::DeployStep;
use tracing::{info, warn};

use crate::{
    AgentRegistry, db,
    deploy_jobs::{DeployJobs, JobId, Outcome},
    ssh,
};

/// How long a deploy may take in all. A host that keeps the connection open and
/// never answers would otherwise hold the deploy for ever; this ends it, well
/// inside the time after which the server treats a deploy as abandoned.
pub(super) const DEPLOY_DEADLINE: Duration = Duration::from_mins(20);
/// How long a restarted agent has to connect back before the deploy fails.
pub(super) const RECONNECT_TIMEOUT: Duration = Duration::from_secs(90);
/// How often the server looks for the restarted agent.
pub(super) const RECONNECT_POLL: Duration = Duration::from_millis(500);
/// How long after the restart an agent that names no process instance has to
/// have been connected before it counts as the restarted one: it cannot be told
/// from the old process by its identity, only by outlasting its shutdown.
const RECONNECT_GRACE: Duration = Duration::from_secs(3);

/// Everything a deploy needs once it runs on its own, away from the request.
pub(super) struct DeployRun {
    pub(super) pool: sqlx::PgPool,
    pub(super) registry: AgentRegistry,
    pub(super) jobs: DeployJobs,
    /// Which deploy of the agent this is, so its reports reach only it.
    pub(super) job: JobId,
    pub(super) agent_id: i64,
    pub(super) hostname: String,
    pub(super) ssh_host: String,
    pub(super) ssh_user: String,
    pub(super) ssh_port: u16,
    pub(super) ssh_password: Option<String>,
    pub(super) binary_dir: PathBuf,
    pub(super) install_path: String,
    pub(super) server_url: String,
    pub(super) token: String,
    pub(super) server_key: String,
    pub(super) systemd_service_content: Option<String>,
    pub(super) available_version: Option<String>,
    pub(super) server_commit_count: Option<i32>,
    /// How long the restarted agent has to connect back.
    pub(super) reconnect_timeout: Duration,
    /// How often the server looks for it.
    pub(super) reconnect_poll: Duration,
    /// How long the whole deploy may take.
    pub(super) deadline: Duration,
}

/// Why a deploy stopped.
#[derive(Debug, thiserror::Error)]
enum DeployError {
    #[error("{0}")]
    Ssh(#[from] ssh::SshError),
    #[error(
        "the agent was installed and restarted but did not connect back to the server within \
         {} seconds; check the agent's logs on the host",
        .0.as_secs()
    )]
    NotReconnected(Duration),
    #[error("the deploy did not finish within {} seconds and was stopped", .0.as_secs())]
    TimedOut(Duration),
}

/// Runs a deploy to its end and records how it went. Never fails: whatever
/// goes wrong is the deploy's result.
pub(super) async fn run_deploy(run: DeployRun) {
    let before = presence(&run.registry, run.agent_id).await;
    let result = match tokio::time::timeout(run.deadline, execute_deploy(&run, &before)).await {
        Ok(result) => result,
        Err(_) => Err(DeployError::TimedOut(run.deadline)),
    };
    if let Err(e) = &result {
        warn!(hostname = %run.hostname, error = %e, "agent deploy failed");
    }
    run.jobs.finish(
        run.agent_id,
        run.job,
        Outcome {
            skipped: false,
            token: Some(run.token),
            available_version: run.available_version,
            error: result.err().map(|e| e.to_string()),
        },
    );
}

async fn execute_deploy(run: &DeployRun, before: &Presence) -> Result<(), DeployError> {
    ssh::deploy_agent(
        &ssh::DeployAgentParams {
            host: &run.ssh_host,
            user: &run.ssh_user,
            port: run.ssh_port,
            binary_dir: &run.binary_dir,
            remote_path: &run.install_path,
            server_url: &run.server_url,
            token: &run.token,
            server_key: &run.server_key,
            password: run.ssh_password.as_deref(),
            systemd_service_content: run.systemd_service_content.as_deref(),
        },
        &|step| run.jobs.begin_step(run.agent_id, run.job, step),
    )
    .await?;

    // The host is deployed, so a failure to record it must not fail the deploy.
    if let Some(version) = &run.available_version
        && let Err(e) = db::update_last_seen_and_version(
            &run.pool,
            run.agent_id,
            version,
            None,
            None,
            run.server_commit_count,
        )
        .await
    {
        warn!(hostname = %run.hostname, error = %e, "failed to record the deployed agent version");
    }
    if let Err(e) = db::update_last_ssh_user(&run.pool, run.agent_id, &run.ssh_user).await {
        warn!(hostname = %run.hostname, error = %e, "failed to record the deploy's SSH user");
    }
    info!(hostname = %run.hostname, ssh_host = %run.ssh_host, "agent deployed, waiting for it to reconnect");

    run.jobs
        .begin_step(run.agent_id, run.job, DeployStep::Reconnect);
    if wait_for_reconnect(
        &run.registry,
        run.agent_id,
        before,
        run.reconnect_timeout,
        run.reconnect_poll,
    )
    .await
    {
        info!(hostname = %run.hostname, "agent reconnected after deploy");
        Ok(())
    } else {
        Err(DeployError::NotReconnected(run.reconnect_timeout))
    }
}

/// Whether an agent is connected to the server, and as which process.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Presence {
    /// Not connected.
    Absent,
    /// Connected as the process named by the instance id, or as one that names
    /// none.
    Present(Option<String>),
}

async fn presence(registry: &AgentRegistry, agent_id: i64) -> Presence {
    match registry.instance_id(agent_id).await {
        None => Presence::Absent,
        Some(instance) => Presence::Present(instance),
    }
}

/// Whether the agent connected now is the one the deploy restarted, rather than
/// the process that was running before it: `before` is the agent's presence
/// before the restart, `now` its presence now, `since_restart` how long the
/// restart has been under way.
fn is_restarted_agent(before: &Presence, now: &Presence, since_restart: Duration) -> bool {
    match (before, now) {
        (_, Presence::Absent) => false,
        // It was not connected, so any connection is the new one.
        (Presence::Absent, Presence::Present(_)) => true,
        (Presence::Present(Some(previous)), Presence::Present(current)) => {
            current.as_ref() != Some(previous)
        }
        // The old process named no instance, so only its shutdown tells the two
        // apart: a connection that is still there after a grace period is not it.
        (Presence::Present(None), Presence::Present(_)) => since_restart >= RECONNECT_GRACE,
    }
}

/// Waits up to `timeout` for the deployed agent to connect back. Returns
/// whether it did.
async fn wait_for_reconnect(
    registry: &AgentRegistry,
    agent_id: i64,
    before: &Presence,
    timeout: Duration,
    poll: Duration,
) -> bool {
    let started = tokio::time::Instant::now();
    loop {
        let now = presence(registry, agent_id).await;
        if is_restarted_agent(before, &now, started.elapsed()) {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(id: &str) -> Presence {
        Presence::Present(Some(id.to_owned()))
    }

    #[test]
    fn an_agent_that_was_not_connected_counts_as_restarted_once_it_connects() {
        assert!(is_restarted_agent(
            &Presence::Absent,
            &Presence::Present(None),
            Duration::ZERO
        ));
        assert!(is_restarted_agent(
            &Presence::Absent,
            &present("new"),
            Duration::ZERO
        ));
        assert!(!is_restarted_agent(
            &Presence::Absent,
            &Presence::Absent,
            Duration::from_mins(1)
        ));
    }

    #[test]
    fn a_connection_from_the_old_process_is_not_the_restarted_agent() {
        let before = present("old");
        assert!(!is_restarted_agent(
            &before,
            &present("old"),
            Duration::from_mins(1)
        ));
        assert!(!is_restarted_agent(
            &before,
            &Presence::Absent,
            Duration::from_mins(1)
        ));
        assert!(is_restarted_agent(&before, &present("new"), Duration::ZERO));
        assert!(
            is_restarted_agent(&before, &Presence::Present(None), Duration::ZERO),
            "an agent that now names no instance is a different process"
        );
    }

    #[test]
    fn an_agent_without_an_instance_must_outlast_the_grace_period() {
        let before = Presence::Present(None);
        assert!(!is_restarted_agent(
            &before,
            &Presence::Present(None),
            RECONNECT_GRACE.saturating_sub(Duration::from_millis(1))
        ));
        assert!(is_restarted_agent(
            &before,
            &Presence::Present(None),
            RECONNECT_GRACE
        ));
    }

    async fn connect(registry: &AgentRegistry, agent_id: i64, instance_id: &str) {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        registry
            .register(agent_id, sender, false, None, Some(instance_id.to_owned()))
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_returns_once_the_restarted_agent_connects() {
        let registry = AgentRegistry::new();
        connect(&registry, 1, "old").await;
        let before = presence(&registry, 1).await;

        let reconnecting = registry.clone();
        let wait = tokio::spawn(async move {
            wait_for_reconnect(
                &reconnecting,
                1,
                &before,
                Duration::from_secs(30),
                Duration::from_millis(500),
            )
            .await
        });
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!wait.is_finished(), "the old process is still connected");

        connect(&registry, 1, "new").await;
        assert!(wait.await.expect("the wait ends"));
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_gives_up_when_the_agent_never_comes_back() {
        let registry = AgentRegistry::new();
        connect(&registry, 1, "old").await;
        let before = presence(&registry, 1).await;
        registry.unregister(1).await;

        let reconnected = wait_for_reconnect(
            &registry,
            1,
            &before,
            Duration::from_secs(30),
            Duration::from_millis(500),
        )
        .await;

        assert!(!reconnected);
    }

    #[test]
    fn a_deploy_that_runs_out_of_time_says_how_long_it_had() {
        let message = DeployError::TimedOut(DEPLOY_DEADLINE).to_string();

        assert!(message.contains("1200 seconds"), "{message}");
        assert!(message.contains("did not finish"), "{message}");
    }

    #[test]
    fn the_deadline_ends_a_deploy_before_the_server_would_call_it_abandoned() {
        assert!(DEPLOY_DEADLINE < crate::deploy_jobs::RUNNING_JOB_TTL);
    }

    #[test]
    fn a_deploy_that_never_reconnects_says_how_long_it_waited() {
        let message = DeployError::NotReconnected(RECONNECT_TIMEOUT).to_string();

        assert!(message.contains("90 seconds"), "{message}");
        assert!(message.contains("did not connect back"), "{message}");
    }
}

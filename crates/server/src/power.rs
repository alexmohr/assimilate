// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Waking, starting, stopping, and shutting down power-managed hosts around
//! a backup.
//!
//! Two independent hosts can be power-managed for a given run: the source
//! (the agent's host, reached over its WebSocket connection) and the
//! repository (an SSH-only destination, no agent involved). For each, the
//! same rule applies: check whether it's already reachable, and only if not,
//! wake it and wait; afterward, undo only what this run itself turned on.
//!
//! [`ensure_agent_online`] and [`ensure_repo_online`] are deliberately
//! infallible (best-effort): a wake or start that doesn't pan out is logged
//! and recorded as a run event, not surfaced as an error from this module.
//! The caller finds out the same way it always has -- by checking whether
//! the agent is connected / the repo is reachable once these return, which
//! is exactly the existing "still unreachable" failure path a backup target
//! already goes through today. This also keeps [`PowerSessionTracker`]
//! bookkeeping accurate: the outcome returned always reflects what actually
//! happened, even when a step along the way failed.

use std::{
    collections::HashMap,
    fmt,
    future::Future,
    net::IpAddr,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use shared::{
    protocol::ServerToUi,
    types::{RunEventTarget, RunEventType},
};
use tokio::{net::UdpSocket, sync::RwLock, time::sleep};
use tracing::warn;

use crate::{
    db::{AgentRow, RepoRow, run_events},
    ssh::{self, SystemctlAction},
    ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

/// Everything this module needs from [`crate::AppState`], borrowed
/// separately rather than taking the whole state -- the scheduler's own
/// per-target context ([`crate::scheduler`]) is built the same way, and
/// mirroring it here means this module doesn't need a full `AppState` to
/// exercise in tests.
#[derive(Clone, Copy)]
pub struct PowerCtx<'a> {
    /// Database connection pool.
    pub pool: &'a sqlx::PgPool,
    /// Registry of connected WebSocket agents.
    pub registry: &'a AgentRegistry,
    /// Broadcast channel for UI-facing WebSocket messages.
    pub ui_broadcast: &'a UiBroadcast,
    /// Reference-counts concurrent users of each power-managed host.
    pub power_sessions: &'a PowerSessionTracker,
}

/// UDP port Wake-on-LAN magic packets are conventionally sent to.
const WOL_PORT: u16 = 9;
/// Broadcast address used when a host doesn't specify its own.
const DEFAULT_BROADCAST_ADDR: &str = "255.255.255.255";
/// How often to poll for reachability while waiting for a host to come up.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long a single reachability probe (SSH connect attempt) is allowed to
/// take before it counts as "no response".
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a single SSH action that connects and then runs one remote
/// command (start/stop the agent service, shut a host down) is allowed to
/// take before it counts as failed. Longer than `PROBE_TIMEOUT` since it
/// covers actually running the command, not just connecting -- but still
/// bounded, so a host that accepts the TCP connection but never completes
/// the handshake (or a firewall silently dropping packets) can't stall the
/// caller past this module's documented "always returns" behavior.
const SSH_ACTION_TIMEOUT: Duration = Duration::from_secs(30);
/// Fallback SSH user when an agent has never recorded one (only reachable
/// via `start_agent_enabled`, which the API layer requires `last_ssh_user`
/// for -- this is a last-resort default, not the expected path).
const FALLBACK_SSH_USER: &str = "root";

/// A 6-byte MAC address, parsed from and displayed in colon-hex form
/// (`AA:BB:CC:DD:EE:FF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// Builds the standard Wake-on-LAN magic packet: six `0xFF` bytes
    /// followed by this MAC address repeated sixteen times.
    fn magic_packet(self) -> [u8; 102] {
        let mut packet = [0xFFu8; 102];
        for chunk in packet[6..].as_chunks_mut::<6>().0 {
            chunk.copy_from_slice(&self.0);
        }
        packet
    }
}

impl FromStr for MacAddress {
    type Err = MacAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Exactly two hex digits per octet, matching the DB's
        // `agents_wake_mac_format`/`repos_wake_mac_format` CHECK constraints
        // byte-for-byte -- `u8::from_str_radix` alone is looser (it accepts
        // single-digit octets and a leading `+`), which would let a value
        // pass this parse and still fail that constraint as an opaque 500.
        let parts: Vec<&str> = s.split(':').collect();
        let [p0, p1, p2, p3, p4, p5]: [&str; 6] =
            parts.try_into().map_err(|_| MacAddressParseError)?;
        let mut octets = [0u8; 6];
        for (octet, part) in octets.iter_mut().zip([p0, p1, p2, p3, p4, p5]) {
            if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(MacAddressParseError);
            }
            *octet = u8::from_str_radix(part, 16).map_err(|_| MacAddressParseError)?;
        }
        Ok(Self(octets))
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let octets: Vec<String> = self.0.iter().map(|octet| format!("{octet:02X}")).collect();
        write!(f, "{}", octets.join(":"))
    }
}

/// A MAC address string didn't parse as six colon-separated hex octets. The
/// DB layer also CHECK-constrains this format, so this is mainly hit at the
/// API validation boundary before a value ever reaches the database.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("not a valid MAC address (expected AA:BB:CC:DD:EE:FF)")]
pub struct MacAddressParseError;

/// Which power-managed host a [`PowerSessionTracker`] entry is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerHostKey {
    /// An agent's host, identified by the agent's database ID.
    Agent(i64),
    /// A repository's host, identified by the repository's database ID.
    Repo(i64),
}

#[derive(Default)]
struct PowerSession {
    /// How many currently-executing targets are relying on this host
    /// staying up.
    count: usize,
    /// Whether *any* participating target woke this host this session.
    woke: bool,
    /// Whether *any* participating target started the agent process this
    /// session (agent hosts only).
    started_agent: bool,
}

/// Reference-counts how many concurrently running targets rely on each
/// power-managed host being up, so a host woken for one schedule isn't torn
/// down out from under another schedule that is still using it. A target
/// always calls [`begin`](Self::begin) once it has finished (successfully or
/// not) trying to reach a host, and [`end`](Self::end) once after its run
/// completes; only the call that brings the session's count back to zero
/// gets back the accumulated `(woke, started_agent)` flags to act on.
#[derive(Clone, Default)]
pub struct PowerSessionTracker {
    sessions: Arc<RwLock<HashMap<PowerHostKey, PowerSession>>>,
}

impl PowerSessionTracker {
    /// Registers a target as relying on `key` staying up for the duration of
    /// its run, *before* attempting to reach/wake it. Reserving up front
    /// (rather than only once the wake attempt has resolved) is what makes
    /// the reference count correct while two targets are concurrently
    /// waking the same host: without it, a target whose wake resolves
    /// quickly could tear the host down via [`end`](Self::end) while a
    /// slower sibling is still mid-wait, because the tracker wouldn't know
    /// the sibling exists yet.
    pub async fn reserve(&self, key: PowerHostKey) {
        let mut sessions = self.sessions.write().await;
        let session = sessions.entry(key).or_default();
        session.count = session.count.saturating_add(1);
    }

    /// Records what a reserved target's wake attempt actually did, `ORed`
    /// into the session's accumulated flags. Call after
    /// [`reserve`](Self::reserve); a call for a key with no active
    /// reservation is a no-op.
    pub async fn record_outcome(&self, key: PowerHostKey, woke: bool, started_agent: bool) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&key) {
            session.woke |= woke;
            session.started_agent |= started_agent;
        }
    }

    /// Convenience for callers that already know the outcome up front (e.g.
    /// tests): [`reserve`](Self::reserve) immediately followed by
    /// [`record_outcome`](Self::record_outcome).
    #[cfg(test)]
    async fn begin(&self, key: PowerHostKey, woke: bool, started_agent: bool) {
        self.reserve(key).await;
        self.record_outcome(key, woke, started_agent).await;
    }

    /// Ends this target's participation in `key`'s session. Returns the
    /// session's accumulated `(woke, started_agent)` flags once this was the
    /// last participant -- the signal to actually tear the host down -- or
    /// `None` while other targets are still relying on it.
    pub async fn end(&self, key: PowerHostKey) -> Option<(bool, bool)> {
        let mut sessions = self.sessions.write().await;
        let done = match sessions.get_mut(&key) {
            Some(session) => {
                session.count = session.count.saturating_sub(1);
                session.count == 0
            }
            None => return None,
        };
        if !done {
            return None;
        }
        sessions
            .remove(&key)
            .map(|session| (session.woke, session.started_agent))
    }
}

/// What happened while making sure an agent's host was reachable.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentPowerOutcome {
    /// Whether this run sent a Wake-on-LAN packet that brought the host
    /// online.
    pub woke: bool,
    /// Whether this run started the agent process over SSH.
    pub started_agent: bool,
}

/// What happened while making sure a repository's host was reachable.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoPowerOutcome {
    /// Whether this run sent a Wake-on-LAN packet that brought the host
    /// online.
    pub woke: bool,
}

/// Records one step of a run's power-management timeline: both persists it
/// (for the run detail history) and pushes it live over the UI WebSocket (so
/// an open run detail view updates without polling).
/// Identifies which of a multi-target schedule's target pairings a run
/// event belongs to -- `run_id` alone can't, since every target in the
/// schedule shares it. See the `backup_run_events` migration.
#[derive(Debug, Clone, Copy)]
struct TargetIds {
    agent_id: i64,
    repo_id: i64,
}

async fn record_event(
    ctx: PowerCtx<'_>,
    run_id: &str,
    target_ids: TargetIds,
    target: RunEventTarget,
    event_type: RunEventType,
    message: impl Into<String>,
    hostname: &str,
) {
    let message = message.into();
    let occurred_at = match run_events::insert_run_event(
        ctx.pool,
        run_id,
        target_ids.agent_id,
        target_ids.repo_id,
        target,
        event_type,
        &message,
    )
    .await
    {
        Ok(row) => row.occurred_at,
        Err(e) => {
            warn!(error = %e, run_id, "failed to record run event");
            chrono::Utc::now()
        }
    };
    ctx.ui_broadcast.send(ServerToUi::RunEvent {
        run_id: run_id.to_owned(),
        target,
        event_type,
        message,
        occurred_at,
        hostname: hostname.to_owned(),
    });
}

/// Sends a Wake-on-LAN magic packet for `mac` to `broadcast_addr`.
async fn send_wol_packet(mac: MacAddress, broadcast_addr: &str) -> Result<(), WolError> {
    let addr: IpAddr = broadcast_addr
        .parse()
        .map_err(|_| WolError::InvalidBroadcastAddress(broadcast_addr.to_owned()))?;
    let socket = UdpSocket::bind("0.0.0.0:0").await.map_err(WolError::Io)?;
    socket.set_broadcast(true).map_err(WolError::Io)?;
    socket
        .send_to(&mac.magic_packet(), (addr, WOL_PORT))
        .await
        .map_err(WolError::Io)?;
    Ok(())
}

/// A Wake-on-LAN packet could not be sent.
#[derive(Debug, thiserror::Error)]
enum WolError {
    /// The configured broadcast address isn't a valid IP address.
    #[error("invalid broadcast address: {0}")]
    InvalidBroadcastAddress(String),
    /// The UDP socket could not be bound or written to.
    #[error("failed to send Wake-on-LAN packet: {0}")]
    Io(std::io::Error),
}

/// Converts a DB `INTEGER` port to `u16`, falling back to 22 -- the CHECK
/// constraint (indirectly, via the port actually being reachable) keeps this
/// in range in practice, but the DB's `i32` doesn't prove it to the type
/// system.
fn port_u16(port: i32) -> u16 {
    u16::try_from(port).unwrap_or(22)
}

/// Converts a DB `wake_timeout_seconds`/similar `i32` to a [`Duration`],
/// falling back to 180s.
fn timeout_duration(seconds: i32) -> Duration {
    Duration::from_secs(u64::try_from(seconds).unwrap_or(180))
}

/// Polls `check` every [`POLL_INTERVAL`] until it returns `true` or
/// `timeout` elapses.
async fn wait_for<F, Fut>(timeout: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let start = Instant::now();
    loop {
        if check().await {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        sleep(POLL_INTERVAL).await;
    }
}

/// A short-lived SSH connection attempt, used only to answer "is the
/// repository host reachable right now" -- not a full `borg` check.
async fn repo_reachable(repo: &RepoRow) -> bool {
    matches!(
        tokio::time::timeout(
            PROBE_TIMEOUT,
            ssh::connect_with_key(
                &repo.ssh_host,
                &repo.ssh_user,
                port_u16(repo.ssh_port),
                None
            ),
        )
        .await,
        Ok(Ok(_))
    )
}

/// Makes sure `agent`'s host is reachable before a backup runs: if it's
/// already connected, does nothing. Otherwise, if wake is enabled, sends a
/// Wake-on-LAN packet and waits; if the agent still isn't connected once the
/// host is up (or wake is disabled) and starting it over SSH is enabled,
/// starts the agent's systemd unit and waits again. Always returns -- the
/// caller checks reachability itself afterward, the same way it always has.
pub async fn ensure_agent_online(
    ctx: PowerCtx<'_>,
    agent: &AgentRow,
    repo_id: i64,
    run_id: &str,
) -> AgentPowerOutcome {
    let mut outcome = AgentPowerOutcome::default();
    let target_ids = TargetIds {
        agent_id: agent.id,
        repo_id,
    };

    if !agent.wake_enabled && !agent.start_agent_enabled {
        return outcome;
    }
    if ctx.registry.is_connected(agent.id).await {
        return outcome;
    }

    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Source,
        RunEventType::ReachabilityCheck,
        "Checked agent -- no response",
        &agent.hostname,
    )
    .await;

    if agent.wake_enabled {
        outcome.woke = wake_agent_host(ctx, agent, target_ids, run_id).await;
        if outcome.woke {
            record_event(
                ctx,
                run_id,
                target_ids,
                RunEventTarget::Source,
                RunEventType::HostOnline,
                "Host came online",
                &agent.hostname,
            )
            .await;
        }
    }

    if !ctx.registry.is_connected(agent.id).await && agent.start_agent_enabled {
        // `started_agent` tracks whether the SSH start command itself
        // succeeded, not whether the agent went on to reconnect within
        // `wake_timeout_seconds` -- teardown must stop/shut down what this
        // run actually started even if the reconnect happens to run long,
        // rather than silently never undoing it. Waiting for the connect is
        // still worth doing so the timeline records when it happened, but
        // its outcome no longer feeds back into that decision.
        outcome.started_agent = start_agent_process(ctx, agent, target_ids, run_id).await;
        if outcome.started_agent
            && wait_for(timeout_duration(agent.wake_timeout_seconds), || {
                ctx.registry.is_connected(agent.id)
            })
            .await
        {
            record_event(
                ctx,
                run_id,
                target_ids,
                RunEventTarget::Source,
                RunEventType::AgentConnected,
                "Agent connected",
                &agent.hostname,
            )
            .await;
        }
    }

    outcome
}

/// Sends the Wake-on-LAN packet for `agent` and waits for it to connect,
/// logging and returning `false` on any failure along the way (invalid MAC,
/// send failure) rather than aborting the run.
async fn wake_agent_host(
    ctx: PowerCtx<'_>,
    agent: &AgentRow,
    target_ids: TargetIds,
    run_id: &str,
) -> bool {
    let Some(mac) = agent
        .wake_mac_address
        .as_deref()
        .and_then(|s| MacAddress::from_str(s).ok())
    else {
        warn!(
            agent_id = agent.id,
            "wake enabled with no valid MAC address configured"
        );
        return false;
    };
    let broadcast = agent
        .wake_broadcast_address
        .as_deref()
        .unwrap_or(DEFAULT_BROADCAST_ADDR);

    if let Err(e) = send_wol_packet(mac, broadcast).await {
        warn!(agent_id = agent.id, error = %e, "failed to send Wake-on-LAN packet");
        return false;
    }
    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Source,
        RunEventType::WakeSent,
        format!("Sent Wake-on-LAN packet to {mac}"),
        &agent.hostname,
    )
    .await;

    wait_for(timeout_duration(agent.wake_timeout_seconds), || {
        ctx.registry.is_connected(agent.id)
    })
    .await
}

/// Starts `agent`'s systemd unit over SSH, logging and returning `false` on
/// any failure along the way rather than aborting the run. The returned
/// bool reflects only whether the SSH start command itself succeeded --
/// callers wait separately for the agent to actually reconnect, since that
/// outcome must not gate whether this run is on the hook to stop what it
/// just started.
async fn start_agent_process(
    ctx: PowerCtx<'_>,
    agent: &AgentRow,
    target_ids: TargetIds,
    run_id: &str,
) -> bool {
    let Some(ssh_host) = agent.ssh_host.as_deref() else {
        warn!(
            agent_id = agent.id,
            "start agent enabled with no SSH host configured"
        );
        return false;
    };
    let ssh_user = agent.last_ssh_user.as_deref().unwrap_or(FALLBACK_SSH_USER);

    match tokio::time::timeout(
        SSH_ACTION_TIMEOUT,
        ssh::set_systemd_service(
            ssh_host,
            ssh_user,
            port_u16(agent.ssh_port),
            &agent.agent_service_name,
            SystemctlAction::Start,
        ),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            warn!(agent_id = agent.id, error = %e, "failed to start agent process over SSH");
            return false;
        }
        Err(_) => {
            warn!(
                agent_id = agent.id,
                "timed out starting agent process over SSH"
            );
            return false;
        }
    }
    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Source,
        RunEventType::AgentStartSent,
        format!("Started {} over SSH", agent.agent_service_name),
        &agent.hostname,
    )
    .await;

    true
}

/// Makes sure `repo`'s host is reachable over SSH before a backup writes to
/// it: if it already is, does nothing. Otherwise, if wake is enabled, sends
/// a Wake-on-LAN packet and waits. Always returns -- a repository host that
/// never comes back online simply fails the backup naturally when borg
/// tries to reach it, the same way it always has.
pub async fn ensure_repo_online(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    agent_id: i64,
    run_id: &str,
    hostname: &str,
) -> RepoPowerOutcome {
    let mut outcome = RepoPowerOutcome::default();
    let target_ids = TargetIds {
        agent_id,
        repo_id: repo.id,
    };

    if !repo.wake_enabled || repo_reachable(repo).await {
        return outcome;
    }

    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Repository,
        RunEventType::ReachabilityCheck,
        "Checked SSH -- no response",
        hostname,
    )
    .await;

    let Some(mac) = repo
        .wake_mac_address
        .as_deref()
        .and_then(|s| MacAddress::from_str(s).ok())
    else {
        warn!(
            repo_id = repo.id,
            "wake enabled with no valid MAC address configured"
        );
        return outcome;
    };
    let broadcast = repo
        .wake_broadcast_address
        .as_deref()
        .unwrap_or(DEFAULT_BROADCAST_ADDR);

    if let Err(e) = send_wol_packet(mac, broadcast).await {
        warn!(repo_id = repo.id, error = %e, "failed to send Wake-on-LAN packet");
        return outcome;
    }
    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Repository,
        RunEventType::WakeSent,
        format!("Sent Wake-on-LAN packet to {mac}"),
        hostname,
    )
    .await;

    outcome.woke = wait_for(timeout_duration(repo.wake_timeout_seconds), || {
        repo_reachable(repo)
    })
    .await;

    if outcome.woke {
        record_event(
            ctx,
            run_id,
            target_ids,
            RunEventTarget::Repository,
            RunEventType::HostOnline,
            "Host online -- SSH reachable",
            hostname,
        )
        .await;
    }

    outcome
}

/// Shuts an agent's host down (if this session woke it) or stops the agent
/// process (if this session started it, and the host wasn't also shut down)
/// -- but only once every concurrently-running target relying on this host
/// has finished, per [`PowerSessionTracker`]. Best-effort: logs failures
/// rather than failing the run, since the backup itself already completed.
pub async fn teardown_agent_power(ctx: PowerCtx<'_>, agent: &AgentRow, repo_id: i64, run_id: &str) {
    let Some((woke, started_agent)) = ctx.power_sessions.end(PowerHostKey::Agent(agent.id)).await
    else {
        return;
    };
    if !woke && !started_agent {
        return;
    }
    let Some(ssh_host) = agent.ssh_host.as_deref() else {
        return;
    };
    let ssh_user = agent.last_ssh_user.as_deref().unwrap_or(FALLBACK_SSH_USER);
    let port = port_u16(agent.ssh_port);
    let target_ids = TargetIds {
        agent_id: agent.id,
        repo_id,
    };

    if (woke || started_agent) && agent.shutdown_after_backup {
        record_event(
            ctx,
            run_id,
            target_ids,
            RunEventTarget::Source,
            RunEventType::ShutdownSent,
            "Shutting down host",
            &agent.hostname,
        )
        .await;
        match tokio::time::timeout(
            SSH_ACTION_TIMEOUT,
            ssh::shutdown_host(ssh_host, ssh_user, port),
        )
        .await
        {
            Ok(Ok(())) => {
                record_event(
                    ctx,
                    run_id,
                    target_ids,
                    RunEventTarget::Source,
                    RunEventType::HostOffline,
                    "Host offline",
                    &agent.hostname,
                )
                .await;
            }
            Ok(Err(e)) => warn!(agent_id = agent.id, error = %e, "failed to shut down agent host"),
            Err(_) => warn!(agent_id = agent.id, "timed out shutting down agent host"),
        }
    } else if started_agent && agent.stop_agent_after_backup {
        record_event(
            ctx,
            run_id,
            target_ids,
            RunEventTarget::Source,
            RunEventType::AgentStopSent,
            "Stopping agent",
            &agent.hostname,
        )
        .await;
        match tokio::time::timeout(
            SSH_ACTION_TIMEOUT,
            ssh::set_systemd_service(
                ssh_host,
                ssh_user,
                port,
                &agent.agent_service_name,
                SystemctlAction::Stop,
            ),
        )
        .await
        {
            Ok(Ok(())) => {
                record_event(
                    ctx,
                    run_id,
                    target_ids,
                    RunEventTarget::Source,
                    RunEventType::AgentStopped,
                    "Agent stopped",
                    &agent.hostname,
                )
                .await;
            }
            Ok(Err(e)) => warn!(agent_id = agent.id, error = %e, "failed to stop agent process"),
            Err(_) => warn!(agent_id = agent.id, "timed out stopping agent process"),
        }
    }
}

/// Shuts a repository's host down, if this session woke it and every
/// concurrently-running target relying on it has finished. Best-effort: logs
/// failures rather than failing the run.
pub async fn teardown_repo_power(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    agent_id: i64,
    run_id: &str,
    hostname: &str,
) {
    let Some((woke, _)) = ctx.power_sessions.end(PowerHostKey::Repo(repo.id)).await else {
        return;
    };
    if !woke || !repo.shutdown_after_backup {
        return;
    }
    let target_ids = TargetIds {
        agent_id,
        repo_id: repo.id,
    };

    record_event(
        ctx,
        run_id,
        target_ids,
        RunEventTarget::Repository,
        RunEventType::ShutdownSent,
        "Shutting down host",
        hostname,
    )
    .await;
    match tokio::time::timeout(
        SSH_ACTION_TIMEOUT,
        ssh::shutdown_host(&repo.ssh_host, &repo.ssh_user, port_u16(repo.ssh_port)),
    )
    .await
    {
        Ok(Ok(())) => {
            record_event(
                ctx,
                run_id,
                target_ids,
                RunEventTarget::Repository,
                RunEventType::HostOffline,
                "Host offline",
                hostname,
            )
            .await;
        }
        Ok(Err(e)) => warn!(repo_id = repo.id, error = %e, "failed to shut down repository host"),
        Err(_) => warn!(repo_id = repo.id, "timed out shutting down repository host"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_address_parses_and_displays_canonically() {
        let mac: MacAddress = "3c:97:0e:2b:9a:44".parse().unwrap();
        assert_eq!(mac.to_string(), "3C:97:0E:2B:9A:44");
    }

    #[test]
    fn mac_address_rejects_malformed_input() {
        assert!("not-a-mac".parse::<MacAddress>().is_err());
        assert!("AA:BB:CC:DD:EE".parse::<MacAddress>().is_err());
        assert!("AA:BB:CC:DD:EE:GG".parse::<MacAddress>().is_err());
    }

    // The DB's `..._wake_mac_format` CHECK constraints require exactly two
    // hex digits per octet unconditionally; a value that parses here but
    // fails that constraint surfaces as an opaque 500 instead of a clean
    // 400, so the parser must reject everything the constraint would.
    #[test]
    fn mac_address_rejects_what_the_db_check_constraint_would() {
        assert!("1:2:3:4:5:6".parse::<MacAddress>().is_err());
        assert!("+AA:BB:CC:DD:EE:FF".parse::<MacAddress>().is_err());
        assert!("AA:BB:CC:DD:EE:F".parse::<MacAddress>().is_err());
    }

    #[test]
    fn magic_packet_is_six_ff_bytes_then_the_mac_sixteen_times() {
        let mac: MacAddress = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let packet = mac.magic_packet();
        assert_eq!(packet.len(), 102);
        assert_eq!(&packet[0..6], &[0xFF; 6]);
        for chunk in packet[6..].as_chunks::<6>().0 {
            assert_eq!(chunk, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        }
    }

    #[tokio::test]
    async fn power_session_tracker_only_signals_done_on_last_participant() {
        let tracker = PowerSessionTracker::default();
        let key = PowerHostKey::Agent(1);

        tracker.begin(key, true, false).await;
        tracker.begin(key, false, true).await;

        // First of two participants ending: session isn't done yet.
        assert!(tracker.end(key).await.is_none());

        // Last participant ending: gets back the accumulated flags.
        assert_eq!(tracker.end(key).await, Some((true, true)));
    }

    #[tokio::test]
    async fn power_session_tracker_end_without_begin_is_none() {
        let tracker = PowerSessionTracker::default();
        assert!(tracker.end(PowerHostKey::Agent(99)).await.is_none());
    }

    use crate::db;

    /// A host guaranteed to refuse the connection immediately (nothing
    /// listens on port 1), rather than actually being reachable or hanging
    /// until a TCP timeout -- lets the wake/teardown tests exercise the SSH
    /// failure path deterministically and fast.
    const UNREACHABLE_HOST: &str = "127.0.0.1";
    const UNREACHABLE_PORT: i32 = 1;

    async fn test_agent(pool: &sqlx::PgPool) -> AgentRow {
        db::insert_agent(pool, "power-test-agent", None, "hash", None, None)
            .await
            .unwrap()
    }

    async fn test_repo(pool: &sqlx::PgPool) -> RepoRow {
        db::insert_repo(
            pool,
            &db::InsertRepoParams {
                name: "power-test-repo",
                repo_path: "/backup/power-test",
                ssh_user: "borg",
                ssh_host: UNREACHABLE_HOST,
                ssh_port: UNREACHABLE_PORT,
                passphrase_encrypted: b"irrelevant",
                compression: "lz4",
                encryption: "none",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .unwrap()
    }

    fn ctx<'a>(
        pool: &'a sqlx::PgPool,
        registry: &'a AgentRegistry,
        ui_broadcast: &'a UiBroadcast,
        power_sessions: &'a PowerSessionTracker,
    ) -> PowerCtx<'a> {
        PowerCtx {
            pool,
            registry,
            ui_broadcast,
            power_sessions,
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_is_a_noop_when_wake_and_start_are_both_disabled(
        pool: sqlx::PgPool,
    ) {
        let agent = test_agent(&pool).await;
        let repo = test_repo(&pool).await;
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        assert!(!outcome.woke);
        assert!(!outcome.started_agent);
        assert!(
            db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_is_a_noop_when_already_connected(pool: sqlx::PgPool) {
        let repo = test_repo(&pool).await;
        let agent = db::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 1,
                shutdown_after_backup: false,
                start_agent_enabled: false,
                stop_agent_after_backup: false,
                ssh_host: None,
                ssh_port: 22,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();

        let registry = AgentRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        registry.register(agent.id, tx, false, None).await;
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        assert!(!outcome.woke);
        assert!(
            db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_records_events_and_gives_up_when_host_never_comes_up(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = db::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 1,
                shutdown_after_backup: false,
                start_agent_enabled: true,
                stop_agent_after_backup: false,
                ssh_host: Some(UNREACHABLE_HOST),
                ssh_port: UNREACHABLE_PORT,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();

        let registry = AgentRegistry::new(); // never actually connects
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        assert!(!outcome.woke, "the agent never connects in this test");
        assert!(
            !outcome.started_agent,
            "the SSH host refuses the connection"
        );

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["reachability_check", "wake_sent"]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_repo_online_is_a_noop_when_wake_disabled(pool: sqlx::PgPool) {
        let agent = test_agent(&pool).await;
        let repo = test_repo(&pool).await;
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_repo_online(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            agent.id,
            "run-1",
            "repo-host",
        )
        .await;

        assert!(!outcome.woke);
        assert!(
            db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_repo_online_records_events_and_gives_up_when_host_never_comes_up(
        pool: sqlx::PgPool,
    ) {
        let agent = test_agent(&pool).await;
        let repo = db::update_repo_power(
            &pool,
            test_repo(&pool).await.id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("9C:B6:D0:1A:44:7F"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 1,
                shutdown_after_backup: false,
            },
        )
        .await
        .unwrap();

        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_repo_online(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            agent.id,
            "run-1",
            "repo-host",
        )
        .await;

        assert!(!outcome.woke, "the unreachable host never answers SSH");

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["reachability_check", "wake_sent"]);
    }

    #[tokio::test]
    async fn teardown_agent_power_does_nothing_while_a_sibling_run_still_needs_the_host() {
        let sessions = PowerSessionTracker::default();
        let key = PowerHostKey::Agent(1);
        sessions.begin(key, true, false).await;
        sessions.begin(key, true, false).await; // a sibling schedule still running

        // Ending only this run's participation must not shut anything down
        // while the sibling is still relying on the host.
        assert!(sessions.end(key).await.is_none());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_does_nothing_when_this_run_neither_woke_nor_started_it(
        pool: sqlx::PgPool,
    ) {
        let agent = test_agent(&pool).await;
        let repo = test_repo(&pool).await;
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        sessions
            .begin(PowerHostKey::Agent(agent.id), false, false)
            .await;

        teardown_agent_power(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        assert!(
            db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_attempts_shutdown_when_this_run_woke_the_host(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = db::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
                start_agent_enabled: false,
                stop_agent_after_backup: false,
                ssh_host: Some(UNREACHABLE_HOST),
                ssh_port: UNREACHABLE_PORT,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        sessions
            .begin(PowerHostKey::Agent(agent.id), true, false)
            .await;

        teardown_agent_power(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        // The SSH shutdown attempt fails (nothing listens on the port), so
        // only the attempt itself is recorded, not a HostOffline event.
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["shutdown_sent"]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_attempts_stop_when_this_run_started_the_agent(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = db::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            db::AgentPowerPatch {
                wake_enabled: false,
                wake_mac_address: None,
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: false,
                start_agent_enabled: true,
                stop_agent_after_backup: true,
                ssh_host: Some(UNREACHABLE_HOST),
                ssh_port: UNREACHABLE_PORT,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        sessions
            .begin(PowerHostKey::Agent(agent.id), false, true)
            .await;

        teardown_agent_power(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        // The SSH stop attempt fails (nothing listens on the port), so only
        // the attempt itself is recorded, not the terminal AgentStopped
        // event that a successful stop records.
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["agent_stop_sent"]);
    }

    /// Regression test: a host that isn't a persistent boot service needs
    /// both `wake_enabled` (to power the machine on) and `start_agent_enabled`
    /// (to bring the agent process up over SSH once it is) -- the WOL wait in
    /// `ensure_agent_online` times out for exactly this host shape, since the
    /// agent never reconnects on its own, so `woke` stays `false` even though
    /// this run is what brought the host up. Shutdown must still fire off the
    /// `started_agent` flag alone, not require `woke` too.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_attempts_shutdown_when_this_run_only_started_the_agent(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = db::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            db::AgentPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
                start_agent_enabled: true,
                stop_agent_after_backup: false,
                ssh_host: Some(UNREACHABLE_HOST),
                ssh_port: UNREACHABLE_PORT,
                agent_service_name: "assimilate-agent",
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        // The WOL wait timed out (host came up too slowly to reconnect on its
        // own) but the SSH start succeeded, matching the scenario above.
        sessions
            .begin(PowerHostKey::Agent(agent.id), false, true)
            .await;

        teardown_agent_power(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
        )
        .await;

        // Shutdown must be attempted -- not the stop-agent branch -- even
        // though `woke` is false, because `started_agent` alone means this
        // run is responsible for the host being up.
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["shutdown_sent"]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_repo_power_does_nothing_when_this_run_did_not_wake_it(pool: sqlx::PgPool) {
        let agent = test_agent(&pool).await;
        let repo = test_repo(&pool).await;
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        sessions
            .begin(PowerHostKey::Repo(repo.id), false, false)
            .await;

        teardown_repo_power(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            agent.id,
            "run-1",
            "repo-host",
        )
        .await;

        assert!(
            db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_repo_power_attempts_shutdown_when_this_run_woke_it(pool: sqlx::PgPool) {
        let agent = test_agent(&pool).await;
        let repo = db::update_repo_power(
            &pool,
            test_repo(&pool).await.id,
            db::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("9C:B6:D0:1A:44:7F"),
                wake_broadcast_address: None,
                wake_timeout_seconds: 180,
                shutdown_after_backup: true,
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        sessions
            .begin(PowerHostKey::Repo(repo.id), true, false)
            .await;

        teardown_repo_power(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            agent.id,
            "run-1",
            "repo-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(event_types, vec!["shutdown_sent"]);
    }
}

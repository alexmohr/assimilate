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
        let octets: Vec<u8> = s
            .split(':')
            .map(|part| u8::from_str_radix(part, 16).map_err(|_| MacAddressParseError))
            .collect::<Result<_, _>>()?;
        let octets: [u8; 6] = octets.try_into().map_err(|_| MacAddressParseError)?;
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
    /// its run.
    pub async fn begin(&self, key: PowerHostKey, woke: bool, started_agent: bool) {
        let mut sessions = self.sessions.write().await;
        let session = sessions.entry(key).or_default();
        session.count = session.count.saturating_add(1);
        session.woke |= woke;
        session.started_agent |= started_agent;
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
async fn record_event(
    ctx: PowerCtx<'_>,
    run_id: &str,
    target: RunEventTarget,
    event_type: RunEventType,
    message: impl Into<String>,
    hostname: &str,
) {
    let message = message.into();
    let occurred_at =
        match run_events::insert_run_event(ctx.pool, run_id, target, event_type, &message).await {
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
    run_id: &str,
) -> AgentPowerOutcome {
    let mut outcome = AgentPowerOutcome::default();

    if !agent.wake_enabled && !agent.start_agent_enabled {
        return outcome;
    }
    if ctx.registry.is_connected(agent.id).await {
        return outcome;
    }

    record_event(
        ctx,
        run_id,
        RunEventTarget::Source,
        RunEventType::ReachabilityCheck,
        "Checked agent -- no response",
        &agent.hostname,
    )
    .await;

    if agent.wake_enabled {
        outcome.woke = wake_agent_host(ctx, agent, run_id).await;
        if outcome.woke {
            record_event(
                ctx,
                run_id,
                RunEventTarget::Source,
                RunEventType::HostOnline,
                "Host came online",
                &agent.hostname,
            )
            .await;
        }
    }

    if !ctx.registry.is_connected(agent.id).await && agent.start_agent_enabled {
        outcome.started_agent = start_agent_process(ctx, agent, run_id).await;
        if outcome.started_agent {
            record_event(
                ctx,
                run_id,
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
async fn wake_agent_host(ctx: PowerCtx<'_>, agent: &AgentRow, run_id: &str) -> bool {
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

/// Starts `agent`'s systemd unit over SSH and waits for it to connect,
/// logging and returning `false` on any failure along the way rather than
/// aborting the run.
async fn start_agent_process(ctx: PowerCtx<'_>, agent: &AgentRow, run_id: &str) -> bool {
    let Some(ssh_host) = agent.ssh_host.as_deref() else {
        warn!(
            agent_id = agent.id,
            "start agent enabled with no SSH host configured"
        );
        return false;
    };
    let ssh_user = agent.last_ssh_user.as_deref().unwrap_or(FALLBACK_SSH_USER);

    if let Err(e) = ssh::set_systemd_service(
        ssh_host,
        ssh_user,
        port_u16(agent.ssh_port),
        &agent.agent_service_name,
        SystemctlAction::Start,
    )
    .await
    {
        warn!(agent_id = agent.id, error = %e, "failed to start agent process over SSH");
        return false;
    }
    record_event(
        ctx,
        run_id,
        RunEventTarget::Source,
        RunEventType::AgentStartSent,
        format!("Started {} over SSH", agent.agent_service_name),
        &agent.hostname,
    )
    .await;

    wait_for(timeout_duration(agent.wake_timeout_seconds), || {
        ctx.registry.is_connected(agent.id)
    })
    .await
}

/// Makes sure `repo`'s host is reachable over SSH before a backup writes to
/// it: if it already is, does nothing. Otherwise, if wake is enabled, sends
/// a Wake-on-LAN packet and waits. Always returns -- a repository host that
/// never comes back online simply fails the backup naturally when borg
/// tries to reach it, the same way it always has.
pub async fn ensure_repo_online(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    run_id: &str,
    hostname: &str,
) -> RepoPowerOutcome {
    let mut outcome = RepoPowerOutcome::default();

    if !repo.wake_enabled || repo_reachable(repo).await {
        return outcome;
    }

    record_event(
        ctx,
        run_id,
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
pub async fn teardown_agent_power(ctx: PowerCtx<'_>, agent: &AgentRow, run_id: &str) {
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

    if woke && agent.shutdown_after_backup {
        record_event(
            ctx,
            run_id,
            RunEventTarget::Source,
            RunEventType::ShutdownSent,
            "Shutting down host",
            &agent.hostname,
        )
        .await;
        match ssh::shutdown_host(ssh_host, ssh_user, port).await {
            Ok(()) => {
                record_event(
                    ctx,
                    run_id,
                    RunEventTarget::Source,
                    RunEventType::HostOffline,
                    "Host offline",
                    &agent.hostname,
                )
                .await;
            }
            Err(e) => warn!(agent_id = agent.id, error = %e, "failed to shut down agent host"),
        }
    } else if started_agent && agent.stop_agent_after_backup {
        record_event(
            ctx,
            run_id,
            RunEventTarget::Source,
            RunEventType::AgentStopSent,
            "Stopping agent",
            &agent.hostname,
        )
        .await;
        if let Err(e) = ssh::set_systemd_service(
            ssh_host,
            ssh_user,
            port,
            &agent.agent_service_name,
            SystemctlAction::Stop,
        )
        .await
        {
            warn!(agent_id = agent.id, error = %e, "failed to stop agent process");
        }
    }
}

/// Shuts a repository's host down, if this session woke it and every
/// concurrently-running target relying on it has finished. Best-effort: logs
/// failures rather than failing the run.
pub async fn teardown_repo_power(ctx: PowerCtx<'_>, repo: &RepoRow, run_id: &str, hostname: &str) {
    let Some((woke, _)) = ctx.power_sessions.end(PowerHostKey::Repo(repo.id)).await else {
        return;
    };
    if !woke || !repo.shutdown_after_backup {
        return;
    }

    record_event(
        ctx,
        run_id,
        RunEventTarget::Repository,
        RunEventType::ShutdownSent,
        "Shutting down host",
        hostname,
    )
    .await;
    match ssh::shutdown_host(&repo.ssh_host, &repo.ssh_user, port_u16(repo.ssh_port)).await {
        Ok(()) => {
            record_event(
                ctx,
                run_id,
                RunEventTarget::Repository,
                RunEventType::HostOffline,
                "Host offline",
                hostname,
            )
            .await;
        }
        Err(e) => warn!(repo_id = repo.id, error = %e, "failed to shut down repository host"),
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
}

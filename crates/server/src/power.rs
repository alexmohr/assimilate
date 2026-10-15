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
    types::{RunEventTarget, RunEventType, ScheduleWakeOverride},
};
use tokio::{
    net::UdpSocket,
    sync::{Mutex, RwLock},
    time::sleep,
};
use tracing::warn;

pub use self::orphans::{power_hold_sweep_interval, release_orphaned_holds};

mod orphans;
use crate::{
    db::{
        AgentRow, RepoRow,
        power_holds::{self, PowerHoldRow},
        run_events,
    },
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
pub(crate) const DEFAULT_BROADCAST_ADDR: &str = "255.255.255.255";
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
    /// A repository host, identified by its database ID. Keyed by host rather
    /// than by repository: two repositories on one machine written in the
    /// same run share one session, so the machine is woken once and shut down
    /// only after the last of them is done.
    RepoHost(i64),
}

impl PowerHostKey {
    /// The key for host `host_id` of kind `kind`.
    #[must_use]
    pub const fn new(kind: PowerHostKind, host_id: i64) -> Self {
        match kind {
            PowerHostKind::Agent => Self::Agent(host_id),
            PowerHostKind::RepoHost => Self::RepoHost(host_id),
        }
    }

    /// Which kind of host this is.
    #[must_use]
    pub const fn kind(self) -> PowerHostKind {
        match self {
            Self::Agent(_) => PowerHostKind::Agent,
            Self::RepoHost(_) => PowerHostKind::RepoHost,
        }
    }

    /// The host's database ID: the agent's, or the repository host's.
    #[must_use]
    pub const fn host_id(self) -> i64 {
        match self {
            Self::Agent(id) | Self::RepoHost(id) => id,
        }
    }
}

/// The kind of host a [`PowerHostKey`] names, as stored in
/// `power_holds.host_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display, strum_macros::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum PowerHostKind {
    /// An agent's host.
    Agent,
    /// A repository host.
    RepoHost,
}

impl sqlx::Type<sqlx::Postgres> for PowerHostKind {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for PowerHostKind {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let text = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(text.parse::<Self>()?)
    }
}

/// Who holds a power-managed host: the run, and the target within it. A
/// dependency's hold on a repository host has no repository - it is the
/// agent's dependency, not the repository's, and that is also what tells it
/// apart from the hold the same target keeps on its own repository's host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldOwner<'a> {
    /// The run.
    pub run_id: &'a str,
    /// The target's agent.
    pub agent_id: i64,
    /// The target's repository, or `None` for a dependency's hold.
    pub repo_id: Option<i64>,
}

impl<'a> HoldOwner<'a> {
    /// The hold one backup target - an agent writing to a repository - keeps
    /// on either of its hosts.
    #[must_use]
    pub const fn target(run_id: &'a str, agent_id: i64, repo_id: i64) -> Self {
        Self {
            run_id,
            agent_id,
            repo_id: Some(repo_id),
        }
    }

    /// The hold a dependency check keeps on the repository host a dependency
    /// shares a machine with.
    #[must_use]
    pub const fn dependency(run_id: &'a str, agent_id: i64) -> Self {
        Self {
            run_id,
            agent_id,
            repo_id: None,
        }
    }
}

/// A hold owner for tests that only exercise the in-memory counting.
#[cfg(test)]
pub(crate) const TEST_HOLD: HoldOwner<'static> = HoldOwner::target("run-test", 1, 1);

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
/// always calls [`reserve`](Self::reserve) before trying to reach a host,
/// [`record_outcome`](Self::record_outcome) once it has, and
/// [`end`](Self::end) once after its run completes; only the call that brings
/// the session's count back to zero gets back the accumulated
/// `(woke, started_agent)` flags to act on.
///
/// The counts in memory are what every decision is made on. A tracker built
/// with [`persistent`](Self::persistent) also mirrors each hold to the
/// `power_holds` table, best-effort: a failed write is logged and never holds
/// up or fails a run. That mirror is what [`recover`](Self::recover) reads
/// back after a restart, so a machine a run woke before the restart is still
/// shut down once that run is done - see [`release_orphaned_holds`].
#[derive(Clone, Default)]
pub struct PowerSessionTracker {
    sessions: Arc<RwLock<HashMap<PowerHostKey, PowerSession>>>,
    store: Option<sqlx::PgPool>,
    orphans: Arc<Mutex<Vec<PowerHoldRow>>>,
}

impl PowerSessionTracker {
    /// A tracker that mirrors its holds to `pool`'s `power_holds` table.
    #[must_use]
    pub fn persistent(pool: sqlx::PgPool) -> Self {
        Self {
            store: Some(pool),
            ..Self::default()
        }
    }

    /// Registers `owner` as relying on `key` staying up for the duration of
    /// its run, *before* attempting to reach/wake it. Reserving up front
    /// (rather than only once the wake attempt has resolved) is what makes
    /// the reference count correct while two targets are concurrently
    /// waking the same host: without it, a target whose wake resolves
    /// quickly could tear the host down via [`end`](Self::end) while a
    /// slower sibling is still mid-wait, because the tracker wouldn't know
    /// the sibling exists yet.
    pub async fn reserve(&self, key: PowerHostKey, owner: HoldOwner<'_>) {
        {
            let mut sessions = self.sessions.write().await;
            let session = sessions.entry(key).or_default();
            session.count = session.count.saturating_add(1);
        }
        if let Some(pool) = &self.store
            && let Err(e) = power_holds::insert_power_hold(pool, key, owner).await
        {
            warn!(?key, run_id = owner.run_id, error = %e, "failed to persist a power hold");
        }
    }

    /// Records what a reserved target's wake attempt actually did, `ORed`
    /// into the session's accumulated flags. Call after
    /// [`reserve`](Self::reserve); a call for a key with no active
    /// reservation is a no-op.
    pub async fn record_outcome(&self, key: PowerHostKey, woke: bool, started_agent: bool) {
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(&key) {
                session.woke |= woke;
                session.started_agent |= started_agent;
            }
        }
        if let Some(pool) = &self.store
            && (woke || started_agent)
            && let Err(e) =
                power_holds::record_power_hold_outcome(pool, key, woke, started_agent).await
        {
            warn!(?key, error = %e, "failed to persist a power hold's outcome");
        }
    }

    /// Convenience for callers that already know the outcome up front (e.g.
    /// tests): [`reserve`](Self::reserve) immediately followed by
    /// [`record_outcome`](Self::record_outcome).
    #[cfg(test)]
    async fn begin(&self, key: PowerHostKey, woke: bool, started_agent: bool) {
        self.reserve(key, TEST_HOLD).await;
        self.record_outcome(key, woke, started_agent).await;
    }

    /// Ends `owner`'s participation in `key`'s session. Returns the
    /// session's accumulated `(woke, started_agent)` flags once this was the
    /// last participant -- the signal to actually tear the host down -- or
    /// `None` while other targets are still relying on it.
    pub async fn end(&self, key: PowerHostKey, owner: HoldOwner<'_>) -> Option<(bool, bool)> {
        let flags = self.end_in_memory(key).await;
        if let Some(pool) = &self.store
            && let Err(e) = power_holds::delete_power_hold(pool, key, owner).await
        {
            warn!(?key, run_id = owner.run_id, error = %e, "failed to delete a power hold");
        }
        flags
    }

    async fn end_in_memory(&self, key: PowerHostKey) -> Option<(bool, bool)> {
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

    /// Reads back the holds a previous server process left behind and counts
    /// them as held again, so a run started after the restart neither shuts
    /// a machine down under them nor loses the credit for having woken it.
    /// They are kept as orphans for [`release_orphaned_holds`] to release
    /// once their runs are done. Call once at startup, before any run can
    /// start. Returns how many holds were recovered.
    pub async fn recover(&self) -> usize {
        let Some(pool) = &self.store else {
            return 0;
        };
        let rows = match power_holds::list_power_holds(pool).await {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %e, "failed to read back the power holds of interrupted runs");
                return 0;
            }
        };
        {
            let mut sessions = self.sessions.write().await;
            for row in &rows {
                let session = sessions.entry(row.key).or_default();
                session.count = session.count.saturating_add(1);
                session.woke |= row.woke;
                session.started_agent |= row.started_agent;
            }
        }
        let recovered = rows.len();
        self.orphans.lock().await.extend(rows);
        recovered
    }

    /// Takes every hold still waiting to be released, for
    /// [`release_orphaned_holds`] to go through.
    async fn take_orphans(&self) -> Vec<PowerHoldRow> {
        std::mem::take(&mut *self.orphans.lock().await)
    }

    /// Puts back the holds [`release_orphaned_holds`] kept for a later pass.
    async fn keep_orphans(&self, kept: Vec<PowerHoldRow>) {
        self.orphans.lock().await.extend(kept);
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
    /// Whether the host answered SSH once any wake was spent, or `None` when
    /// it was never asked because no wake was called for.
    pub reachable: Option<bool>,
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

/// Which target of which run a timeline step belongs to, for callers outside
/// this module.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunEventSite<'a> {
    /// The run.
    pub run_id: &'a str,
    /// The target's agent.
    pub agent_id: i64,
    /// The target's repository.
    pub repo_id: i64,
    /// The agent's hostname, carried on the live UI message.
    pub hostname: &'a str,
}

/// Records one step of a run's timeline from outside this module - the
/// dependency check's steps sit on the same timeline as the wake of the
/// source and repository hosts.
pub(crate) async fn record_run_event(
    ctx: PowerCtx<'_>,
    site: RunEventSite<'_>,
    target: RunEventTarget,
    event_type: RunEventType,
    message: impl Into<String>,
) {
    record_event(
        ctx,
        site.run_id,
        TargetIds {
            agent_id: site.agent_id,
            repo_id: site.repo_id,
        },
        target,
        event_type,
        message,
        site.hostname,
    )
    .await;
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
        agent_id: target_ids.agent_id,
        repo_id: target_ids.repo_id,
    });
}

/// Sends a Wake-on-LAN magic packet for `mac` to `broadcast_addr`.
pub(crate) async fn send_wol_packet(mac: MacAddress, broadcast_addr: &str) -> Result<(), WolError> {
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
pub(crate) enum WolError {
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
pub(crate) fn timeout_duration(seconds: i32) -> Duration {
    Duration::from_secs(u64::try_from(seconds).unwrap_or(180))
}

/// Polls `check` every [`POLL_INTERVAL`] until it returns `true` or
/// `timeout` elapses.
pub(crate) async fn wait_for<F, Fut>(timeout: Duration, mut check: F) -> bool
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
///
/// Only gates a run for a host the schedule waits for - marked as not always
/// online, or one the schedule is set to wait for (see `crate::repo_wait`).
/// For any other host, gating dispatch on this would turn a hiccup reaching
/// it - a slow answer, a refused key, a momentary blip - into a backup that
/// never ran at all, which is a worse outcome than the one it would be
/// reporting; there the scheduler asks only after a run has already failed,
/// to find out whether an absent host is why.
pub(crate) async fn repo_reachable(repo: &RepoRow) -> bool {
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
/// already connected, does nothing. Otherwise, if waking is called for, sends
/// a Wake-on-LAN packet and waits; if the agent still isn't connected once
/// the host is up (or waking isn't called for) and starting it over SSH is
/// enabled, starts the agent's systemd unit and waits again. Always returns
/// -- the caller checks reachability itself afterward, the same way it always
/// has.
///
/// `wake_override` is the running schedule's own answer to whether it wakes
/// its hosts, resolved here against the agent's `wake_enabled` default. It
/// governs waking only: starting the agent process follows
/// `start_agent_enabled` in every variant, since a host that is already
/// powered on needs no wake to be started on.
pub async fn ensure_agent_online(
    ctx: PowerCtx<'_>,
    agent: &AgentRow,
    repo_id: i64,
    run_id: &str,
    wake_override: ScheduleWakeOverride,
) -> AgentPowerOutcome {
    let mut outcome = AgentPowerOutcome::default();
    let target_ids = TargetIds {
        agent_id: agent.id,
        repo_id,
    };
    let wake_enabled = wake_override.resolve(agent.wake_enabled);

    if !wake_enabled && !agent.start_agent_enabled {
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

    if wake_enabled {
        // `outcome.woke` tracks whether the WOL packet was actually sent,
        // not whether the host came back online within `wake_timeout_seconds`
        // -- same reasoning as `started_agent` below: teardown must still
        // shut down a host that boots slower than the timeout, since this
        // run is what woke it regardless of how long that took. Without
        // this, a wake-only host (no start-agent fallback) that's simply
        // slow to boot would never get shutdown credit on the run that woke
        // it, and every later run would then find it already connected and
        // never attempt (or credit) a wake again -- silently and
        // permanently disabling shutdown_after_backup for that host.
        outcome.woke = wake_agent_host(ctx, agent, target_ids, run_id).await;
        if outcome.woke
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

/// Sends the Wake-on-LAN packet for `agent`, logging and returning `false`
/// on any failure along the way (invalid MAC, send failure) rather than
/// aborting the run. The returned bool reflects only whether the packet was
/// sent -- callers wait separately for the host to actually come online,
/// since that outcome must not gate whether this run is on the hook to shut
/// it back down.
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
            "wake called for with no valid MAC address configured"
        );
        record_event(
            ctx,
            run_id,
            target_ids,
            RunEventTarget::Source,
            RunEventType::WakeUnavailable,
            "Cannot wake -- no MAC address configured",
            &agent.hostname,
        )
        .await;
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
        "Sent Wake-on-LAN packet",
        &agent.hostname,
    )
    .await;

    true
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
/// it: if it already is, does nothing. Otherwise, if waking is called for,
/// sends a Wake-on-LAN packet and waits. Always returns -- a repository host
/// that never comes back online simply fails the backup naturally when borg
/// tries to reach it, and [`repo_reachable`] is what the scheduler asks
/// afterwards to find out whether that is why it failed.
///
/// `wake_override` is the running schedule's own answer to whether it wakes
/// its hosts, resolved here against the repository's `wake_enabled` default.
pub async fn ensure_repo_online(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    agent_id: i64,
    run_id: &str,
    hostname: &str,
    wake_override: ScheduleWakeOverride,
) -> RepoPowerOutcome {
    let mut outcome = RepoPowerOutcome::default();
    let target_ids = TargetIds {
        agent_id,
        repo_id: repo.id,
    };

    if !wake_override.resolve(repo.wake_enabled) {
        return outcome;
    }
    if repo_reachable(repo).await {
        outcome.reachable = Some(true);
        return outcome;
    }
    outcome.reachable = Some(false);

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
            "wake called for with no valid MAC address configured"
        );
        record_event(
            ctx,
            run_id,
            target_ids,
            RunEventTarget::Repository,
            RunEventType::WakeUnavailable,
            "Cannot wake -- no MAC address configured",
            hostname,
        )
        .await;
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
        "Sent Wake-on-LAN packet",
        hostname,
    )
    .await;

    // `outcome.woke` reflects only that the packet was sent, not that SSH
    // became reachable within the timeout -- same reasoning as
    // ensure_agent_online's wake path: teardown must still shut this host
    // down even if it's simply slow to boot, and a later run's reachability
    // check succeeding shouldn't be the only way this run gets credit for
    // having woken it.
    outcome.woke = true;
    let answered = wait_for(timeout_duration(repo.wake_timeout_seconds), || {
        repo_reachable(repo)
    })
    .await;
    outcome.reachable = Some(answered);
    if answered {
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
    let Some((woke, started_agent)) = ctx
        .power_sessions
        .end(
            PowerHostKey::Agent(agent.id),
            HoldOwner::target(run_id, agent.id, repo_id),
        )
        .await
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

    // Shutting the whole host down requires this run to have woken it.
    // `started_agent` alone doesn't qualify: `start_agent_process` runs its
    // command over SSH, which only ever succeeds against a host that's
    // already reachable -- so it proves the agent process was started, never
    // that this run brought the host itself up. Without this check, a host
    // that was already on the whole time (only its agent process needed a
    // kick) would get shut down out from under whatever else was using it.
    if woke && agent.shutdown_after_backup {
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
///
/// `reserved_host_id` is the host this target reserved before the run. It is
/// the session released here even if `repo` has since been moved to another
/// host, so the reservation is always given back; the machine is only shut
/// down if it is still the one `repo` lives on.
pub async fn teardown_repo_power(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    reserved_host_id: i64,
    agent_id: i64,
    run_id: &str,
    hostname: &str,
) {
    let Some((woke, _)) = ctx
        .power_sessions
        .end(
            PowerHostKey::RepoHost(reserved_host_id),
            HoldOwner::target(run_id, agent_id, repo.id),
        )
        .await
    else {
        return;
    };
    if !woke || !repo.shutdown_after_backup || repo.repo_host_id != reserved_host_id {
        return;
    }
    shutdown_repo_host(
        ctx,
        repo,
        TargetIds {
            agent_id,
            repo_id: repo.id,
        },
        run_id,
        hostname,
    )
    .await;
}

/// Releases the hold a dependency check put on the repository host it shares
/// a machine with, and shuts that host down if this was the last run relying
/// on it, some run in the session woke it, and the host's own setting asks for
/// it.
///
/// A dependency is never shut down for its own sake. But while a backup needs
/// the share on it, the machine must not be shut down by a repository run that
/// happened to finish first - so the dependency check holds the host's session
/// like any repository target would, and whichever hold is released last
/// decides, exactly as two repository targets sharing a host do.
pub(crate) async fn release_repo_host_for_dependency(
    ctx: PowerCtx<'_>,
    repo_host_id: i64,
    site: RunEventSite<'_>,
) {
    release_dependency_hold(
        ctx,
        repo_host_id,
        HoldOwner::dependency(site.run_id, site.agent_id),
        Some(site.repo_id),
        site.hostname,
    )
    .await;
}

/// [`release_repo_host_for_dependency`] for a hold whose run's repository
/// may not be known - one read back after a restart, which records none for a
/// dependency. Without one, a shut-down is recorded on the timeline of the
/// repository its SSH login is borrowed from.
async fn release_dependency_hold(
    ctx: PowerCtx<'_>,
    repo_host_id: i64,
    owner: HoldOwner<'_>,
    event_repo_id: Option<i64>,
    hostname: &str,
) {
    let Some((woke, _)) = ctx
        .power_sessions
        .end(PowerHostKey::RepoHost(repo_host_id), owner)
        .await
    else {
        return;
    };
    if !woke {
        return;
    }
    // Shutting a repository host down takes an SSH login, which only its
    // repositories carry; any one of them will do.
    let repo = match crate::db::repo_hosts::list_repos_on_host(ctx.pool, repo_host_id).await {
        Ok(repos) => match repos.first() {
            Some(first) => crate::db::get_repo_by_id(ctx.pool, first.id).await.ok(),
            None => None,
        },
        Err(e) => {
            warn!(repo_host_id, error = %e, "failed to look up the repositories on a host");
            None
        }
    };
    let Some(repo) = repo else {
        return;
    };
    if !repo.shutdown_after_backup {
        return;
    }
    shutdown_repo_host(
        ctx,
        &repo,
        TargetIds {
            agent_id: owner.agent_id,
            repo_id: event_repo_id.unwrap_or(repo.id),
        },
        owner.run_id,
        hostname,
    )
    .await;
}

async fn shutdown_repo_host(
    ctx: PowerCtx<'_>,
    repo: &RepoRow,
    target_ids: TargetIds,
    run_id: &str,
    hostname: &str,
) {
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
    mod holds;

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
        assert!(tracker.end(key, TEST_HOLD).await.is_none());

        // Last participant ending: gets back the accumulated flags.
        assert_eq!(tracker.end(key, TEST_HOLD).await, Some((true, true)));
    }

    #[tokio::test]
    async fn power_session_tracker_end_without_begin_is_none() {
        let tracker = PowerSessionTracker::default();
        assert!(
            tracker
                .end(PowerHostKey::Agent(99), TEST_HOLD)
                .await
                .is_none()
        );
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

    /// Sets the power settings of the host `repo` lives on, and returns the
    /// repository as it now reads them.
    async fn set_repo_host_power(
        pool: &sqlx::PgPool,
        repo: RepoRow,
        power: crate::test_support::RepoPowerPatch<'_>,
    ) -> Result<RepoRow, crate::error::ApiError> {
        crate::test_support::update_repo_host_power(pool, repo.repo_host_id, power).await?;
        db::get_repo_by_id(pool, repo.id).await
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
            ScheduleWakeOverride::HostDefault,
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
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 1,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: false,
                    stop_agent_after_backup: false,
                    ssh_host: None,
                    ssh_port: 22,
                    agent_service_name: "assimilate-agent",
                },
            },
        )
        .await
        .unwrap();

        let registry = AgentRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        registry.register(agent.id, tx, false, None, None).await;
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
            ScheduleWakeOverride::HostDefault,
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
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 1,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: true,
                    stop_agent_after_backup: false,
                    ssh_host: Some(UNREACHABLE_HOST),
                    ssh_port: UNREACHABLE_PORT,
                    agent_service_name: "assimilate-agent",
                },
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
            ScheduleWakeOverride::HostDefault,
        )
        .await;

        assert!(
            outcome.woke,
            "the WOL packet send itself succeeds even though the agent never connects in this \
             test -- teardown must still credit (and later undo) this run's wake regardless of \
             how the reconnect wait resolves"
        );
        assert!(
            !outcome.started_agent,
            "the SSH host refuses the connection"
        );

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![RunEventType::ReachabilityCheck, RunEventType::WakeSent]
        );
        // Regression: the WakeSent message must not embed the MAC address --
        // list_run_events has no per-viewer permission gate, and this
        // message is broadcast unfiltered to every connected UI client, so
        // it must not leak what the settings response already redacts for
        // non-privileged viewers.
        let wake_sent_message = &events.get(1).unwrap().message;
        assert!(
            !wake_sent_message.contains("3C:97:0E:2B:9A:44"),
            "WakeSent message must not contain the MAC address: {wake_sent_message}"
        );
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
            ScheduleWakeOverride::HostDefault,
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

    /// A host that already answers SSH is not woken, and the run learns it
    /// answered - nothing is put on its timeline.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_repo_online_does_not_wake_a_host_that_already_answers(pool: sqlx::PgPool) {
        let _key_dir = crate::test_support::TestSshKeyDir::setup().await;
        let server = crate::test_support::spawn_test_ssh_server().await;
        let agent = test_agent(&pool).await;
        let repo = test_repo(&pool).await;
        db::repo_hosts::update_repo_host_address(
            &pool,
            repo.repo_host_id,
            "127.0.0.1",
            i32::from(server.port),
        )
        .await
        .unwrap();
        let repo = set_repo_host_power(
            &pool,
            repo,
            crate::test_support::RepoPowerPatch {
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
            ScheduleWakeOverride::HostDefault,
        )
        .await;
        server.shutdown().await;

        assert!(!outcome.woke);
        assert_eq!(outcome.reachable, Some(true));
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
        let repo = set_repo_host_power(
            &pool,
            test_repo(&pool).await,
            crate::test_support::RepoPowerPatch {
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
            ScheduleWakeOverride::HostDefault,
        )
        .await;

        assert!(
            outcome.woke,
            "the WOL packet send itself succeeds even though the host never answers SSH in this \
             test -- teardown must still credit (and later undo) this run's wake regardless of \
             how the reachability wait resolves"
        );

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![RunEventType::ReachabilityCheck, RunEventType::WakeSent]
        );
        // Same regression as ensure_agent_online's WakeSent message: must
        // not embed the MAC address.
        let wake_sent_message = &events.get(1).unwrap().message;
        assert!(
            !wake_sent_message.contains("9C:B6:D0:1A:44:7F"),
            "WakeSent message must not contain the MAC address: {wake_sent_message}"
        );
    }

    #[tokio::test]
    async fn teardown_agent_power_does_nothing_while_a_sibling_run_still_needs_the_host() {
        let sessions = PowerSessionTracker::default();
        let key = PowerHostKey::Agent(1);
        sessions.begin(key, true, false).await;
        sessions.begin(key, true, false).await; // a sibling schedule still running

        // Ending only this run's participation must not shut anything down
        // while the sibling is still relying on the host.
        assert!(sessions.end(key, TEST_HOLD).await.is_none());
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
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 180,
                    shutdown_after_backup: true,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: false,
                    stop_agent_after_backup: false,
                    ssh_host: Some(UNREACHABLE_HOST),
                    ssh_port: UNREACHABLE_PORT,
                    agent_service_name: "assimilate-agent",
                },
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
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(event_types, vec![RunEventType::ShutdownSent]);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_attempts_stop_when_this_run_started_the_agent(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: false,
                    wake_mac_address: None,
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 180,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: true,
                    stop_agent_after_backup: true,
                    ssh_host: Some(UNREACHABLE_HOST),
                    ssh_port: UNREACHABLE_PORT,
                    agent_service_name: "assimilate-agent",
                },
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
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(event_types, vec![RunEventType::AgentStopSent]);
    }

    /// Regression test: `start_agent_process` runs its command over SSH,
    /// which only ever succeeds against a host that's already reachable --
    /// so `started_agent` alone never means this run turned the host on,
    /// only that it kicked an already-running host's agent process back up.
    /// A host in that shape (already on, agent process needed a restart)
    /// must be left running even with `shutdown_after_backup` enabled --
    /// only `woke` earns a shutdown.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn teardown_agent_power_does_not_shut_down_when_this_run_only_started_the_agent(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 180,
                    shutdown_after_backup: true,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: true,
                    stop_agent_after_backup: false,
                    ssh_host: Some(UNREACHABLE_HOST),
                    ssh_port: UNREACHABLE_PORT,
                    agent_service_name: "assimilate-agent",
                },
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();
        // This run started the agent process over SSH but never sent a
        // wake -- the host was already on the whole time.
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

        // Neither branch fires: shutdown requires `woke`, and the
        // stop-agent branch requires `stop_agent_after_backup` (off here).
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        assert!(events.is_empty());
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
            .begin(PowerHostKey::RepoHost(repo.repo_host_id), false, false)
            .await;

        teardown_repo_power(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            repo.repo_host_id,
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
        let repo = set_repo_host_power(
            &pool,
            test_repo(&pool).await,
            crate::test_support::RepoPowerPatch {
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
            .begin(PowerHostKey::RepoHost(repo.repo_host_id), true, false)
            .await;

        teardown_repo_power(
            ctx(&pool, &registry, &bus, &sessions),
            &repo,
            repo.repo_host_id,
            agent.id,
            "run-1",
            "repo-host",
        )
        .await;

        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(event_types, vec![RunEventType::ShutdownSent]);
    }

    /// The whole point of the per-schedule override: a host with waking
    /// switched off is still woken by a schedule that asks for it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_wakes_a_host_whose_own_wake_is_off_when_the_schedule_enables_it(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: false,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 1,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: false,
                    stop_agent_after_backup: false,
                    ssh_host: None,
                    ssh_port: 22,
                    agent_service_name: "assimilate-agent",
                },
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
            ScheduleWakeOverride::Enabled,
        )
        .await;

        assert!(outcome.woke);
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![RunEventType::ReachabilityCheck, RunEventType::WakeSent]
        );
    }

    /// The other direction, and the guarantee that goes with it: `Disabled`
    /// overrides a host that wakes by default, and with nothing else to do
    /// for that host the call short-circuits exactly as it does for a host
    /// with power management switched off entirely.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_does_not_wake_a_host_when_the_schedule_disables_it(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 1,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: false,
                    stop_agent_after_backup: false,
                    ssh_host: None,
                    ssh_port: 22,
                    agent_service_name: "assimilate-agent",
                },
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
            ScheduleWakeOverride::Disabled,
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

    /// `Disabled` is about waking only: an agent process that this run would
    /// have started over SSH is still started, so the reachability check
    /// still runs and only the wake is missing from the timeline.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_still_starts_the_agent_when_the_schedule_disables_waking(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = crate::test_support::update_agent_power(
            &pool,
            test_agent(&pool).await.id,
            crate::test_support::AgentPowerPatch {
                wake: crate::db::machines::MachinePowerPatch {
                    wake_enabled: true,
                    wake_mac_address: Some("3C:97:0E:2B:9A:44"),
                    wake_broadcast_address: None,
                    wake_timeout_seconds: 1,
                    shutdown_after_backup: false,
                },
                process: crate::db::AgentProcessPatch {
                    start_agent_enabled: true,
                    stop_agent_after_backup: false,
                    ssh_host: Some(UNREACHABLE_HOST),
                    ssh_port: UNREACHABLE_PORT,
                    agent_service_name: "assimilate-agent",
                },
            },
        )
        .await
        .unwrap();
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
            ScheduleWakeOverride::Disabled,
        )
        .await;

        assert!(!outcome.woke);
        assert!(
            !outcome.started_agent,
            "the SSH host refuses the connection"
        );
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![RunEventType::ReachabilityCheck],
            "the start-agent path still ran (so the call did not short-circuit), and no wake was \
             attempted"
        );
    }

    /// A schedule can ask to wake a host that has no MAC address on file --
    /// something the host's own `wake_enabled` flag can never do, since it
    /// requires one. That has to say so on the run timeline rather than pass
    /// silently.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_agent_online_records_wake_unavailable_without_a_mac_address(
        pool: sqlx::PgPool,
    ) {
        let repo = test_repo(&pool).await;
        let agent = test_agent(&pool).await;
        let registry = AgentRegistry::new();
        let sessions = PowerSessionTracker::default();
        let bus = UiBroadcast::new();

        let outcome = ensure_agent_online(
            ctx(&pool, &registry, &bus, &sessions),
            &agent,
            repo.id,
            "run-1",
            ScheduleWakeOverride::Enabled,
        )
        .await;

        assert!(!outcome.woke);
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![
                RunEventType::ReachabilityCheck,
                RunEventType::WakeUnavailable
            ]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_repo_online_wakes_a_host_whose_own_wake_is_off_when_the_schedule_enables_it(
        pool: sqlx::PgPool,
    ) {
        let agent = test_agent(&pool).await;
        let repo = set_repo_host_power(
            &pool,
            test_repo(&pool).await,
            crate::test_support::RepoPowerPatch {
                wake_enabled: false,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
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
            ScheduleWakeOverride::Enabled,
        )
        .await;

        assert!(outcome.woke);
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![RunEventType::ReachabilityCheck, RunEventType::WakeSent]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn ensure_repo_online_does_not_wake_when_the_schedule_disables_it(pool: sqlx::PgPool) {
        let agent = test_agent(&pool).await;
        let repo = set_repo_host_power(
            &pool,
            test_repo(&pool).await,
            crate::test_support::RepoPowerPatch {
                wake_enabled: true,
                wake_mac_address: Some("3C:97:0E:2B:9A:44"),
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
            ScheduleWakeOverride::Disabled,
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
    async fn ensure_repo_online_records_wake_unavailable_without_a_mac_address(pool: sqlx::PgPool) {
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
            ScheduleWakeOverride::Enabled,
        )
        .await;

        assert!(!outcome.woke);
        let events = db::run_events::list_run_events(&pool, "run-1", agent.id, repo.id)
            .await
            .unwrap();
        let event_types: Vec<RunEventType> = events.iter().map(|e| e.event_type).collect();
        assert_eq!(
            event_types,
            vec![
                RunEventType::ReachabilityCheck,
                RunEventType::WakeUnavailable
            ]
        );
    }
}

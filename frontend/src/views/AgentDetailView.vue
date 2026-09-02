<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  listAgents,
  updateAgent,
  regenerateAgentToken,
  restartAgent as restartAgentApi,
  listAgentRepos,
  listAgentReports,
  createAgentHostnamePattern,
  cancelAgentBackup,
  deleteFailedReports,
  countFailedReports,
} from '../api/agents'
import { listSchedules, getScheduleHealth } from '../api/schedules'
import { getSystemVersion } from '../api/system'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { useElapsedClock } from '../composables/useElapsedTimer'
import { extractError } from '../utils/error'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useToast } from '../composables/useToast'
import { logger } from '../utils/logger'
import BaseSpinner from '../components/BaseSpinner.vue'
import MergeAgentDialog from '../components/MergeAgentDialog.vue'
import AgentDeployDialog from '../components/AgentDeployDialog.vue'
import SshKeyDeployPanel from '../components/SshKeyDeployPanel.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { ScheduleRow } from '../types/schedule'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { agentPowerPhase, type AgentPowerPhase } from '../utils/badge'
import { parseArchiveProgress } from '../utils/archiveProgress'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'
import { isSettingsSection, type SettingsSection } from '../utils/agentSettings'
import { domainParams } from '../utils/agent'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'
import AgentHeader from '../components/AgentHeader.vue'
import AgentOverviewTab, { type LiveBackup } from '../components/AgentOverviewTab.vue'
import AgentSchedulesTab from '../components/AgentSchedulesTab.vue'
import AgentBackupsTab, { type BackupFilter } from '../components/AgentBackupsTab.vue'
import AgentSettingsTab from '../components/AgentSettingsTab.vue'

/**
 * An agent's detail page: a persistent header, then four tabs.
 *
 * Settings is a tab rather than a separate route because every other detail
 * view in the app drives its sections off `route.query.tab`, and a route
 * would put a third button back in the header's action row - the row this
 * layout exists to shrink. Settings is self-contained either way, so
 * promoting it later is a router change and nothing else.
 */
type TabId = 'overview' | 'schedules' | 'backups' | 'settings'

function isTabId(value: unknown): value is TabId {
  return (
    value === 'overview' || value === 'schedules' || value === 'backups' || value === 'settings'
  )
}

const props = defineProps<{ hostname: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const activeTab = computed<TabId>({
  get() {
    return isTabId(route.query.tab) ? route.query.tab : 'overview'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const settingsSection = computed<SettingsSection>({
  get() {
    return isSettingsSection(route.query.section) ? route.query.section : 'identity'
  },
  set(val: SettingsSection) {
    router.replace({ query: { ...route.query, section: val } })
  },
})

const agent = ref<AgentRow | null>(null)
/**
 * Set only when `props.hostname` matches more than one agent and the
 * `domain` query param doesn't pick one out - the page shows a picker
 * instead of guessing which agent the caller meant.
 */
const ambiguousMatches = ref<AgentRow[]>([])
const routeDomain = computed<string | undefined>(() =>
  typeof route.query.domain === 'string' ? route.query.domain : undefined,
)
const repos = ref<Repo[]>([])
const schedules = ref<ScheduleRow[]>([])
const reports = ref<ReportRow[]>([])
const scheduleHealth = ref<ScheduleHealthEntry[]>([])
const { loading, error, run } = useAsyncAction()
const expandedReportId = ref<number | null>(null)

const agentSchedules = computed(() => {
  const hostname = agent.value?.hostname
  return hostname ? schedules.value.filter((s) => s.target_hostnames.includes(hostname)) : []
})

/**
 * Every tab carries its tally, including zero. On an imported host the empty
 * Schedules tab is the point: it says why there is nothing there and how to
 * change that, which a hidden tab cannot.
 */
const tabs = computed<TabOption<TabId>[]>(() => [
  { id: 'overview', label: 'Overview' },
  { id: 'schedules', label: 'Schedules', count: agentSchedules.value.length },
  { id: 'backups', label: 'Backups', count: reports.value.length },
  { id: 'settings', label: 'Settings' },
])

// Backup filter / sort
function isRunStatusFilter(value: unknown): value is 'success' | 'warning' | 'failed' {
  return value === 'success' || value === 'warning' || value === 'failed'
}
const filterStatus = ref<BackupFilter>(
  isRunStatusFilter(route.query.status) ? route.query.status : 'all',
)
const sortAscending = ref(false)

const highlightedArchiveName = computed(() => {
  const a = route.query.archive
  return typeof a === 'string' ? a : undefined
})

const pinnedStatus = computed(() => {
  const s = route.query.status
  return isRunStatusFilter(s) ? s : undefined
})
const pinnedReportId = ref<number | null>(null)
const pinnedForStatus = ref<typeof pinnedStatus.value>(undefined)

function isOverdueQuery(value: unknown): value is 'overdue' {
  return value === 'overdue'
}
const overdueHighlighted = computed(() => isOverdueQuery(route.query.health))

const isAdmin = computed(() => authStore.isAdmin)
const isImported = computed(() => agent.value?.is_imported ?? false)

/** Merge back the fields the defaults panel just wrote, leaving the rest. */
function onDefaultsSaved(updated: AgentRow): void {
  agent.value = agent.value ? { ...agent.value, ...updated } : updated
}

// Merge dialog
const allAgents = ref<AgentRow[]>([])
const showMergeDialog = ref(false)

useEscapeKey(showMergeDialog, () => {
  showMergeDialog.value = false
})

// Token regen
const showTokenDialog = ref(false)
const regenToken = ref<string | null>(null)
const regenLoading = ref(false)
const regenError = ref<string | null>(null)
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

// Restart agent
const restartLoading = ref(false)
const restartError = ref<string | null>(null)

// Deploy/Upgrade agent
const availableAgentVersion = ref<string | null>(null)
const serverCommitCount = ref<number | null>(null)
const showDeployDialog = ref(false)
const deployForceRedeploy = ref(false)

// Deploy SSH key
const showDeploySshKey = ref(false)

useEscapeKey(showDeploySshKey, () => {
  showDeploySshKey.value = false
})

function deployButtonLabel(): string | null {
  if (!agent.value) return null
  if (!agent.value.agent_version) return 'Deploy'
  const commitCount = agent.value.agent_commit_count ?? null
  if (serverCommitCount.value !== null && commitCount !== null) {
    return commitCount >= serverCommitCount.value ? null : 'Upgrade'
  }
  if (!availableAgentVersion.value) return null
  return agent.value.agent_version === availableAgentVersion.value ? null : 'Upgrade'
}

/** Suppressed where it cannot be acted on, so the accented slot stays honest. */
const headerDeployLabel = computed(() =>
  !isImported.value && authStore.canUpgradeAgent ? deployButtonLabel() : null,
)

/** Once an agent has been deployed at least once, it can always be redeployed. */
const canRedeploy = computed(
  () => !isImported.value && authStore.canUpgradeAgent && !!agent.value?.agent_version,
)

function openRedeployDialog(): void {
  deployForceRedeploy.value = true
  showDeployDialog.value = true
}

// Hostname & display name editing. A dialog rather than the inline panel this
// used to be: the panel appeared mid-page and pushed six cards down, while
// every other form in the app opens through BaseModal.
const editingIdentity = ref(false)
const identityHostname = ref('')
const identityDomain = ref('')
const identityDisplayName = ref('')
const identitySaving = ref(false)
const identityError = ref<string | null>(null)

function startEditIdentity(): void {
  if (!agent.value) return
  identityHostname.value = agent.value.hostname
  identityDomain.value = agent.value.domain ?? ''
  identityDisplayName.value = agent.value.display_name ?? ''
  identityError.value = null
  editingIdentity.value = true
}

function cancelEditIdentity(): void {
  editingIdentity.value = false
}

async function saveIdentity(): Promise<void> {
  if (!agent.value) return
  identitySaving.value = true
  identityError.value = null
  try {
    const oldHostname = agent.value.hostname
    const newHostname = identityHostname.value.trim()
    const newDomain = identityDomain.value.trim() || null
    const hostnameChanged = newHostname !== oldHostname && newHostname.length > 0
    const domainChanged = newDomain !== (agent.value.domain ?? null)
    const updated = await updateAgent(
      oldHostname,
      {
        hostname: hostnameChanged ? newHostname : undefined,
        display_name: identityDisplayName.value.trim() || null,
        domain: newDomain,
        default_backup_paths: agent.value.default_backup_paths,
        default_exclude_patterns: agent.value.default_exclude_patterns,
        default_pre_backup_commands: agent.value.default_pre_backup_commands,
        default_post_backup_commands: agent.value.default_post_backup_commands,
        default_file_change_patterns_raw: agent.value.default_file_change_patterns_raw,
      },
      agent.value.domain,
    )
    if (hostnameChanged) {
      pendingAliasOldHostname.value = oldHostname
      pendingAliasNewHostname.value = newHostname
      showAliasConfirm.value = true
      router.replace({ path: `/agents/${newHostname}`, query: domainParams(newDomain) })
    } else if (domainChanged) {
      router.replace({ path: `/agents/${oldHostname}`, query: domainParams(newDomain) })
    }
    agent.value = { ...agent.value, ...updated }
    editingIdentity.value = false
  } catch (e: unknown) {
    identityError.value = extractError(e)
  } finally {
    identitySaving.value = false
  }
}

// Hostname alias confirmation
const settingsTab = ref<InstanceType<typeof AgentSettingsTab> | null>(null)
const showAliasConfirm = ref(false)
const pendingAliasOldHostname = ref('')
const pendingAliasNewHostname = ref('')

useEscapeKey(showAliasConfirm, () => {
  showAliasConfirm.value = false
})

async function confirmAddAlias(): Promise<void> {
  await createAgentHostnamePattern(
    pendingAliasNewHostname.value,
    pendingAliasOldHostname.value,
    agent.value?.domain,
  )
  // Only mounted while the Settings tab is showing its aliases section; when
  // it is not, the list reloads from scratch the next time it is opened.
  await settingsTab.value?.reloadAliases(pendingAliasNewHostname.value)
  showAliasConfirm.value = false
}

function declineAlias(): void {
  showAliasConfirm.value = false
}

async function adoptHost(): Promise<void> {
  if (!agent.value) return
  try {
    const cleanDisplayName =
      agent.value.display_name?.replace(/\s*\(imported\)$/, '').trim() || null
    await updateAgent(
      agent.value.hostname,
      { display_name: cleanDisplayName, domain: agent.value.domain },
      agent.value.domain,
    )
    const res = await regenerateAgentToken(agent.value.hostname, agent.value.domain)
    agent.value = {
      ...agent.value,
      ...res.agent,
      id: Number(res.agent.id),
      is_imported: false,
      display_name: cleanDisplayName,
    }
    regenToken.value = res.token
    tokenCopied.value = false
    showTokenDialog.value = true
  } catch (e: unknown) {
    logger.error('Failed to adopt host', e)
  }
}

function openMergeDialog(): void {
  showMergeDialog.value = true
}

function onMerged(): void {
  showMergeDialog.value = false
  router.push('/agents')
}

useEscapeKey(showTokenDialog, () => {
  showTokenDialog.value = false
})

function openReport(r: ReportRow): void {
  const query: Record<string, string> = { tab: 'archives' }
  if (r.archive_name) {
    query.archive = r.archive_name
  }
  router.push({ path: `/repos/${r.repo_id}`, query })
}

function toggleReport(r: ReportRow): void {
  expandedReportId.value = expandedReportId.value === r.id ? null : r.id
}

// SSH-key deploy, merge and redeploy all operate on the currently-resolved
// `agent` object and are gated `v-if="agent"` (merge/redeploy additionally
// on their own `show*` flag). Once the host can no longer be uniquely
// resolved - ambiguous, or gone entirely - keeping one of these open would
// mean it silently keeps acting on a stale/orphaned agent, so close them
// alongside `agent`/`ambiguousMatches` rather than relying on `v-if="agent"`
// alone (which only helps when `agent.value` is actually nulled).
function closeAgentScopedModals(): void {
  showDeploySshKey.value = false
  showMergeDialog.value = false
  showDeployDialog.value = false
}

// A background refresh (see `refreshAgent` below) must leave the last-good
// `agent` on screen if this throws or the host briefly drops out of the
// list, rather than blanking the page - so `agent`/`ambiguousMatches` are
// only written once a match (or a genuine ambiguity) is confirmed, never on
// the "not found" path.
async function fetchAgent(): Promise<void> {
  const agentRows = await listAgents()
  allAgents.value = agentRows
  const matches = agentRows.filter((m) => m.hostname === props.hostname)
  const domain = routeDomain.value
  let resolved: AgentRow | null
  if (domain !== undefined) {
    resolved = matches.find((m) => (m.domain ?? '') === domain) ?? null
  } else if (matches.length > 1) {
    // The breadcrumb reads `agent.value` directly, outside the template's
    // v-else-if chain - null it so a background refresh that turns a
    // previously-resolved single match ambiguous (e.g. a duplicate hostname
    // appearing) doesn't leave a stale domain hint next to the hostname
    // while the picker below asks which one is meant.
    agent.value = null
    ambiguousMatches.value = matches
    closeAgentScopedModals()
    return
  } else {
    resolved = matches[0] ?? null
  }
  if (!resolved) {
    closeAgentScopedModals()
    throw new Error(`Agent "${props.hostname}" not found`)
  }
  ambiguousMatches.value = []
  agent.value = resolved
  await loadTabData()
}

async function loadAgent(): Promise<void> {
  // Cleared up front (rather than left to `fetchAgent`) so a hostname/domain
  // change doesn't briefly show the *previous* host's stale disambiguation
  // picker while this fresh load is still in flight behind the spinner.
  ambiguousMatches.value = []
  await run(fetchAgent)
}

/**
 * Re-fetches agent data in the background (WS-driven updates, reconnects)
 * without toggling `loading` - that would unmount and remount the whole
 * settings tab, discarding any in-progress edit in it. See `refreshRepo` in
 * RepoDetailView.vue for the same pattern.
 */
async function refreshAgent(): Promise<void> {
  try {
    await fetchAgent()
  } catch (e: unknown) {
    logger.error('background agent refresh failed', e)
  }
}

async function loadTabData(): Promise<void> {
  if (!agent.value) return
  const hostname = agent.value.hostname
  // Fetched independently of the Promise.all below: it backs a menu badge,
  // not the page itself, so a failure here (e.g. a not-yet-registered host)
  // must not take down the rest of the tab data with it.
  countFailedReports(hostname, agent.value.domain)
    .then((count) => {
      failedReportCount.value = count
    })
    .catch((e: unknown) => logger.error('countFailedReports failed', e))
  try {
    const [repoRows, scheduleRows, reportRows, healthRows] = await Promise.all([
      listAgentRepos(hostname, agent.value.domain),
      listSchedules(),
      listAgentReports(hostname, undefined, agent.value.domain),
      getScheduleHealth(),
    ])
    repos.value = repoRows
    schedules.value = scheduleRows
    reports.value = reportRows
    scheduleHealth.value = healthRows.filter((h) => h.hostname === hostname)
    const runningReports = reportRows.filter((r) => {
      const status = normalizeBackupStatus(r.status)
      return status === 'pending' || status === 'started'
    })
    runningReports.forEach((r) => {
      if (!r.repo_name || activeBackups.value.some((b) => b.targetName === r.repo_name)) return
      const startedAt = new Date(r.started_at).getTime()
      activeBackups.value = [
        ...activeBackups.value,
        {
          targetName: r.repo_name,
          repoId: r.repo_id,
          archiveName: r.archive_name,
          startedAt,
          progress: null,
        },
      ]
    })
  } catch (e: unknown) {
    logger.error('loadTabData failed', e)
  }
}

/**
 * Arriving from a status link (`?status=failed`) opens the newest matching
 * run on the Backups tab, where the rows live.
 */
watch(
  [reports, pinnedStatus],
  ([, status]) => {
    if (!status) {
      if (pinnedReportId.value !== null && expandedReportId.value === pinnedReportId.value) {
        expandedReportId.value = null
      }
      pinnedForStatus.value = undefined
      pinnedReportId.value = null
      return
    }
    if (pinnedForStatus.value === status && pinnedReportId.value !== null) return
    const match = [...reports.value]
      .filter((r) => normalizeBackupStatus(r.status) === status)
      .sort((a, b) => new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime())[0]
    if (!match) return
    pinnedForStatus.value = status
    pinnedReportId.value = match.id
    expandedReportId.value = match.id
    nextTick(() => {
      document
        .getElementById(`report-${match.id}`)
        ?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    })
  },
  { immediate: true },
)

watch(
  [reports, highlightedArchiveName],
  ([, archiveName]) => {
    if (!archiveName) return
    const report = reports.value.find((r) => r.archive_name === archiveName)
    if (!report) return
    expandedReportId.value = report.id
    nextTick(() => {
      document
        .getElementById(`report-${report.id}`)
        ?.scrollIntoView({ behavior: 'smooth', block: 'center' })
    })
  },
  { immediate: true },
)

function repoNameForSchedule(s: ScheduleRow): string {
  return (
    repos.value.find((r) => r.id === s.repo_id)?.name ??
    (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
  )
}

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
}

function goToActivityLog(): void {
  router.push(`/activity?category=backup&hostname=${encodeURIComponent(props.hostname)}`)
}

// Token regeneration
async function regenerateToken(): Promise<void> {
  regenLoading.value = true
  regenError.value = null
  regenToken.value = null
  tokenCopied.value = false
  try {
    const res = await regenerateAgentToken(props.hostname, agent.value?.domain)
    regenToken.value = res.token
    agent.value = res.agent
    showTokenDialog.value = true
  } catch (e: unknown) {
    regenError.value = extractError(e)
    showTokenDialog.value = true
  } finally {
    regenLoading.value = false
  }
}

async function restartAgent(): Promise<void> {
  restartLoading.value = true
  restartError.value = null
  try {
    await restartAgentApi(props.hostname, agent.value?.domain)
  } catch (e: unknown) {
    restartError.value = extractError(e)
  } finally {
    restartLoading.value = false
  }
}

const { success: toastSuccess, error: toastError } = useToast()
const cancellingRepoIds = ref<number[]>([])

async function cancelBackupInProgress(repoId: number): Promise<void> {
  cancellingRepoIds.value = [...cancellingRepoIds.value, repoId]
  try {
    await cancelAgentBackup(props.hostname, repoId, agent.value?.domain)
    toastSuccess('Cancel request sent.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    cancellingRepoIds.value = cancellingRepoIds.value.filter((id) => id !== repoId)
  }
}

// Clean up failed backup reports (overflow menu). Counted separately from
// `reports.value` (which the report list's own `limit` bounds) so the menu
// label and confirmation dialog never understate how many records the
// unbounded delete is actually about to remove.
const failedReportCount = ref(0)
const showCleanFailedDialog = ref(false)
const cleaningFailedReports = ref(false)

async function confirmCleanFailedReports(): Promise<void> {
  if (!agent.value) return
  cleaningFailedReports.value = true
  try {
    const result = await deleteFailedReports(agent.value.hostname, agent.value.domain)
    showCleanFailedDialog.value = false
    toastSuccess(
      result.deleted === 1
        ? 'Deleted 1 failed backup report.'
        : `Deleted ${result.deleted} failed backup reports.`,
    )
    await loadAgent()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    cleaningFailedReports.value = false
  }
}

watch([() => props.hostname, routeDomain], () => {
  powerPhase.value = null
  loadAgent()
})
onMounted(() => {
  loadAgent()
  getSystemVersion()
    .then((res) => {
      availableAgentVersion.value = res.agent_version
      serverCommitCount.value = res.server_commit_count
    })
    .catch(logger.error)
})

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('DataChanged', () => refreshAgent())
// Known limitation: this and the `RunEvent` handler below match on bare
// hostname only, because neither payload carries a `domain`/`agent_id`.
// Two agents sharing a hostname across different domains can therefore
// cross-contaminate each other's transient `powerPhase` badge here. Low
// impact and self-healing (see POWER_PHASE_TIMEOUT_MS below) - closing it
// for real needs `domain`/`agent_id` added to the WS payloads.
onMessage('AgentConnected', (payload) => {
  if (payload.hostname !== props.hostname) return
  powerPhase.value = null
  refreshAgent()
})
onMessage('AgentDisconnected', () => refreshAgent())

/**
 * The transient phase `AgentHeader` shows in place of Online/Offline while
 * this host is being reached or powered down around a backup - derived
 * entirely from the run's own event stream rather than a dedicated "current
 * phase" field, matching how the run detail timeline is built. Only events
 * about this agent's own host move the needle: a `repository`-target event
 * belongs to the same run but says nothing about this page's host.
 */
const powerPhase = ref<AgentPowerPhase | null>(null)

// Safety net for a phase that never resolves - e.g. a shutdown/agent-stop
// attempt whose SSH command fails partway (the host never goes offline, the
// agent never disconnects, so neither a `host_offline` RunEvent nor an
// `AgentConnected` message ever arrives to clear it). Generous relative to
// the server's own wake/shutdown timeouts so it never fires during a normal
// wait, just as a last resort against a badge stuck forever.
const POWER_PHASE_TIMEOUT_MS = 5 * 60 * 1000
let powerPhaseTimeout: ReturnType<typeof setTimeout> | undefined
watch(powerPhase, (phase) => {
  clearTimeout(powerPhaseTimeout)
  if (phase) {
    powerPhaseTimeout = setTimeout(() => {
      powerPhase.value = null
    }, POWER_PHASE_TIMEOUT_MS)
  }
})
onUnmounted(() => clearTimeout(powerPhaseTimeout))

onMessage('RunEvent', (payload) => {
  if (payload.hostname !== props.hostname || payload.target !== 'source') return
  powerPhase.value = agentPowerPhase(payload.event_type)
})

interface ArchiveProgressData {
  nfiles: number
  originalSize: number
  currentPath: string
}

interface ActiveBackup {
  targetName: string
  repoId: number | null
  archiveName: string | null
  startedAt: number
  progress: ArchiveProgressData | null
}

const activeBackups = ref<ActiveBackup[]>([])
const hasActiveBackups = computed(() => activeBackups.value.length > 0)
const { now } = useElapsedClock(hasActiveBackups)

/** The clock lives here, so the Overview tab is handed plain numbers. */
const liveBackups = computed<LiveBackup[]>(() =>
  activeBackups.value.map((b) => ({
    targetName: b.targetName,
    repoId: b.repoId,
    archiveName: b.archiveName,
    elapsedSecs: Math.max(0, Math.floor((now.value - b.startedAt) / 1000)),
    progress: b.progress,
  })),
)

onMessage('BackupStarted', (payload) => {
  if (payload.hostname !== props.hostname) return
  if (activeBackups.value.some((b) => b.targetName === payload.target_name)) return
  activeBackups.value = [
    ...activeBackups.value,
    {
      targetName: payload.target_name,
      repoId: repos.value.find((r) => r.name === payload.target_name)?.id ?? null,
      archiveName: payload.archive_name ?? null,
      startedAt: Date.parse(payload.started_at),
      progress: null,
    },
  ]
})

onMessage('BackupCompleted', (payload) => {
  if (payload.hostname === props.hostname) {
    activeBackups.value = activeBackups.value.filter((b) => b.targetName !== payload.target_name)
  }
  refreshAgent()
})

onMessage('BackupLog', (payload) => {
  if (payload.hostname !== props.hostname) return
  const targetName = repos.value.find((r) => r.id === payload.repo_id)?.name
  if (!targetName) return
  const backup = activeBackups.value.find((b) => b.targetName === targetName)
  if (!backup) return
  const progress = parseArchiveProgress(payload.line)
  if (progress !== null) {
    backup.progress = {
      nfiles: progress.nfiles,
      originalSize: progress.original_size,
      currentPath: progress.path ?? '',
    }
  }
})

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    refreshAgent()
  }
})
</script>

<template>
  <div class="host-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/agents"
        class="crumb-link"
      >
        Agents
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ props.hostname }}</span>
      <span
        v-if="agent?.domain"
        class="muted"
        >({{ agent.domain }})</span
      >
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div v-else-if="ambiguousMatches.length > 0">
      <p class="pane-lede">
        More than one host is named <strong>{{ props.hostname }}</strong
        >. Choose which one:
      </p>
      <div class="card-grid">
        <div
          v-for="m in ambiguousMatches"
          :key="m.id"
          class="entity-card"
          @click="router.push({ path: `/agents/${props.hostname}`, query: domainParams(m.domain) })"
        >
          <div class="card-top">
            <div class="card-info">
              <span class="card-name">{{ m.domain ?? 'No domain set' }}</span>
              <span
                v-if="m.display_name"
                class="card-display"
                >{{ m.display_name }}</span
              >
            </div>
          </div>
        </div>
      </div>
    </div>
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="agent">
      <AgentHeader
        :agent="agent"
        :power-phase="powerPhase"
        :deploy-label="headerDeployLabel"
        :can-redeploy="canRedeploy"
        :restart-loading="restartLoading"
        :regen-loading="regenLoading"
        :restart-error="restartError"
        :is-admin="isAdmin"
        :failed-report-count="failedReportCount"
        @adopt="adoptHost"
        @merge="openMergeDialog"
        @deploy="
          () => {
            deployForceRedeploy = false
            showDeployDialog = true
          }
        "
        @redeploy="openRedeployDialog"
        @activity-log="goToActivityLog"
        @edit-identity="startEditIdentity"
        @deploy-ssh-key="showDeploySshKey = true"
        @regenerate-token="regenerateToken"
        @restart="restartAgent"
        @clean-failed-reports="showCleanFailedDialog = true"
      />

      <BaseTabs
        v-model="activeTab"
        :tabs="tabs"
        label="Agent sections"
      />

      <div class="tab-content fade-in">
        <AgentOverviewTab
          v-if="activeTab === 'overview'"
          :agent="agent"
          :repos="repos"
          :schedules="agentSchedules"
          :health="scheduleHealth"
          :reports="reports"
          :live-backups="liveBackups"
          :cancelling-repo-ids="cancellingRepoIds"
          :repo-name-for="repoNameForSchedule"
          @open-schedule="navigateToSchedule"
          @open-report="openReport"
          @show-tab="activeTab = $event"
          @cancel-backup="cancelBackupInProgress"
        />

        <AgentSchedulesTab
          v-else-if="activeTab === 'schedules'"
          :agent="agent"
          :schedules="agentSchedules"
          :health="scheduleHealth"
          :highlight-overdue="overdueHighlighted"
          :repo-name-for="repoNameForSchedule"
          @open="navigateToSchedule"
        />

        <AgentBackupsTab
          v-else-if="activeTab === 'backups'"
          v-model:filter="filterStatus"
          v-model:sort-ascending="sortAscending"
          :reports="reports"
          :expanded-report-id="expandedReportId"
          :highlighted-archive-name="highlightedArchiveName"
          :pinned-report-id="pinnedReportId"
          @toggle="toggleReport"
          @open="openReport"
        />

        <AgentSettingsTab
          v-else-if="activeTab === 'settings'"
          ref="settingsTab"
          v-model:section="settingsSection"
          :agent="agent"
          :is-admin="isAdmin"
          :regen-loading="regenLoading"
          @edit-identity="startEditIdentity"
          @regenerate-token="regenerateToken"
          @saved="onDefaultsSaved"
        />
      </div>
    </template>

    <!-- Edit Identity -->
    <BaseModal
      :open="editingIdentity"
      title="Edit agent identity"
      @close="cancelEditIdentity"
    >
      <div class="field">
        <label
          class="field-label"
          for="identity-hostname"
          >Hostname</label
        >
        <input
          id="identity-hostname"
          v-model="identityHostname"
          class="input"
          placeholder="hostname"
          @keyup.enter="saveIdentity"
        />
      </div>
      <div class="field">
        <label
          class="field-label"
          for="identity-domain"
          >Domain</label
        >
        <input
          id="identity-domain"
          v-model="identityDomain"
          class="input"
          placeholder="Optional, e.g. lab.example.com"
          @keyup.enter="saveIdentity"
        />
        <span class="field-hint">Only needed if another host already uses this hostname.</span>
      </div>
      <div class="field">
        <label
          class="field-label"
          for="identity-display-name"
          >Display name</label
        >
        <input
          id="identity-display-name"
          v-model="identityDisplayName"
          class="input"
          placeholder="Optional friendly name"
          @keyup.enter="saveIdentity"
        />
      </div>
      <div
        v-if="identityError"
        class="form-error"
      >
        {{ identityError }}
      </div>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="cancelEditIdentity"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="identitySaving"
          @click="saveIdentity"
        >
          {{ identitySaving ? 'Saving...' : 'Save' }}
        </button>
      </template>
    </BaseModal>

    <!-- Deploy SSH Key -->
    <BaseModal
      v-if="agent"
      :open="showDeploySshKey"
      title="Deploy SSH key"
      @close="showDeploySshKey = false"
    >
      <SshKeyDeployPanel
        :ssh-host="agent.hostname"
        show-credentials
      />

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showDeploySshKey = false"
        >
          Close
        </button>
      </template>
    </BaseModal>

    <!-- Clean up failed backups -->
    <BaseModal
      :open="showCleanFailedDialog"
      title="Clean up failed backups"
      @close="showCleanFailedDialog = false"
    >
      <p>
        Permanently delete <strong>{{ failedReportCount }}</strong>
        {{ failedReportCount === 1 ? 'failed backup report' : 'failed backup reports' }} for this
        agent? Only failed runs that produced no archive are removed — nothing on disk is touched.
        This action cannot be undone.
      </p>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showCleanFailedDialog = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-danger"
          :disabled="cleaningFailedReports"
          @click="confirmCleanFailedReports"
        >
          {{ cleaningFailedReports ? 'Deleting...' : 'Delete failed reports' }}
        </button>
      </template>
    </BaseModal>

    <!-- Token Dialog -->
    <BaseModal
      :open="showTokenDialog"
      :title="regenToken ? 'New Token Generated' : 'Error'"
      @close="showTokenDialog = false"
    >
      <template v-if="regenToken">
        <p class="token-warning">Copy this token now. It will not be shown again.</p>
        <div class="token-box">
          <code class="token-text">{{ regenToken }}</code>
          <button
            class="btn btn-sm btn-ghost"
            @click="copyToClipboard(regenToken ?? '')"
          >
            {{ tokenCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </template>
      <div
        v-else-if="regenError"
        class="form-error"
      >
        {{ regenError }}
      </div>

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showTokenDialog = false"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <!-- Hostname Alias Confirmation Dialog -->
    <BaseModal
      :open="showAliasConfirm"
      title="Add hostname pattern?"
      @close="declineAlias"
    >
      <p>
        Hostname changed from <strong>{{ pendingAliasOldHostname }}</strong> to
        <strong>{{ pendingAliasNewHostname }}</strong
        >.
      </p>
      <p>
        Add <code>{{ pendingAliasOldHostname }}</code> as an alternative hostname pattern so
        existing archives still match?
      </p>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="declineAlias"
        >
          No
        </button>
        <button
          class="btn btn-primary"
          @click="confirmAddAlias"
        >
          Add pattern
        </button>
      </template>
    </BaseModal>

    <!-- Merge Agent Dialog -->
    <MergeAgentDialog
      v-if="showMergeDialog && agent"
      :source="agent"
      :all-agents="allAgents"
      @merged="onMerged"
      @cancel="showMergeDialog = false"
    />

    <!-- Deploy Agent Dialog -->
    <AgentDeployDialog
      v-if="showDeployDialog && agent"
      :hostname="agent.hostname"
      :domain="agent.domain"
      :agent-version="agent.agent_version ?? null"
      :available-version="availableAgentVersion"
      :last-ssh-user="agent.last_ssh_user"
      :force-redeploy="deployForceRedeploy"
      @close="
        () => {
          showDeployDialog = false
          deployForceRedeploy = false
        }
      "
      @deployed="loadAgent"
    />
  </div>
</template>

<style scoped>
.host-detail {
  max-width: 1100px;
}

.state-error {
  color: var(--danger);
}
</style>

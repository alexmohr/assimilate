<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { listAgents, listAgentTags, createAgent } from '../api/agents'
import { listTags } from '../api/tags'
import { getDashboardOverview, getScheduleCounts } from '../api/stats'
import { getScheduleHealth, listSchedules } from '../api/schedules'
import { getSystemVersion } from '../api/system'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useWebSocket } from '../composables/useWebSocket'
import { useClipboard } from '../composables/useClipboard'
import { useListSort } from '../composables/useListSort'
import {
  usePersistedRef,
  usePersistedBoolean,
  useQueryOverride,
} from '../composables/usePersistedRef'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { domainParams } from '../utils/agent'
import { ChevronRight, Plus, SlidersHorizontal, Server } from '@lucide/vue'
import AgentChannelKey from '../components/AgentChannelKey.vue'
import BaseSegmented, { type SegmentedOption } from '../components/BaseSegmented.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import PaneRow from '../components/PaneRow.vue'
import SortControls from '../components/SortControls.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
import type { DashboardOverview } from '../types/dashboard'
import type { AgentRow } from '../types/agent'
import type { TagRow } from '../types/tag'
import type { AgentTagEntryResponse, HealthSummaryResponse } from '../types/generated'
import BaseModal from '../components/BaseModal.vue'

interface AgentHealth {
  failed: number
  overdue: number
  warning: number
  total: number
  last_error_message: string | null
  /** Most recent completed backup across this host's schedules. */
  mostRecentBackupAt: string | null
}

type SortField = 'hostname' | 'status' | 'last_seen' | 'version'

const SORT_OPTIONS: readonly { field: SortField; label: string }[] = [
  { field: 'hostname', label: 'Name' },
  { field: 'status', label: 'Status' },
  { field: 'last_seen', label: 'Last seen' },
  { field: 'version', label: 'Version' },
]
/**
 * The counted pills above the list. `attention` is every agent with an issue
 * chip (failed, overdue, nothing scheduled); `online` and `offline` are the
 * connection alone, so a dashboard link such as `?status=offline` still lists
 * every unreachable host, whatever its backups say.
 */
type FilterStatus = 'all' | 'attention' | 'online' | 'offline'
type CoverageFilter = 'all' | 'protected' | 'unassigned' | 'never-succeeded' | 'disabled-only'
/** What the list is grouped by: how each agent is doing, or what it runs. */
type GroupMode = 'status' | 'version'

function isFilterStatus(value: string): value is FilterStatus {
  return value === 'all' || value === 'attention' || value === 'online' || value === 'offline'
}

function isGroupMode(value: string): value is GroupMode {
  return value === 'status' || value === 'version'
}

const GROUP_OPTIONS: readonly SegmentedOption<GroupMode>[] = [
  { value: 'status', label: 'Status' },
  { value: 'version', label: 'Version' },
]

/** The colour of a row's or a pill's status dot; `muted` is the plain grey dot. */
type DotTone = 'danger' | 'info' | 'warning' | 'success' | 'muted'

const DOT_CLASS: Record<DotTone, string | null> = {
  danger: 'status-dot--danger',
  info: 'status-dot--info',
  warning: 'status-dot--warning',
  success: 'status-dot--success',
  muted: null,
}

const STATUS_PILLS: readonly { value: FilterStatus; label: string; dot: DotTone | null }[] = [
  { value: 'all', label: 'All', dot: null },
  { value: 'attention', label: 'Needs attention', dot: 'warning' },
  { value: 'online', label: 'Online', dot: 'success' },
  { value: 'offline', label: 'Offline', dot: 'muted' },
]

/**
 * How many offline agents the Offline group shows before "Show N more". An
 * offline host with nothing due is the least interesting row on the page, and
 * a fleet with a lot of them would otherwise push everything else down.
 */
const OFFLINE_PREVIEW = 5

function isCoverageFilter(value: string): value is CoverageFilter {
  return (
    value === 'all' ||
    value === 'protected' ||
    value === 'unassigned' ||
    value === 'never-succeeded' ||
    value === 'disabled-only'
  )
}

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)
const agents = ref<AgentRow[]>([])
const showHidden = usePersistedBoolean('assimilate-agents-show-hidden', false)
const machineScheduleCount = ref<Record<number, number>>({})
// Whether `machineScheduleCount` reflects an answer from the server. A failed
// request leaves it empty, and an empty map must not read as "no host has any
// schedule".
const scheduleCountsKnown = ref(false)
const fleetScheduleCount = ref(0)
const healthByHost = ref<Record<string, AgentHealth>>({})
const loading = ref(false)
const error = ref<string | null>(null)

// Every toolbar choice below is persisted under its own `assimilate-agents-*`
// key and restored on the next visit - see the identical note in
// SchedulesView.vue. `filterStatus` and `filterCoverage` also take a query
// override, since a link such as `?coverage=unassigned` from the dashboard is
// an explicit choice made just now and should win over whatever was last
// persisted.
const {
  field: sortField,
  direction: sortDir,
  toggle: toggleSort,
  sign: sortSign,
} = useListSort<SortField>('hostname', 'asc', {
  key: 'assimilate-agents-sort',
  fields: SORT_OPTIONS.map((option) => option.field),
})
const filterStatus = usePersistedRef<FilterStatus>(
  'assimilate-agents-filter-status',
  'all',
  isFilterStatus,
  route.query.status as string | null | undefined,
)
const filterText = ref('')
// Not persisted, unlike the filters around it: usePersistedRef only
// validates a single string value against a type predicate, and a tag
// selection is a set of numeric IDs - persisting it would need a different
// shape (and its own staleness handling for a deleted tag), not a one-line
// addition here.
const filterTagIds = ref<number[]>([])
const filterCoverage = usePersistedRef<CoverageFilter>(
  'assimilate-agents-filter-coverage',
  'all',
  isCoverageFilter,
  route.query.coverage as string | null | undefined,
)
const coverageHostIds = ref<Record<Exclude<CoverageFilter, 'all'>, Set<number>>>({
  protected: new Set(),
  unassigned: new Set(),
  'never-succeeded': new Set(),
  'disabled-only': new Set(),
})
const showMobileFilters = ref(false)
/** Filters set in the filters card, so its button says when it is narrowing the list. */
const activeFilterCount = computed(
  () =>
    (filterCoverage.value !== 'all' ? 1 : 0) +
    filterTagIds.value.length +
    (showHidden.value ? 1 : 0),
)

const allAgentTags = ref<TagRow[]>([])
const agentTagsMap = ref<Record<number, { name: string; color: string }[]>>({})

const groupMode = usePersistedRef<GroupMode>('assimilate-agents-group', 'status', isGroupMode)
const showAllOffline = ref(false)

function matchesStatus(agent: AgentRow, status: FilterStatus): boolean {
  switch (status) {
    case 'all':
      return true
    case 'attention':
      return needsAttention(agent)
    case 'online':
      return isOnline(agent)
    case 'offline':
      return !isOnline(agent)
  }
}

/**
 * Every filter but the status one. The pills count this list, so each one
 * says how many agents it would leave given the search, coverage and tags
 * already chosen.
 */
const unstatusedAgents = computed(() => {
  let list = [...agents.value]

  if (filterCoverage.value !== 'all') {
    const hostIds = coverageHostIds.value[filterCoverage.value]
    list = list.filter((agent) => hostIds.has(agent.id))
  }

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (m) =>
        m.hostname.toLowerCase().includes(q) ||
        (m.display_name?.toLowerCase().includes(q) ?? false) ||
        (m.domain?.toLowerCase().includes(q) ?? false) ||
        (agentTagsMap.value[m.id] ?? []).some((t) => t.name.toLowerCase().includes(q)),
    )
  }

  if (filterTagIds.value.length > 0) {
    const selectedNames = new Set(
      allAgentTags.value.filter((t) => filterTagIds.value.includes(t.id)).map((t) => t.name),
    )
    list = list.filter((m) =>
      (agentTagsMap.value[m.id] ?? []).some((t) => selectedNames.has(t.name)),
    )
  }

  return list
})

const statusPills = computed(() =>
  STATUS_PILLS.map((pill) => ({
    ...pill,
    count: unstatusedAgents.value.filter((agent) => matchesStatus(agent, pill.value)).length,
  })),
)

const filteredAgents = computed(() => {
  const list = unstatusedAgents.value.filter((agent) => matchesStatus(agent, filterStatus.value))

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'hostname':
        cmp = a.hostname.localeCompare(b.hostname)
        break
      case 'status':
        cmp = Number(b.is_connected) - Number(a.is_connected)
        break
      case 'last_seen':
        cmp = (a.last_seen_at ?? '').localeCompare(b.last_seen_at ?? '')
        break
      case 'version':
        cmp = (a.agent_version ?? '').localeCompare(b.agent_version ?? '')
        break
    }
    return cmp * sortSign()
  })

  return list
})

const showAddDialog = ref(false)
const addForm = reactive({ hostname: '', display_name: '', domain: '' })
const addLoading = ref(false)
const addError = ref<string | null>(null)
const newToken = ref<string | null>(null)
const { copied: tokenCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showAddDialog, closeAddDialog)

function isOnline(agent: AgentRow): boolean {
  return agent.is_connected ?? false
}

function isImported(agent: AgentRow): boolean {
  return agent.is_imported ?? false
}

function formatLastSeen(iso: string | null | undefined): string {
  if (!iso) return 'Never'
  const ts = new Date(iso).getTime()
  if (isNaN(ts) || ts === 0) return 'Never'
  const diff = Date.now() - ts
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'Just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  const days = Math.floor(hrs / 24)
  return `${days}d ago`
}

function scheduleCount(agent: AgentRow): number {
  return machineScheduleCount.value[agent.id] ?? 0
}

function agentTags(agent: AgentRow): { name: string; color: string }[] {
  return agentTagsMap.value[agent.id] ?? []
}

function agentHealthStatus(agent: AgentRow): AgentHealth | null {
  return healthByHost.value[agent.hostname] ?? null
}

/**
 * Whether nothing at all backs this host up.
 *
 * Deliberately conservative, because the claim is an accusation: it needs the
 * schedule-count request to have actually answered (a failed one leaves the
 * map empty, which is not the same as every host having none), and it needs
 * the health summary to agree - a host with health entries has schedules,
 * whatever the count endpoint says.
 */
function isUnprotected(agent: AgentRow): boolean {
  if (!scheduleCountsKnown.value) return false
  if (scheduleCount(agent) > 0) return false
  return (agentHealthStatus(agent)?.total ?? 0) === 0
}

// The most recent backup that actually completed, across this agent's
// schedules. Whether the agent is *behind* is a separate, server-computed
// question, reported by the overdue chip rather than re-derived here.
function lastBackupAt(agent: AgentRow): string | null {
  return agentHealthStatus(agent)?.mostRecentBackupAt ?? null
}

interface FleetVersionCount {
  version: string
  count: number
  /** `badge--success` for the version the server ships, `--warning` behind it. */
  badgeClass: string
  /** Says what the colour means, for a pointer and for assistive technology. */
  title: string
}

/**
 * The tiles above the list. Counted over every listed agent rather than the
 * filtered list: they summarise the fleet, and a filter narrowing the rows
 * below should not change what the fleet is.
 */
const fleetSummary = computed(() => {
  const all = agents.value
  const total = all.length
  const online = all.filter(isOnline).length
  const backingUp = all.filter((a) => hostActiveBackups(a).length > 0).length
  const attention = all.filter(needsAttention).length
  const failing = all.filter((a) => issueKinds(a).includes('failed')).length
  const overdue = all.filter((a) => issueKinds(a).includes('overdue')).length
  const unprotected = all.filter((a) => issueKinds(a).includes('unprotected')).length
  const totalSchedules = fleetScheduleCount.value

  // Keyed on the raw `agent_version`, `null` included, so "never reported one"
  // stays a distinct key instead of colliding with an agent that genuinely
  // reports the string "unknown" - and so the comparisons below never test a
  // wide string against a literal.
  const versionCounts = new Map<string | null, number>()
  for (const a of all) {
    versionCounts.set(a.agent_version, (versionCounts.get(a.agent_version) ?? 0) + 1)
  }
  const available = availableAgentVersion.value
  const versions: FleetVersionCount[] = [...versionCounts.entries()]
    .map(([version, count]) => {
      const current = version !== null && version === available
      // Behind the binary the server has. Silent when the server has none:
      // then there is nothing to be behind of.
      const outdated = version !== null && available !== null && version !== available
      return {
        version: version ?? 'unknown',
        count,
        badgeClass: current ? 'badge--success' : outdated ? 'badge--warning' : 'badge--neutral',
        title: current
          ? 'The version this server ships'
          : outdated
            ? 'Behind the version this server ships'
            : version === null
              ? 'Never reported a version'
              : 'No server binary to compare against',
      }
    })
    .sort((a, b) => b.count - a.count)

  const attentionParts = [`${failing} failing`, `${overdue} overdue`]
  if (unprotected > 0) attentionParts.push(`${unprotected} unprotected`)

  return {
    total,
    online,
    backingUp,
    attention,
    attentionSub: attentionParts.join(' · '),
    totalSchedules,
    versions,
  }
})

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? '' : 's'}`
}

/** A host offline for longer than this reads as gone, not as briefly away. */
const STALE_LAST_SEEN_MS = 24 * 60 * 60 * 1000

/**
 * The tone of the last-backup figure. A host with no schedule gets none - the
 * card's own "No schedules" chip says why the value reads "Never", and a red
 * figure beside a red chip would say it twice.
 */
function lastBackupClass(agent: AgentRow): string | null {
  if (isUnprotected(agent)) return null
  const health = agentHealthStatus(agent)
  if (!health) return null
  if (!health.mostRecentBackupAt) return 'stat-value--danger'
  if (health.failed > 0 || health.overdue > 0) return 'stat-value--warning'
  return 'stat-value--success'
}

/** The tone of the last-seen figure: only an agent that is gone gets one. */
function lastSeenClass(agent: AgentRow): string | null {
  if (agent.is_connected) return null
  if (!agent.last_seen_at) return 'stat-value--danger'
  const ts = new Date(agent.last_seen_at).getTime()
  if (isNaN(ts)) return 'stat-value--danger'
  return Date.now() - ts > STALE_LAST_SEEN_MS ? 'stat-value--danger' : 'stat-value--warning'
}

/** One panel of rows: a status (Needs attention, Online, Offline) or a version. */
interface AgentListGroup {
  key: string
  /** The status, the version itself, or "Unknown" for one never reported. */
  title: string
  /** Version titles are machine values and read in the mono face. */
  mono: boolean
  /** The status dot before a status group's title; none on a version group. */
  dot: DotTone | null
  /** What a version group is relative to the binary the server has, when it knows. */
  badge: { label: string; className: string } | null
  /** A short note at the header's far end. */
  aside: string | null
  agents: AgentRow[]
  /** Whether the group shows only its first `OFFLINE_PREVIEW` rows until asked. */
  collapsible: boolean
}

/**
 * The list bucketed by the agent version each host reports - the shape
 * SchedulesView uses for its time buckets. A fleet running two builds reads as
 * two headings rather than as eight values to compare by eye.
 */
const versionGroups = computed<AgentListGroup[]>(() => {
  const byVersion = new Map<string | null, AgentRow[]>()
  for (const agent of filteredAgents.value) {
    const list = byVersion.get(agent.agent_version) ?? []
    list.push(agent)
    byVersion.set(agent.agent_version, list)
  }

  const available = availableAgentVersion.value
  const known = [...byVersion.keys()].filter((version): version is string => version !== null)
  // Current first, then newest to oldest. Numeric collation so 0.1.108 sorts
  // above 0.1.98 rather than below it, as a plain string compare would.
  known.sort((a, b) => {
    if (available !== null) {
      if (a === available) return -1
      if (b === available) return 1
    }
    return b.localeCompare(a, undefined, { numeric: true })
  })

  const groups: AgentListGroup[] = known.map((version) => {
    const isCurrent = version === available
    return {
      key: `version:${version}`,
      title: version,
      mono: true,
      dot: null,
      // Without a server binary to compare against, "behind" is not a claim
      // the UI can make - so it says nothing rather than guessing.
      badge:
        available === null
          ? null
          : isCurrent
            ? { label: 'Current', className: 'badge--success' }
            : { label: 'Behind', className: 'badge--warning' },
      aside: null,
      agents: byVersion.get(version) ?? [],
      collapsible: false,
    }
  })

  const unknown = byVersion.get(null)
  if (unknown) {
    groups.push({
      key: 'version:unknown',
      title: 'Unknown',
      mono: false,
      dot: null,
      badge: { label: 'Never reported', className: 'badge--neutral' },
      aside: null,
      agents: unknown,
      collapsible: false,
    })
  }

  return groups
})

/**
 * The list bucketed by how each agent is doing: what needs looking at first,
 * then the healthy hosts that are up, then the ones that are down with nothing
 * due. Within a group the toolbar's sort holds, except that a failing host
 * leads the attention group - a failure is worse than a late or missing
 * schedule, whatever its name.
 */
const statusGroups = computed<AgentListGroup[]>(() => {
  const attention: AgentRow[] = []
  const online: AgentRow[] = []
  const offline: AgentRow[] = []
  for (const agent of filteredAgents.value) {
    if (needsAttention(agent)) attention.push(agent)
    else if (isOnline(agent)) online.push(agent)
    else offline.push(agent)
  }
  // Array.prototype.sort is stable, so the toolbar's order survives within
  // each rank.
  attention.sort((a, b) => attentionRank(a) - attentionRank(b))

  const backingUp = online.filter((agent) => hostActiveBackups(agent).length > 0).length
  const groups: AgentListGroup[] = [
    {
      key: 'status:attention',
      title: 'Needs attention',
      mono: false,
      dot: 'warning',
      badge: null,
      aside: null,
      agents: attention,
      collapsible: false,
    },
    {
      key: 'status:online',
      title: 'Online',
      mono: false,
      dot: 'success',
      badge: null,
      aside: backingUp > 0 ? `${backingUp} backing up now` : null,
      agents: online,
      collapsible: false,
    },
    {
      key: 'status:offline',
      title: 'Offline',
      mono: false,
      dot: 'muted',
      badge: null,
      aside: 'Nothing due',
      agents: offline,
      collapsible: true,
    },
  ]
  return groups.filter((group) => group.agents.length > 0)
})

const agentGroups = computed<AgentListGroup[]>(() =>
  groupMode.value === 'version' ? versionGroups.value : statusGroups.value,
)

/** The rows a group shows: all of them, or the first few until expanded. */
function visibleAgents(group: AgentListGroup): AgentRow[] {
  if (!group.collapsible || showAllOffline.value) return group.agents
  return group.agents.slice(0, OFFLINE_PREVIEW)
}

/** The rows a collapsed group is holding back. */
function hiddenCount(group: AgentListGroup): number {
  return group.collapsible ? Math.max(0, group.agents.length - OFFLINE_PREVIEW) : 0
}

/** Where each issue chip lands on the agent's detail page. */
const ISSUE_QUERY: Record<'failed' | 'overdue' | 'unprotected', Record<string, string>> = {
  failed: { tab: 'schedules' },
  overdue: { tab: 'schedules', health: 'overdue' },
  unprotected: { tab: 'schedules' },
}

function agentIssues(agent: AgentRow): EntityIssue[] {
  const issues: EntityIssue[] = []
  // A host nothing is scheduled against is the coverage gap the dashboard
  // already reports; on the card it was a grey `0` that read like a quiet,
  // healthy machine.
  if (isUnprotected(agent)) {
    issues.push({
      key: 'unprotected',
      label: 'No schedules',
      severity: 'danger',
      title: 'Nothing is scheduled to back this host up',
      onClick: () => navigateToAgentIssue(agent, 'unprotected'),
    })
  }
  const h = agentHealthStatus(agent)
  if (!h) return issues
  if (h.failed > 0) {
    issues.push({
      key: 'failed',
      label: `${h.failed} failed`,
      severity: 'danger',
      onClick: () => navigateToAgentIssue(agent, 'failed'),
    })
  }
  if (h.overdue > 0) {
    issues.push({
      key: 'overdue',
      label: `${h.overdue} overdue`,
      severity: 'warning',
      onClick: () => navigateToAgentIssue(agent, 'overdue'),
    })
  }
  return issues
}

type IssueKind = 'failed' | 'overdue' | 'unprotected'

/**
 * The issues an agent's chips report, by kind - the same three checks
 * `agentIssues` makes, without the chip wiring, for the grouping, the pills
 * and the tiles to count on.
 */
function issueKinds(agent: AgentRow): IssueKind[] {
  const kinds: IssueKind[] = []
  if (isUnprotected(agent)) kinds.push('unprotected')
  const h = agentHealthStatus(agent)
  if (h && h.failed > 0) kinds.push('failed')
  if (h && h.overdue > 0) kinds.push('overdue')
  return kinds
}

/** Whether the agent carries an issue chip, which is what "needs attention" means. */
function needsAttention(agent: AgentRow): boolean {
  return issueKinds(agent).length > 0
}

/** Failing first, then nothing scheduled, then only late. */
function attentionRank(agent: AgentRow): number {
  const kinds = issueKinds(agent)
  if (kinds.includes('failed')) return 0
  if (kinds.includes('unprotected')) return 1
  return 2
}

/**
 * The row's dot, worst first: a failure, then a backup in flight (which says
 * more right now than an older lateness), then any other issue, then whether
 * the agent is reachable at all.
 */
function rowTone(agent: AgentRow): DotTone {
  const kinds = issueKinds(agent)
  if (kinds.includes('failed')) return 'danger'
  if (hostActiveBackups(agent).length > 0) return 'info'
  if (kinds.length > 0) return 'warning'
  return isOnline(agent) ? 'success' : 'muted'
}

function navigateToAgentIssue(agent: AgentRow, kind: 'failed' | 'overdue' | 'unprotected'): void {
  const query = ISSUE_QUERY[kind]
  router.push({
    path: `/agents/${agent.hostname}`,
    query: { ...query, ...domainParams(agent.domain) },
  })
}

function toggleTagFilter(tagId: number): void {
  const idx = filterTagIds.value.indexOf(tagId)
  if (idx === -1) {
    filterTagIds.value = [...filterTagIds.value, tagId]
  } else {
    filterTagIds.value = filterTagIds.value.filter((id) => id !== tagId)
  }
}

async function loadAgents(): Promise<void> {
  if (agents.value.length === 0) {
    loading.value = true
    error.value = null
  }
  try {
    agents.value = await listAgents(showHidden.value)
    error.value = null
    loading.value = false

    const emptyOverview: DashboardOverview = {
      summary: {
        protected_hosts: 0,
        eligible_hosts: 0,
        needs_attention: 0,
        running_operations: 0,
        total_storage_bytes: 0,
      },
      findings: [],
      protection: {
        protected_hosts: 0,
        eligible_hosts: 0,
        protected_agent_links: [],
        unassigned_agents: [],
        never_succeeded_targets: 0,
        never_succeeded_agents: [],
        disabled_only_agents: [],
      },
      running_operations: [],
      upcoming_schedules: [],
      repository_capacity: [],
    }
    const [
      agentTagAssocRes,
      agentTagsRes,
      healthRes,
      scheduleCountsRes,
      schedulesRes,
      overviewRes,
    ] = await Promise.all([
      listAgentTags({ timeout: 8000 }).catch(() => [] as AgentTagEntryResponse[]),
      listTags('host', { timeout: 8000 }).catch(() => [] as TagRow[]),
      getScheduleHealth({ timeout: 8000 }).catch(() => [] as HealthSummaryResponse[]),
      getScheduleCounts({ timeout: 8000 }).catch(() => null),
      listSchedules({ timeout: 8000 }).catch(() => [] as { id: number }[]),
      getDashboardOverview({ timeout: 8000 }).catch(() => emptyOverview),
    ])
    if (scheduleCountsRes) {
      const counts: Record<number, number> = {}
      scheduleCountsRes.forEach((entry) => {
        counts[entry.agent_id] = entry.count
      })
      machineScheduleCount.value = counts
      scheduleCountsKnown.value = true
    }
    // Fleet-wide total is the distinct schedule count, not a sum of
    // per-agent counts - a schedule targeting N agents would otherwise be
    // counted N times.
    fleetScheduleCount.value = schedulesRes.length

    allAgentTags.value = agentTagsRes
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    agentTagAssocRes.forEach((ht) => {
      if (!tagMap[ht.agent_id]) tagMap[ht.agent_id] = []
      tagMap[ht.agent_id].push({ name: ht.tag_name, color: ht.tag_color })
    })
    agentTagsMap.value = tagMap

    const hMap: Record<string, AgentHealth> = {}
    healthRes.forEach((entry) => {
      if (!hMap[entry.hostname]) {
        hMap[entry.hostname] = {
          failed: 0,
          overdue: 0,
          warning: 0,
          total: 0,
          last_error_message: null,
          mostRecentBackupAt: null,
        }
      }
      const host = hMap[entry.hostname]
      host.total++
      const status = entry.last_status !== null ? normalizeBackupStatus(entry.last_status) : null
      if (status === 'failed') {
        host.failed++
        if (entry.last_error_message) {
          host.last_error_message = entry.last_error_message
        }
      }
      if (status === 'warning') host.warning++
      if (entry.is_overdue) host.overdue++

      // `last_success_at` is the newest run that produced an archive, so a
      // failed latest run doesn't hide the backup that came before it. A
      // caller that omits the field (an e2e mock, an older cached payload)
      // falls back to the last *completed* run, gated on its own outcome
      // rather than `status` above: while a newer run is in flight, `status`
      // is null, which must not hide a prior success. Truthy rather than
      // `!== null`: an omitted field is `undefined`, and
      // normalizeBackupStatus() would throw on that and abort this whole
      // loop before `healthByHost.value` is ever assigned.
      const completedStatus = entry.last_backup_status
        ? normalizeBackupStatus(entry.last_backup_status)
        : null
      const completedSuccessAt =
        completedStatus === 'success' || completedStatus === 'warning' ? entry.last_backup_at : null
      const lastSuccessAt = entry.last_success_at ?? completedSuccessAt
      if (lastSuccessAt && (!host.mostRecentBackupAt || lastSuccessAt > host.mostRecentBackupAt)) {
        host.mostRecentBackupAt = lastSuccessAt
      }
    })
    healthByHost.value = hMap

    const activeMap: Record<string, string[]> = {}
    overviewRes.running_operations.forEach((op) => {
      const list = activeMap[op.hostname] ?? []
      if (!list.includes(op.repo_name)) list.push(op.repo_name)
      activeMap[op.hostname] = list
    })
    activeBackupsByHost.value = activeMap

    coverageHostIds.value = {
      protected: new Set(overviewRes.protection.protected_agent_links.map((host) => host.agent_id)),
      unassigned: new Set(overviewRes.protection.unassigned_agents.map((host) => host.agent_id)),
      'never-succeeded': new Set(
        overviewRes.protection.never_succeeded_agents.map((host) => host.agent_id),
      ),
      'disabled-only': new Set(
        overviewRes.protection.disabled_only_agents.map((host) => host.agent_id),
      ),
    }
  } catch (e: unknown) {
    if (agents.value.length === 0) {
      error.value = extractError(e)
    }
  } finally {
    loading.value = false
  }
}

function openAddDialog(): void {
  addForm.hostname = ''
  addForm.display_name = ''
  addForm.domain = ''
  addError.value = null
  newToken.value = null
  tokenCopied.value = false
  showAddDialog.value = true
}

async function submitAdd(): Promise<void> {
  const hostname = addForm.hostname.replaceAll(/\s/g, '')
  if (!hostname) {
    addError.value = 'Hostname is required'
    return
  }
  addLoading.value = true
  addError.value = null
  try {
    const res = await createAgent({
      hostname,
      display_name: addForm.display_name.trim() || null,
      domain: addForm.domain.trim() || null,
    })
    agents.value.push({ ...res.agent, id: Number(res.agent.id) })
    newToken.value = res.token
  } catch (e: unknown) {
    addError.value = extractError(e)
  } finally {
    addLoading.value = false
  }
}

function closeAddDialog(): void {
  showAddDialog.value = false
  newToken.value = null
}

function agentLink(agent: AgentRow): { path: string; query: Record<string, string> } {
  return { path: `/agents/${agent.hostname}`, query: domainParams(agent.domain) }
}

onMounted(() => {
  loadAgents().catch(logger.error)
  getSystemVersion()
    .then((res) => {
      availableAgentVersion.value = res.agent_version
    })
    .catch(logger.error)
})

const { onMessage, status: wsStatus } = useWebSocket()
onMessage('AgentConnected', () => loadAgents().catch(logger.error))
onMessage('AgentDisconnected', () => loadAgents().catch(logger.error))
onMessage('DataChanged', () => loadAgents().catch(logger.error))

const activeBackupsByHost = ref<Record<string, string[]>>({})

const availableAgentVersion = ref<string | null>(null)
onMessage('BackupStarted', (payload) => {
  const list = activeBackupsByHost.value[payload.hostname] ?? []
  if (!list.includes(payload.target_name)) {
    activeBackupsByHost.value = {
      ...activeBackupsByHost.value,
      [payload.hostname]: [...list, payload.target_name],
    }
  }
})

onMessage('BackupCompleted', (payload) => {
  const list = activeBackupsByHost.value[payload.hostname]
  if (list) {
    const filtered = list.filter((t) => t !== payload.target_name)
    if (filtered.length === 0) {
      const copy = { ...activeBackupsByHost.value }
      delete copy[payload.hostname]
      activeBackupsByHost.value = copy
    } else {
      activeBackupsByHost.value = { ...activeBackupsByHost.value, [payload.hostname]: filtered }
    }
  }
  loadAgents().catch(logger.error)
})

function hostActiveBackups(agent: AgentRow): string[] {
  return activeBackupsByHost.value[agent.hostname] ?? []
}

function hostRunningLabel(agent: AgentRow): string {
  const targets = hostActiveBackups(agent)
  return targets.length > 0 ? `Backing up: ${targets.join(', ')}` : 'Running'
}

/**
 * Keeps a click on a control inside a row - an issue chip - from also
 * following the row's link. Runs in the capture phase, before the
 * control's own handler stops the event, and only cancels the link's default:
 * the control still does what it was clicked for.
 */
function guardRowControls(event: MouseEvent): void {
  if (event.target instanceof Element && event.target.closest('button')) {
    event.preventDefault()
  }
}

/** The second line under a row's name on a phone, where the columns are gone. */
function narrowSummary(agent: AgentRow): string {
  const parts: string[] = []
  if (agent.display_name) parts.push(agent.display_name)
  parts.push(plural(scheduleCount(agent), 'schedule'))
  if (agent.agent_version) parts.push(agent.agent_version)
  return parts.join(' · ')
}

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    loadAgents().catch(logger.error)
  }
})

watch(showHidden, () => {
  loadAgents().catch(logger.error)
})

useQueryOverride(() => route.query.status, isFilterStatus, filterStatus)
useQueryOverride(() => route.query.coverage, isCoverageFilter, filterCoverage)
</script>

<template>
  <div class="hosts-view">
    <div class="page-header">
      <h1 class="page-title">Agents</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openAddDialog"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <div>
      <!-- Search and a Filters button on one line, the status pills under
           them; everything else waits in the filters card. -->
      <div class="agents-filter-bar">
        <div class="agents-search-row">
          <input
            v-model="filterText"
            class="input search-input"
            placeholder="Filter by hostname or tag..."
          />
          <button
            type="button"
            class="btn btn-sm"
            :class="{ active: showMobileFilters }"
            :aria-expanded="showMobileFilters"
            @click="showMobileFilters = !showMobileFilters"
          >
            <SlidersHorizontal :size="14" />
            Filters{{ activeFilterCount > 0 ? ` · ${activeFilterCount}` : '' }}
          </button>
        </div>
        <div
          class="filter-pills"
          role="group"
          aria-label="Status"
        >
          <button
            v-for="pill in statusPills"
            :key="pill.value"
            type="button"
            class="filter-pill"
            :class="{ active: filterStatus === pill.value }"
            :aria-pressed="filterStatus === pill.value"
            @click="filterStatus = pill.value"
          >
            <span
              v-if="pill.dot"
              class="status-dot"
              :class="DOT_CLASS[pill.dot]"
            ></span>
            {{ pill.label }} · {{ pill.count }}
          </button>
        </div>
        <div
          v-if="showMobileFilters"
          class="panel agents-filters"
        >
          <div class="pane-rows">
            <PaneRow
              title="Coverage"
              label-for="agents-coverage"
            >
              <select
                id="agents-coverage"
                v-model="filterCoverage"
                class="input"
              >
                <option value="all">All coverage</option>
                <option value="protected">Protected</option>
                <option value="unassigned">Unassigned</option>
                <option value="never-succeeded">Never succeeded</option>
                <option value="disabled-only">Disabled schedules only</option>
              </select>
            </PaneRow>
            <PaneRow
              v-if="allAgentTags.length > 0"
              title="Tags"
              stack
            >
              <div
                class="filter-pills"
                role="group"
                aria-label="Tags"
              >
                <button
                  v-for="tag in allAgentTags"
                  :key="tag.id"
                  type="button"
                  class="filter-pill"
                  :class="{ active: filterTagIds.includes(tag.id) }"
                  :aria-pressed="filterTagIds.includes(tag.id)"
                  @click="toggleTagFilter(tag.id)"
                >
                  <span
                    class="status-dot"
                    :style="{ background: tag.color }"
                  ></span>
                  {{ tag.name }}
                </button>
              </div>
            </PaneRow>
            <PaneRow title="Group by">
              <BaseSegmented
                v-model="groupMode"
                :options="GROUP_OPTIONS"
                label="Group agents by"
              />
            </PaneRow>
            <PaneRow
              title="Sort by"
              stack
            >
              <SortControls
                :field="sortField"
                :direction="sortDir"
                :options="SORT_OPTIONS"
                hide-label
                @toggle="toggleSort"
              />
            </PaneRow>
            <PaneRow
              v-if="isAdmin"
              title="Show hidden agents"
            >
              <ToggleSwitch
                v-model="showHidden"
                label="Show hidden agents"
              />
            </PaneRow>
          </div>
        </div>
      </div>

      <div
        v-if="!loading && agents.length > 0"
        class="fleet-strip"
      >
        <div class="fleet-cell">
          <span class="stat-label">Agents</span>
          <span class="stat-value stat-value--lg">{{ fleetSummary.total }}</span>
          <span class="stat-sub">{{ plural(fleetSummary.totalSchedules, 'schedule') }}</span>
        </div>
        <div class="fleet-cell">
          <span class="stat-label">Online</span>
          <span class="stat-value stat-value--lg stat-value--success">{{
            fleetSummary.online
          }}</span>
          <span
            v-if="fleetSummary.backingUp > 0"
            class="stat-sub"
            >{{ fleetSummary.backingUp }} backing up now</span
          >
        </div>
        <div class="fleet-cell">
          <span class="stat-label">Needs attention</span>
          <span
            class="stat-value stat-value--lg"
            :class="{ 'stat-value--warning': fleetSummary.attention > 0 }"
            >{{ fleetSummary.attention }}</span
          >
          <span class="stat-sub">{{ fleetSummary.attentionSub }}</span>
        </div>
        <div class="fleet-cell">
          <span class="stat-label">Versions</span>
          <div class="fleet-versions">
            <span
              v-for="v in fleetSummary.versions"
              :key="v.version"
              class="badge mono"
              :class="v.badgeClass"
              :title="v.title"
              >{{ v.version }} · {{ v.count }}</span
            >
          </div>
        </div>
      </div>

      <BaseSpinner
        v-if="loading"
        size="lg"
      />
      <div
        v-else-if="error"
        class="error-banner"
      >
        {{ error }}
      </div>
      <EmptyState
        v-else-if="agents.length === 0"
        :icon="Server"
        title="No agents registered"
        description="Add your first agent to start backing up."
        action="New agent"
        @action="showAddDialog = true"
      />
      <EmptyState
        v-else-if="filteredAgents.length === 0"
        :icon="Server"
        title="No agents match"
        description="No agents match the current filter."
      />

      <div
        v-else
        class="list-stack agents-list"
      >
        <section
          v-for="group in agentGroups"
          :key="group.key"
          class="list-group"
        >
          <div class="list-group-header">
            <span
              v-if="group.dot"
              class="status-dot"
              :class="DOT_CLASS[group.dot]"
            ></span>
            <h2
              class="list-group-title"
              :class="{ mono: group.mono }"
            >
              {{ group.title }}
            </h2>
            <span
              v-if="group.badge"
              class="badge"
              :class="group.badge.className"
              >{{ group.badge.label }}</span
            >
            <span class="list-group-count">{{ plural(group.agents.length, 'agent') }}</span>
            <span
              v-if="group.aside"
              class="list-group-aside muted"
              >{{ group.aside }}</span
            >
          </div>
          <div
            class="list-cols"
            aria-hidden="true"
          >
            <span>Agent</span>
            <span>Status</span>
            <span>Schedules</span>
            <span>Last backup</span>
            <span>Last seen</span>
            <span>Version</span>
            <span></span>
          </div>
          <RouterLink
            v-for="agent in visibleAgents(group)"
            :key="agent.id"
            :to="agentLink(agent)"
            class="list-row"
            :class="{ 'agents-row--dim': agent.is_hidden || !isOnline(agent) }"
            :data-hostname="agent.hostname"
            @click.capture="guardRowControls"
          >
            <span class="list-row-main">
              <span
                class="status-dot"
                :class="DOT_CLASS[rowTone(agent)]"
              ></span>
              <span class="list-row-text">
                <span class="list-row-name"
                  >{{ agent.hostname
                  }}<span
                    v-if="agent.domain"
                    class="muted"
                  >
                    ({{ agent.domain }})</span
                  ></span
                >
                <span
                  v-if="agent.display_name"
                  class="list-row-sub list-wide-only"
                  >{{ agent.display_name }}</span
                >
                <span class="list-row-sub list-narrow-only">{{ narrowSummary(agent) }}</span>
              </span>
              <span
                class="list-narrow-only agents-row-when list-row-value"
                :class="lastBackupClass(agent)"
                >{{ formatLastSeen(lastBackupAt(agent)) }}</span
              >
            </span>
            <span class="list-row-chips">
              <EntityStatusBadges
                :notable="!isOnline(agent)"
                notable-label="Offline"
                :running="hostActiveBackups(agent).length > 0"
                :running-label="hostRunningLabel(agent)"
                :issues="agentIssues(agent)"
              />
              <span
                v-if="agent.is_hidden"
                class="badge badge--neutral"
                >Hidden</span
              >
              <span
                v-if="isImported(agent)"
                class="badge badge--accent"
                >Imported</span
              >
              <span
                v-for="tag in agentTags(agent)"
                :key="tag.name"
                class="tag-pill"
                :style="{
                  background: tag.color + '22',
                  color: tag.color,
                  borderColor: tag.color + '44',
                }"
              >
                {{ tag.name }}
              </span>
            </span>
            <span
              class="list-row-value list-wide-only"
              data-col="schedules"
              >{{ scheduleCount(agent) }}</span
            >
            <span
              class="list-row-value list-wide-only"
              :class="lastBackupClass(agent)"
              data-col="last-backup"
              >{{ formatLastSeen(lastBackupAt(agent)) }}</span
            >
            <span
              class="list-row-value list-wide-only"
              :class="lastSeenClass(agent)"
              data-col="last-seen"
              >{{ formatLastSeen(agent.last_seen_at) }}</span
            >
            <span
              class="list-row-value list-wide-only mono"
              data-col="version"
              >{{ agent.agent_version ?? '-' }}</span
            >
            <ChevronRight
              :size="16"
              class="list-row-chevron"
            />
          </RouterLink>
          <button
            v-if="hiddenCount(group) > 0"
            type="button"
            class="list-more"
            @click="showAllOffline = !showAllOffline"
          >
            {{
              showAllOffline
                ? 'Show fewer'
                : `Show ${hiddenCount(group)} more offline agent${hiddenCount(group) === 1 ? '' : 's'}`
            }}
          </button>
        </section>
      </div>
    </div>

    <!-- Add Agent Dialog. One dialog, two states: collect the hostname,
         then reveal the generated token once. -->
    <BaseModal
      :open="showAddDialog"
      :title="newToken ? 'Agent Created' : 'Add Agent'"
      @close="closeAddDialog"
    >
      <template v-if="!newToken">
        <div class="field">
          <label class="field-label">Hostname <span class="required">*</span></label>
          <input
            v-model="addForm.hostname"
            class="input"
            placeholder="e.g. workstation-01"
            @keyup.enter="submitAdd"
          />
          <span class="field-hint"
            >Must match the machine's actual hostname (output of <code>hostname</code>).</span
          >
        </div>
        <div class="field">
          <label class="field-label">Domain</label>
          <input
            v-model="addForm.domain"
            class="input"
            placeholder="Optional, e.g. lab.example.com"
          />
          <span class="field-hint">Only needed if another host already uses this hostname.</span>
        </div>
        <div class="field">
          <label class="field-label">Display name</label>
          <input
            v-model="addForm.display_name"
            class="input"
            placeholder="Optional friendly name"
          />
        </div>
        <div
          v-if="addError"
          class="form-error"
        >
          {{ addError }}
        </div>
      </template>

      <div
        v-else
        class="token-notice"
      >
        <p class="token-warning">Copy this agent token now. It will not be shown again.</p>
        <div class="token-box">
          <code class="token-text">{{ newToken }}</code>
          <button
            type="button"
            class="btn btn-sm btn-ghost"
            @click="copyToClipboard(newToken ?? '')"
          >
            {{ tokenCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
        <AgentChannelKey />
      </div>

      <template #footer>
        <template v-if="!newToken">
          <button
            type="button"
            class="btn btn-ghost"
            @click="closeAddDialog"
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn btn-primary"
            :disabled="addLoading || !addForm.hostname.trim()"
            @click="submitAdd"
          >
            {{ addLoading ? 'Creating...' : 'Create' }}
          </button>
        </template>
        <button
          v-else
          type="button"
          class="btn btn-primary"
          @click="closeAddDialog"
        >
          Done
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.agents-filter-bar {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  margin-bottom: var(--space-7);
}

.agents-search-row {
  display: flex;
  gap: var(--space-4);
}

.agents-search-row .search-input {
  flex: 1;
  min-width: 0;
  max-width: 28rem;
}

.agents-search-row .btn {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}

.agents-filters {
  padding-block: 0;
}

.agents-filters .pane-rows {
  border-top: none;
}

.agents-filters .pane-row:last-child {
  border-bottom: none;
}

.hosts-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

/* The status pills, the coverage select, the toggles and the sort share one
   strip, which wraps rather than pushing the page sideways. */
/* The fleet in four figures, as one ruled strip rather than four cards. */
.fleet-strip {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
  margin-bottom: var(--space-7);
}

.fleet-cell {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-5) var(--space-6);
  min-width: 0;
}

.fleet-cell + .fleet-cell {
  border-left: 1px solid var(--border);
}

.fleet-versions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-top: var(--space-1);
}

/* Agent | Status | Schedules | Last backup | Last seen | Version | menu */
.agents-list .list-group {
  --list-cols: minmax(0, 2.2fr) minmax(0, 2.2fr) 5rem 6rem 6rem 5.5rem 2rem;
}

.agents-row--dim .list-row-text,
.agents-row--dim .list-row-value {
  opacity: 0.7;
}

.agents-row-when {
  margin-left: auto;
  font-size: var(--fs-xs);
}

@media (max-width: 640px) {
  .fleet-strip {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .fleet-cell + .fleet-cell {
    border-left: 0;
  }

  .fleet-cell:nth-child(even) {
    border-left: 1px solid var(--border);
  }

  .fleet-cell:nth-child(n + 3) {
    border-top: 1px solid var(--border);
  }

  /* The last-backup figure sits at the end of the name's line, clear of the
     chevron pinned to the row's top-right corner. */
  .agents-row-when {
    padding-right: var(--space-8);
  }
}
</style>

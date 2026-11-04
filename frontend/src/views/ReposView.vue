<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useAuthStore } from '../stores/auth'
import { useMobile } from '../composables/useMobile'
import { useListSort } from '../composables/useListSort'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import { formatBytes } from '../utils/format'
import { useAsyncAction } from '../composables/useAsyncAction'
import { Plus, Download, SlidersHorizontal, Database } from '@lucide/vue'
import RepoCreateDialog from '../components/RepoCreateDialog.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import BaseSegmented, { type SegmentedOption } from '../components/BaseSegmented.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import RepoListRow from '../components/RepoListRow.vue'
import type { Repo, RepoWithStats } from '../types/repo'
import type { TagRow } from '../types/tag'
import type { RepoTagEntryResponse, ServerQuotaResponse } from '../types/generated'
import { listServerQuotas } from '../api/serverQuotas'
import { listRepoStats, listRepoTags } from '../api/repos'
import { listTags } from '../api/tags'
import {
  actionForHealth,
  actionLabel,
  computeSliceGeometry,
  quotaCeiling,
  quotaHealth,
} from '../utils/quota'

type AddTab = 'import' | 'create'
type SortField = 'name' | 'size' | 'last_backup' | 'quota'

const SORT_OPTIONS: readonly { field: SortField; label: string }[] = [
  { field: 'name', label: 'Name' },
  { field: 'size', label: 'Size' },
  { field: 'last_backup', label: 'Last backup' },
  { field: 'quota', label: 'Quota' },
]
type QuotaFilter = 'all' | 'at_risk' | 'no_quota'
type GroupMode = 'host' | 'tag' | 'none'

/** A tag as the repositories list resolves it for one repository. */
interface RepoTag {
  name: string
  color: string
}

interface TagGroup {
  label: string
  color: string | null
  repos: RepoWithStats[]
}

interface HostGroupEntry {
  repo: RepoWithStats
  offsetBytes: number
  visible: boolean
  colorStep: number
}

interface HostGroup {
  sshHost: string
  /** The repository host every repository in the group lives on. */
  repoHostId: number
  /** Whether that host is marked as not always online. */
  intermittent: boolean
  entries: HostGroupEntry[]
  totalDeduplicated: number
  serverQuota: ServerQuotaResponse | null
  boxMaxBytes: number | null
  visibleCount: number
}

/** One bordered panel of rows: a host, a tag, or every repository at once. */
interface ListGroup {
  key: string
  /** The host pool the panel stands for, when grouped by host. */
  host: HostGroup | null
  title: string
  color: string | null
  rows: { repo: RepoWithStats; dim: boolean }[]
}

const authStore = useAuthStore()
const repos = ref<RepoWithStats[]>([])
const { loading, error, run } = useAsyncAction()

const {
  field: sortField,
  direction: sortDir,
  toggle: toggleSort,
  sign: sortSign,
} = useListSort<SortField>('name')
const filterText = ref('')
const filterTagIds = ref<number[]>([])
const groupMode = ref<GroupMode>('host')
const quotaFilter = ref<QuotaFilter>('all')
const showTagDropdown = ref(false)
const serverQuotasByHost = ref<Record<string, ServerQuotaResponse>>({})

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const allRepoTags = ref<TagRow[]>([])
const repoTagsMap = ref<Record<number, RepoTag[]>>({})

const repoDialog = ref<InstanceType<typeof RepoCreateDialog> | null>(null)
const showRepoDialog = ref(false)
const addTab = ref<AddTab>('import')

function repoOwnHealth(repo: RepoWithStats): ReturnType<typeof quotaHealth> {
  return quotaHealth(repo.quota, repo.total_deduplicated_size)
}

/** Utilization of a repo's own quota (usage / ceiling), or null when unconfigured. */
function repoQuotaUtilization(repo: RepoWithStats): number | null {
  if (!repo.quota?.enabled) return null
  const ceiling = quotaCeiling(repo.quota)
  if (ceiling === null || ceiling <= 0) return null
  return repo.total_deduplicated_size / ceiling
}

const atRiskCount = computed(
  () =>
    repos.value.filter((r) => {
      const health = repoOwnHealth(r)
      return health === 'warning' || health === 'critical'
    }).length,
)

const noQuotaCount = computed(
  () => repos.value.filter((r) => repoOwnHealth(r) === 'unconfigured').length,
)

const filteredRepos = computed<RepoWithStats[]>(() => {
  let list = [...repos.value]

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.ssh_host.toLowerCase().includes(q) ||
        r.repo_path.toLowerCase().includes(q),
    )
  }

  if (filterTagIds.value.length > 0) {
    const selectedNames = new Set(
      allRepoTags.value.filter((t) => filterTagIds.value.includes(t.id)).map((t) => t.name),
    )
    list = list.filter((r) =>
      (repoTagsMap.value[r.id] ?? []).some((t) => selectedNames.has(t.name)),
    )
  }

  if (quotaFilter.value === 'at_risk') {
    list = list.filter((r) => {
      const health = repoOwnHealth(r)
      return health === 'warning' || health === 'critical'
    })
  } else if (quotaFilter.value === 'no_quota') {
    list = list.filter((r) => repoOwnHealth(r) === 'unconfigured')
  }

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'name':
        cmp = a.name.localeCompare(b.name)
        break
      case 'size':
        cmp = a.total_deduplicated_size - b.total_deduplicated_size
        break
      case 'last_backup':
        cmp = (a.last_backup_at ?? '').localeCompare(b.last_backup_at ?? '')
        break
      case 'quota':
        cmp = (repoQuotaUtilization(a) ?? 0) - (repoQuotaUtilization(b) ?? 0)
        break
    }
    return cmp * sortSign()
  })

  if (sortField.value === 'quota') {
    const configured = list.filter((r) => repoQuotaUtilization(r) !== null)
    const unconfigured = list.filter((r) => repoQuotaUtilization(r) === null)
    list = [...configured, ...unconfigured]
  }

  return list
})

const hostGroups = computed<HostGroup[]>(() => {
  if (groupMode.value !== 'host') return []

  const byHost = new Map<string, RepoWithStats[]>()
  for (const repo of repos.value) {
    const existing = byHost.get(repo.ssh_host)
    if (existing) existing.push(repo)
    else byHost.set(repo.ssh_host, [repo])
  }

  const visibleIds = new Set(filteredRepos.value.map((r) => r.id))

  return [...byHost.entries()]
    .map(([sshHost, hostRepos]) => {
      const sorted = [...hostRepos].sort(
        (a, b) => b.total_deduplicated_size - a.total_deduplicated_size,
      )
      let offset = 0
      const entries: HostGroupEntry[] = sorted.map((repo, index) => {
        const entry: HostGroupEntry = {
          repo,
          offsetBytes: offset,
          visible: visibleIds.has(repo.id),
          colorStep: index,
        }
        offset += repo.total_deduplicated_size
        return entry
      })
      const serverQuota = serverQuotasByHost.value[sshHost] ?? null
      const boxMaxBytes = serverQuota?.configured
        ? (serverQuota.critical_bytes ?? serverQuota.warn_bytes)
        : null
      const first = hostRepos[0]!
      return {
        sshHost,
        repoHostId: first.repo_host.id,
        intermittent: first.repo_host.intermittent,
        entries,
        totalDeduplicated: offset,
        serverQuota,
        boxMaxBytes,
        visibleCount: entries.filter((e) => e.visible).length,
      }
    })
    .sort((a, b) => a.sshHost.localeCompare(b.sshHost))
})

function hostSegGeometry(entry: HostGroupEntry, group: HostGroup): { left: number; width: number } {
  const g = computeSliceGeometry({
    offsetBytes: entry.offsetBytes,
    usageBytes: entry.repo.total_deduplicated_size,
    boxMaxBytes: group.boxMaxBytes ?? 0,
  })
  return { left: g.leftPercent, width: g.fillWidthPercent }
}

function hostWarnMarkPercent(group: HostGroup): number | null {
  const warnBytes = group.serverQuota?.warn_bytes ?? null
  if (
    !group.boxMaxBytes ||
    warnBytes === null ||
    warnBytes <= 0 ||
    warnBytes >= group.boxMaxBytes
  ) {
    return null
  }
  return (warnBytes / group.boxMaxBytes) * 100
}

function hostPoolNote(group: HostGroup): string {
  const count = `${group.entries.length} repo${group.entries.length === 1 ? '' : 's'}`
  if (group.visibleCount === 0) return 'No matching repos'
  if (group.visibleCount < group.entries.length) {
    return `Showing ${group.visibleCount} of ${group.entries.length} repos`
  }
  if (!group.serverQuota?.configured) return `${count} · no host quota configured`

  const health = quotaHealth(group.serverQuota, group.totalDeduplicated)
  if (health === 'critical' || health === 'warning') {
    const action = actionForHealth(
      health,
      group.serverQuota.warn_action,
      group.serverQuota.critical_action,
    )
    const state = health === 'critical' ? 'over critical' : 'over warn threshold'
    return `${count} · ${state}${action ? ` · ${actionLabel(action)}` : ''}`
  }
  const warnBytes = group.serverQuota.warn_bytes
  if (warnBytes !== null && warnBytes > group.totalDeduplicated) {
    return `${count} · ${formatBytes(warnBytes - group.totalDeduplicated)} below warn`
  }
  return `${count} · healthy`
}

/** Grouping by tag is only offered once there is a tag to group by. */
const groupOptions = computed<SegmentedOption<GroupMode>[]>(() => [
  { value: 'host', label: 'Host' },
  ...(allRepoTags.value.length > 0 ? [{ value: 'tag' as const, label: 'Tag' }] : []),
  { value: 'none', label: 'None' },
])

const groupedRepos = computed<TagGroup[]>(() => {
  if (groupMode.value !== 'tag') return []
  const groups: Map<string, TagGroup> = new Map()
  const untagged: RepoWithStats[] = []

  for (const repo of filteredRepos.value) {
    const tags = repoTagsMap.value[repo.id]
    if (!tags || tags.length === 0) {
      untagged.push(repo)
    } else {
      for (const tag of tags) {
        const existing = groups.get(tag.name)
        if (existing) {
          existing.repos.push(repo)
        } else {
          groups.set(tag.name, { label: tag.name, color: tag.color, repos: [repo] })
        }
      }
    }
  }

  const result = [...groups.values()].sort((a, b) => a.label.localeCompare(b.label))
  if (untagged.length > 0) {
    result.push({ label: 'Untagged', color: null, repos: untagged })
  }
  return result
})

function toggleTagFilter(tagId: number): void {
  const idx = filterTagIds.value.indexOf(tagId)
  if (idx === -1) {
    filterTagIds.value = [...filterTagIds.value, tagId]
  } else {
    filterTagIds.value = filterTagIds.value.filter((id) => id !== tagId)
  }
}

const listGroups = computed<ListGroup[]>(() => {
  if (groupMode.value === 'host') {
    return hostGroups.value.map((group) => ({
      key: `host-${group.sshHost}`,
      host: group,
      title: group.sshHost,
      color: null,
      rows:
        group.visibleCount > 0
          ? group.entries.map((entry) => ({ repo: entry.repo, dim: !entry.visible }))
          : [],
    }))
  }
  if (groupMode.value === 'tag') {
    return groupedRepos.value.map((group) => ({
      key: `tag-${group.label}`,
      host: null,
      title: group.label,
      color: group.color,
      rows: group.repos.map((repo) => ({ repo, dim: false })),
    }))
  }
  return [
    {
      key: 'all',
      host: null,
      title: 'All repositories',
      color: null,
      rows: filteredRepos.value.map((repo) => ({ repo, dim: false })),
    },
  ]
})

async function loadRepos(): Promise<void> {
  await run(async () => {
    const [reposRes, repoTagAssoc, repoTagsRes, serverQuotasRes] = await Promise.all([
      listRepoStats(),
      listRepoTags().catch(() => [] as RepoTagEntryResponse[]),
      listTags('repo').catch(() => [] as TagRow[]),
      authStore.isAdmin
        ? listServerQuotas().catch(() => [] as ServerQuotaResponse[])
        : Promise.resolve([] as ServerQuotaResponse[]),
    ])
    repos.value = reposRes

    allRepoTags.value = repoTagsRes
    const tagMap: Record<number, RepoTag[]> = {}
    repoTagAssoc.forEach((rt) => {
      if (!tagMap[rt.repo_id]) tagMap[rt.repo_id] = []
      tagMap[rt.repo_id].push({ name: rt.tag_name, color: rt.tag_color })
    })
    repoTagsMap.value = tagMap

    const quotaMap: Record<string, ServerQuotaResponse> = {}
    serverQuotasRes.forEach((q) => {
      quotaMap[q.ssh_host] = q
    })
    serverQuotasByHost.value = quotaMap
  })
}

/** Opens the create/import dialog in `mode`, clearing whatever it last held. */
function openRepoDialog(mode: AddTab): void {
  addTab.value = mode
  repoDialog.value?.reset()
  showRepoDialog.value = true
}

/**
 * An import only enqueues the scan, so the repository is added to the list
 * straight away in its importing state rather than waiting for a refetch.
 */
function onRepoImported(created: Repo): void {
  repos.value = [
    ...repos.value,
    {
      id: created.id,
      name: created.name,
      repo_path: created.repo_path,
      ssh_user: created.ssh_user,
      ssh_host: created.ssh_host,
      ssh_port: created.ssh_port,
      ssh_host_key: null,
      compression: created.compression,
      encryption: created.encryption,
      enabled: created.enabled,
      importing: true,
      import_error: null,
      import_progress: 0,
      import_total: 0,
      import_status_message: null,
      archive_count: 0,
      last_backup_at: null,
      total_original_size: 0,
      total_compressed_size: 0,
      total_deduplicated_size: 0,
      agent_count: 0,
      unmatched_count: 0,
      visibility: 'private',
      owner_id: null,
      sync_schedule: null,
      last_synced_at: null,
      relocation_pending: false,
      last_op_kind: null,
      last_op_at: null,
      last_op_by: null,
      current_op: null,
      quota: null,
      // A repository joining a host already listed here inherits whether it
      // sleeps. Otherwise - a new host, or a known one none of whose
      // repositories are listed - it reads as always online until the
      // refresh the server broadcasts after every create brings the host's
      // own setting.
      repo_host: {
        id: created.repo_host_id,
        machine_id: created.machine_id,
        intermittent: repos.value.some(
          (r) => r.repo_host.id === created.repo_host_id && r.repo_host.intermittent,
        ),
      },
      power: created.power,
    },
  ]
}

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => loadRepos().catch(logger.error))

onMessage('ImportProgress', (payload) => {
  const repo = repos.value.find((r) => r.id === payload.repo_id)
  if (repo) {
    if (payload.progress >= 0) {
      repo.import_progress = payload.progress
      repo.import_total = payload.total
    }
    repo.import_status_message = payload.message
  }
})

onMounted(loadRepos)
</script>

<template>
  <div class="repos-view">
    <div class="page-header">
      <h1 class="page-title">Repositories</h1>
      <div
        v-if="authStore.isAdmin"
        class="header-actions"
      >
        <button
          class="btn btn-ghost"
          @click="() => openRepoDialog('import')"
        >
          <Download :size="14" />
          Import
        </button>
        <button
          class="btn btn-primary"
          @click="() => openRepoDialog('create')"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <div class="toolbar">
      <input
        v-model="filterText"
        class="input search-input"
        placeholder="Filter repositories..."
      />
      <button
        v-if="isMobile"
        class="filter-toggle"
        :class="{ active: filterTagIds.length > 0 || groupMode !== 'none' }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterTagIds.length > 0 || groupMode !== 'none'"
          class="filter-badge"
        ></span>
      </button>
      <template v-if="!isMobile || showMobileFilters">
        <div
          v-if="allRepoTags.length > 0"
          class="tag-filter-wrapper"
        >
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: filterTagIds.length > 0 }"
            @click="showTagDropdown = !showTagDropdown"
          >
            Tags{{ filterTagIds.length > 0 ? ` (${filterTagIds.length})` : '' }}
            <span class="dropdown-arrow">{{ showTagDropdown ? '\u25B4' : '\u25BE' }}</span>
          </button>
          <div
            v-if="showTagDropdown"
            class="tag-dropdown"
          >
            <label
              v-for="tag in allRepoTags"
              :key="tag.id"
              class="tag-dropdown-item"
            >
              <input
                type="checkbox"
                :checked="filterTagIds.includes(tag.id)"
                @change="toggleTagFilter(tag.id)"
              />
              <span
                class="tag-dot"
                :style="{ background: tag.color }"
              ></span>
              <span class="tag-dropdown-name">{{ tag.name }}</span>
            </label>
          </div>
        </div>
        <BaseSegmented
          v-model="groupMode"
          :options="groupOptions"
          label="Group by"
        />
        <SortControls
          :field="sortField"
          :direction="sortDir"
          :options="SORT_OPTIONS"
          @toggle="toggleSort"
        />
      </template>
    </div>

    <div
      v-if="!isMobile || showMobileFilters"
      class="filter-pills repo-filters"
      role="group"
      aria-label="Quota filter"
    >
      <button
        class="filter-pill"
        :class="{ active: quotaFilter === 'all' }"
        @click="quotaFilter = 'all'"
      >
        All &middot; {{ repos.length }}
      </button>
      <button
        class="filter-pill"
        :class="{ active: quotaFilter === 'at_risk' }"
        @click="quotaFilter = 'at_risk'"
      >
        <span class="status-dot status-dot--warning"></span>
        At risk &middot; {{ atRiskCount }}
      </button>
      <button
        class="filter-pill"
        :class="{ active: quotaFilter === 'no_quota' }"
        @click="quotaFilter = 'no_quota'"
      >
        <span class="status-dot"></span>
        No quota &middot; {{ noQuotaCount }}
      </button>
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
      v-else-if="repos.length === 0"
      :icon="Database"
      title="No repositories configured"
      description="Add a repository to start managing backups."
      action="New repository"
      @action="() => openRepoDialog('create')"
    />
    <div
      v-else-if="filteredRepos.length === 0 && groupMode !== 'host'"
      class="state-msg"
    >
      No repositories match the current filter.
    </div>

    <div
      v-else
      class="list-stack repos-list"
    >
      <section
        v-for="group in listGroups"
        :key="group.key"
        class="list-group"
      >
        <header
          v-if="group.host"
          class="list-group-header pool-header"
          :class="{ 'pool-header-empty': group.host.visibleCount === 0 }"
        >
          <span class="pool-host">
            <h2 class="list-group-title">
              <RouterLink
                v-if="authStore.isAdmin"
                :to="`/repo-hosts/${group.host.repoHostId}`"
              >
                {{ group.host.sshHost }}
              </RouterLink>
              <template v-else>{{ group.host.sshHost }}</template>
            </h2>
            <span
              v-if="group.host.intermittent"
              class="badge badge--warning"
            >
              Not always online
            </span>
          </span>
          <span class="list-group-count pool-note">{{ hostPoolNote(group.host) }}</span>
          <span class="list-group-aside pool-total">
            {{ formatBytes(group.host.totalDeduplicated) }}
            <template v-if="group.host.boxMaxBytes">
              / {{ formatBytes(group.host.boxMaxBytes) }}
            </template>
          </span>
          <div
            v-if="group.host.boxMaxBytes"
            class="pool-track"
            role="img"
            :aria-label="`${formatBytes(group.host.totalDeduplicated)} used of ${formatBytes(group.host.boxMaxBytes)} across ${group.host.entries.length} repositories`"
          >
            <span
              v-for="entry in group.host.entries"
              :key="entry.repo.id"
              class="pool-seg"
              :class="[`pool-seg-step-${entry.colorStep % 2}`, { 'pool-seg-dim': !entry.visible }]"
              :style="{
                left: `${hostSegGeometry(entry, group.host).left}%`,
                width: `${hostSegGeometry(entry, group.host).width}%`,
              }"
            ></span>
            <span
              v-if="hostWarnMarkPercent(group.host) !== null"
              class="pool-mark"
              :style="{ left: `${hostWarnMarkPercent(group.host)}%` }"
            ></span>
          </div>
        </header>
        <header
          v-else
          class="list-group-header"
        >
          <span
            v-if="group.color"
            class="tag-group-dot"
            :style="{ background: group.color }"
          ></span>
          <h2 class="list-group-title">{{ group.title }}</h2>
          <span class="list-group-count">{{ group.rows.length }}</span>
        </header>

        <template v-if="group.rows.length > 0">
          <div class="list-cols">
            <span>Repository</span>
            <span>Status</span>
            <span>Archives</span>
            <span>Last backup</span>
            <span>Size of quota</span>
            <span></span>
          </div>
          <RepoListRow
            v-for="row in group.rows"
            :key="`${group.key}-${row.repo.id}`"
            :repo="row.repo"
            :dim="row.dim"
            :pooled="group.host !== null"
          />
        </template>
      </section>
    </div>

    <RepoCreateDialog
      ref="repoDialog"
      :open="showRepoDialog"
      :mode="addTab"
      :repos="repos"
      @close="showRepoDialog = false"
      @imported="onRepoImported"
      @created="loadRepos"
    />
  </div>
</template>

<style scoped>
.repos-view {
  max-width: 1100px;
}

.repo-filters {
  margin-top: var(--space-4);
  margin-bottom: var(--space-8);
}

.repos-list .list-group {
  --list-cols: minmax(0, 2.6fr) minmax(0, 1.3fr) 4.5rem 6.5rem minmax(0, 1.6fr) 1rem;
}

.tag-group-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}

/* Host pool header: the host and its badges, the pool note, the usage at the
   right, and the pool bar across the full width underneath. */
.pool-header-empty {
  opacity: 0.55;
}

.pool-host {
  display: inline-flex;
  align-items: center;
  gap: var(--space-4);
  flex-wrap: wrap;
  min-width: 0;
}

.pool-host .list-group-title {
  font-family: var(--mono);
}

.pool-total {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
  white-space: nowrap;
}

.pool-track {
  position: relative;
  flex-basis: 100%;
  height: 8px;
  border-radius: var(--radius-pill);
  background: var(--border);
}

.pool-seg {
  position: absolute;
  top: 0;
  height: 100%;
  border-radius: var(--radius-pill);
}

.pool-seg-step-0 {
  background: var(--warning);
}

.pool-seg-step-1 {
  background: color-mix(in oklab, var(--warning) 62%, var(--bg-card));
}

.pool-seg-dim {
  opacity: 0.4;
}

.pool-mark {
  position: absolute;
  top: -2px;
  bottom: -2px;
  width: 2px;
  border-radius: var(--radius-pill);
  background: var(--text-secondary);
}

@media (max-width: 640px) {
  /* Name and usage on the first line, the bar, then the summary under it. */
  .pool-note {
    order: 1;
    flex-basis: 100%;
  }
}
</style>

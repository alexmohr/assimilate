<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useMobile } from '../composables/useMobile'
import { useListSort } from '../composables/useListSort'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import { formatBytes, relativeTime } from '../utils/format'
import { useAsyncAction } from '../composables/useAsyncAction'
import { Plus, Download, SlidersHorizontal, Database } from '@lucide/vue'
import RepoCreateDialog from '../components/RepoCreateDialog.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
import RepoQuotaMeter from '../components/RepoQuotaMeter.vue'
import RepoQuotaSlice from '../components/RepoQuotaSlice.vue'
import type { Repo, RepoWithStats } from '../types/repo'
import type { TagRow } from '../types/tag'
import type { ServerQuotaResponse } from '../types/generated'
import { listServerQuotas } from '../api/serverQuotas'
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

interface RepoTagRow {
  repo_id: number
  tag_name: string
  tag_color: string
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
  entries: HostGroupEntry[]
  totalDeduplicated: number
  serverQuota: ServerQuotaResponse | null
  boxMaxBytes: number | null
  visibleCount: number
}

const router = useRouter()
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
const groupByTag = ref(false)
const groupByHost = ref(true)
const quotaFilter = ref<QuotaFilter>('all')
const showTagDropdown = ref(false)
const serverQuotasByHost = ref<Record<string, ServerQuotaResponse>>({})

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const allRepoTags = ref<TagRow[]>([])
const repoTagsMap = ref<Record<number, { name: string; color: string }[]>>({})

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
  if (!groupByHost.value) return []

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
      return {
        sshHost,
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

function toggleGroupByTag(): void {
  groupByTag.value = !groupByTag.value
  if (groupByTag.value) groupByHost.value = false
}

function toggleGroupByHost(): void {
  groupByHost.value = !groupByHost.value
  if (groupByHost.value) groupByTag.value = false
}

const groupedRepos = computed<TagGroup[]>(() => {
  if (!groupByTag.value) return []
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

function repoTags(repo: RepoWithStats): { name: string; color: string }[] {
  return repoTagsMap.value[repo.id] ?? []
}

function repoImportPhaseVerb(repo: RepoWithStats): string {
  return (repo.import_status_message ?? '').startsWith('Indexing') ? 'Indexing' : 'Importing'
}

async function loadRepos(): Promise<void> {
  await run(async () => {
    const [reposRes, repoTagAssocRes, repoTagsRes, serverQuotasRes] = await Promise.all([
      apiClient.get<RepoWithStats[]>('/repos/stats'),
      apiClient.get<RepoTagRow[]>('/repo-tags').catch(() => ({ data: [] as RepoTagRow[] })),
      apiClient
        .get<TagRow[]>('/tags', { params: { scope: 'repo' } })
        .catch(() => ({ data: [] as TagRow[] })),
      authStore.isAdmin
        ? listServerQuotas().catch(() => [] as ServerQuotaResponse[])
        : Promise.resolve([] as ServerQuotaResponse[]),
    ])
    repos.value = reposRes.data

    allRepoTags.value = repoTagsRes.data
    const tagMap: Record<number, { name: string; color: string }[]> = {}
    repoTagAssocRes.data.forEach((rt) => {
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

function navigateToRepo(repo: RepoWithStats): void {
  router.push(`/repos/${repo.id}`)
}

function navigateToRepoIssue(repo: RepoWithStats): void {
  router.push(`/repos/${repo.id}?tab=archives`)
}

function repoIssues(repo: RepoWithStats): EntityIssue[] {
  if (repo.unmatched_count <= 0) return []
  return [
    {
      key: 'unmatched',
      label: `${repo.unmatched_count} unmatched`,
      severity: 'warning',
      onClick: () => navigateToRepoIssue(repo),
    },
  ]
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
        :class="{ active: filterTagIds.length > 0 || groupByTag || groupByHost }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterTagIds.length > 0 || groupByTag || groupByHost"
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
        <button
          v-if="allRepoTags.length > 0"
          class="btn btn-sm btn-ghost"
          :class="{ active: groupByTag }"
          @click="toggleGroupByTag"
        >
          Group by tag
        </button>
        <button
          class="btn btn-sm btn-ghost"
          :class="{ active: groupByHost }"
          @click="toggleGroupByHost"
        >
          Group by host
        </button>
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
      class="quota-filter-row"
    >
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'all' }"
        @click="quotaFilter = 'all'"
      >
        All &middot; {{ repos.length }}
      </button>
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'at_risk' }"
        @click="quotaFilter = 'at_risk'"
      >
        <span class="quota-fchip-dot quota-fchip-dot-warn"></span>
        At risk &middot; {{ atRiskCount }}
      </button>
      <button
        class="quota-fchip"
        :class="{ active: quotaFilter === 'no_quota' }"
        @click="quotaFilter = 'no_quota'"
      >
        <span class="quota-fchip-dot quota-fchip-dot-none"></span>
        No quota &middot; {{ noQuotaCount }}
      </button>
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="state-msg state-error"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="repos.length === 0"
      :icon="Database"
      title="No repositories configured"
      description="Add a repository to start managing backups."
      action="Add repository"
      @action="showRepoDialog = true"
    />
    <div
      v-else-if="filteredRepos.length === 0 && !groupByHost"
      class="state-msg"
    >
      No repositories match the current filter.
    </div>

    <div
      v-else-if="groupByHost"
      class="repo-hostgrouped"
    >
      <div
        v-for="group in hostGroups"
        :key="group.sshHost"
        class="host-group"
      >
        <div
          class="pool-header"
          :class="{ 'pool-header-empty': group.visibleCount === 0 }"
        >
          <div class="pool-top">
            <span class="pool-host">{{ group.sshHost }}</span>
            <span class="pool-total">
              {{ formatBytes(group.totalDeduplicated) }}
              <template v-if="group.boxMaxBytes"> / {{ formatBytes(group.boxMaxBytes) }}</template>
            </span>
          </div>
          <div
            v-if="group.boxMaxBytes"
            class="pool-track"
            role="img"
            :aria-label="`${formatBytes(group.totalDeduplicated)} used of ${formatBytes(group.boxMaxBytes)} across ${group.entries.length} repositories`"
          >
            <span
              v-for="entry in group.entries"
              :key="entry.repo.id"
              class="pool-seg"
              :class="[`pool-seg-step-${entry.colorStep % 2}`, { 'pool-seg-dim': !entry.visible }]"
              :style="{
                left: `${hostSegGeometry(entry, group).left}%`,
                width: `${hostSegGeometry(entry, group).width}%`,
              }"
            ></span>
            <span
              v-if="hostWarnMarkPercent(group) !== null"
              class="pool-mark"
              :style="{ left: `${hostWarnMarkPercent(group)}%` }"
            ></span>
          </div>
          <span class="pool-note">{{ hostPoolNote(group) }}</span>
        </div>

        <div
          v-if="group.visibleCount > 0"
          class="card-grid"
        >
          <div
            v-for="entry in group.entries"
            :key="entry.repo.id"
            class="entity-card"
            :class="{
              'entity-card--notable': !entry.repo.enabled,
              'entity-card--dim': !entry.visible,
            }"
            @click="navigateToRepo(entry.repo)"
          >
            <div class="card-top">
              <div class="card-info">
                <span class="card-name">{{ entry.repo.name }}</span>
                <span class="card-ssh"
                  >{{ entry.repo.ssh_user }}@{{ entry.repo.ssh_host }}:{{
                    entry.repo.ssh_port
                  }}</span
                >
              </div>
              <div class="card-badges">
                <span
                  v-if="entry.repo.import_error || entry.repo.importing"
                  class="badge"
                  :class="entry.repo.import_error ? 'badge--danger' : 'badge--warning badge--pulse'"
                  :title="entry.repo.import_error ?? undefined"
                >
                  {{
                    entry.repo.import_error
                      ? 'Import Failed'
                      : entry.repo.import_total > 0
                        ? `${repoImportPhaseVerb(entry.repo)} ${entry.repo.import_progress}/${entry.repo.import_total}`
                        : `${repoImportPhaseVerb(entry.repo)}\u2026`
                  }}
                </span>
              </div>
            </div>
            <EntityStatusBadges
              :notable="!entry.repo.enabled"
              notable-label="Disabled"
              :issues="repoIssues(entry.repo)"
            />
            <div class="card-meta">
              <span class="meta-pill">{{ entry.repo.encryption }}</span>
              <span class="meta-pill">{{ entry.repo.compression }}</span>
            </div>
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{ entry.repo.archive_count }}</span>
                <span class="stat-label">Archives</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ relativeTime(entry.repo.last_backup_at ?? '') }}</span>
                <span class="stat-label">Last backup</span>
              </div>
            </div>
            <RepoQuotaMeter
              v-if="!group.boxMaxBytes || repoQuotaUtilization(entry.repo) !== null"
              :quota="entry.repo.quota"
              :usage-bytes="entry.repo.total_deduplicated_size"
            />
            <RepoQuotaSlice
              v-else
              :usage-bytes="entry.repo.total_deduplicated_size"
              :offset-bytes="entry.offsetBytes"
              :box-max-bytes="group.boxMaxBytes"
              :color-step="entry.colorStep"
            />
          </div>
        </div>
      </div>
    </div>

    <div
      v-else-if="!groupByTag"
      class="card-grid"
    >
      <div
        v-for="repo in filteredRepos"
        :key="repo.id"
        class="entity-card"
        :class="{ 'entity-card--notable': !repo.enabled }"
        @click="navigateToRepo(repo)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-name">{{ repo.name }}</span>
            <span class="card-ssh"
              >{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</span
            >
          </div>
          <div class="card-badges">
            <span
              v-if="repo.import_error || repo.importing"
              class="badge"
              :class="repo.import_error ? 'badge--danger' : 'badge--warning badge--pulse'"
              :title="repo.import_error ?? undefined"
            >
              {{
                repo.import_error
                  ? 'Import Failed'
                  : repo.import_total > 0
                    ? `${repoImportPhaseVerb(repo)} ${repo.import_progress}/${repo.import_total}`
                    : `${repoImportPhaseVerb(repo)}\u2026`
              }}
            </span>
          </div>
        </div>
        <div
          v-if="repo.importing && repo.import_total > 0"
          class="progress-row"
        >
          <div class="progress-track">
            <div
              class="progress-bar"
              :style="{ width: `${Math.round((repo.import_progress / repo.import_total) * 100)}%` }"
            ></div>
          </div>
          <span class="progress-label">
            {{ Math.round((repo.import_progress / repo.import_total) * 100) }}%
          </span>
        </div>
        <p
          v-if="repo.importing && repo.import_status_message"
          class="import-status-inline"
        >
          {{ repo.import_status_message }}
        </p>
        <EntityStatusBadges
          :notable="!repo.enabled"
          notable-label="Disabled"
          :issues="repoIssues(repo)"
        />
        <div class="card-meta">
          <span class="meta-pill">{{ repo.encryption }}</span>
          <span class="meta-pill">{{ repo.compression }}</span>
          <span
            v-for="tag in repoTags(repo)"
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
        </div>
        <div class="card-stats">
          <div class="stat">
            <span class="stat-value">{{ repo.archive_count }}</span>
            <span class="stat-label">Archives</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
            <span class="stat-label">Deduplicated</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ relativeTime(repo.last_backup_at ?? '') }}</span>
            <span class="stat-label">Last backup</span>
          </div>
        </div>
        <RepoQuotaMeter
          :quota="repo.quota"
          :usage-bytes="repo.total_deduplicated_size"
        />
      </div>
    </div>

    <div
      v-else
      class="repo-grouped"
    >
      <div
        v-for="group in groupedRepos"
        :key="group.label"
        class="tag-group"
      >
        <div class="tag-group-header">
          <span
            v-if="group.color"
            class="tag-group-dot"
            :style="{ background: group.color }"
          ></span>
          <h3 class="tag-group-title">{{ group.label }}</h3>
          <span class="tag-group-count">{{ group.repos.length }}</span>
        </div>
        <div class="card-grid">
          <div
            v-for="repo in group.repos"
            :key="`${group.label}-${repo.id}`"
            class="entity-card"
            :class="{ 'entity-card--notable': !repo.enabled }"
            @click="navigateToRepo(repo)"
          >
            <div class="card-top">
              <div class="card-info">
                <span class="card-name">{{ repo.name }}</span>
                <span class="card-ssh"
                  >{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</span
                >
              </div>
              <div class="card-badges">
                <span
                  v-if="repo.import_error || repo.importing"
                  class="badge"
                  :class="repo.import_error ? 'badge--danger' : 'badge--warning badge--pulse'"
                  :title="repo.import_error ?? undefined"
                >
                  {{
                    repo.import_error
                      ? 'Import Failed'
                      : repo.import_total > 0
                        ? `${repoImportPhaseVerb(repo)} ${repo.import_progress}/${repo.import_total}`
                        : `${repoImportPhaseVerb(repo)}\u2026`
                  }}
                </span>
              </div>
            </div>
            <div
              v-if="repo.importing && repo.import_total > 0"
              class="progress-row"
            >
              <div class="progress-track">
                <div
                  class="progress-bar"
                  :style="{
                    width: `${Math.round((repo.import_progress / repo.import_total) * 100)}%`,
                  }"
                ></div>
              </div>
              <span class="progress-label">
                {{ Math.round((repo.import_progress / repo.import_total) * 100) }}%
              </span>
            </div>
            <p
              v-if="repo.importing && repo.import_status_message"
              class="import-status-inline"
            >
              {{ repo.import_status_message }}
            </p>
            <EntityStatusBadges
              :notable="!repo.enabled"
              notable-label="Disabled"
              :issues="repoIssues(repo)"
            />
            <div class="card-meta">
              <span class="meta-pill">{{ repo.encryption }}</span>
              <span class="meta-pill">{{ repo.compression }}</span>
              <span
                v-for="tag in repoTags(repo)"
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
            </div>
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{ repo.archive_count }}</span>
                <span class="stat-label">Archives</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
                <span class="stat-label">Deduplicated</span>
              </div>
              <div class="stat">
                <span class="stat-value">{{ relativeTime(repo.last_backup_at ?? '') }}</span>
                <span class="stat-label">Last backup</span>
              </div>
            </div>
            <RepoQuotaMeter
              :quota="repo.quota"
              :usage-bytes="repo.total_deduplicated_size"
            />
          </div>
        </div>
      </div>
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

.import-status-inline {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-badges {
  display: flex;
  gap: 0.35rem;
  align-items: center;
  flex-shrink: 0;
}

.card-ssh {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  font-family: var(--mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Tag filter dropdown */

/* Tag pills on cards */

/* Grouped view */
.repo-grouped {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.tag-group-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.tag-group-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tag-group-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-primary);
}

.tag-group-count {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  background: var(--bg-hover);
  padding: 0.1rem 0.4rem;
  border-radius: var(--radius-pill);
}

/* Quota filter chips */
.quota-filter-row {
  display: flex;
  gap: 0.4rem;
  flex-wrap: wrap;
  margin-top: 0.5rem;
}

.quota-fchip {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  font-size: var(--fs-xs);
  padding: 0.25rem 0.65rem;
  border-radius: var(--radius-pill);
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-secondary);
  cursor: pointer;
}

.quota-fchip.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-subtle);
}

.quota-fchip-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.quota-fchip-dot-warn {
  background: var(--warning);
}

.quota-fchip-dot-none {
  background: var(--text-muted);
}

/* Grouped by host */
.repo-hostgrouped {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.host-group {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  padding: 0.75rem;
  border-radius: var(--radius);
  background: var(--bg-hover);
}

.pool-header {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.85rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.pool-header-empty {
  opacity: 0.55;
}

.pool-top {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.5rem;
}

.pool-host {
  font-family: var(--mono);
  font-size: var(--fs-sm);
  color: var(--text-primary);
  font-weight: 600;
}

.pool-total {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
  flex-shrink: 0;
}

.pool-track {
  position: relative;
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

.pool-note {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>

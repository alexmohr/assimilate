<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { getRepo } from '../api/repos'
import { useAuthStore } from '../stores/auth'
import { useWebSocket } from '../composables/useWebSocket'
import { useArchiveDeletionEvents } from '../composables/useArchiveDeletionEvents'

import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import { formatBytes, relativeTime } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import BaseSpinner from '../components/BaseSpinner.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'
import BaseTabs from '../components/BaseTabs.vue'
import RepoSchedulesTab from '../components/RepoSchedulesTab.vue'
import RepoArchivesTab from '../components/RepoArchivesTab.vue'
import RepoHeader from '../components/RepoHeader.vue'
import RepoSettingsTab from '../components/RepoSettingsTab.vue'
import { isRepoSettingsSection, type RepoSettingsSection } from '../utils/repoSettings'

type TabId = 'overview' | 'archives' | 'schedules' | 'settings'

const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const repoId = computed(() => Number(props.id))

const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab as string | undefined
    if (t === 'archives' || t === 'schedules' || t === 'settings') return t
    return 'overview'
  },
  set(val: TabId) {
    router.replace({ query: { ...route.query, tab: val } })
  },
})

const tabs: { id: TabId; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'archives', label: 'Archives' },
  { id: 'schedules', label: 'Schedules' },
  { id: 'settings', label: 'Settings' },
]

/** Which settings section the Settings tab shows, kept in the URL so a link
    can point at one. Same shape as the agent detail view. */
const settingsSection = computed<RepoSettingsSection>({
  get() {
    const s = route.query.section
    return isRepoSettingsSection(s) ? s : 'repository'
  },
  set(val: RepoSettingsSection) {
    router.replace({ query: { ...route.query, section: val } })
  },
})

const repo = ref<RepoWithStats | null>(null)
const { loading, error, run } = useAsyncAction()

const currentOp = ref<ActiveRepoOp | null>(null)

// Re-scan

const { onMessage } = useWebSocket()

onMessage('DataChanged', () => {
  refreshRepo().catch(logger.error)
})

onMessage('ImportProgress', (payload) => {
  if (repo.value && repo.value.id === payload.repo_id) {
    if (payload.progress >= 0) {
      repo.value.import_progress = payload.progress
      repo.value.import_total = payload.total
    }
    repo.value.import_status_message = payload.message
    if (payload.message !== null) {
      repo.value.importing = true
    } else {
      repo.value.importing = false
    }
  }
})

onMessage('RepoOpChanged', (payload) => {
  if (repo.value && payload.repo_id === repo.value.id) {
    currentOp.value = payload.op
  }
})

// When navigating from an agent backup tab with ?archive=xxx, filter the list
// to show only that archive instead of every archive in the repo. This is a
// route concern, so it stays with the view that owns the route.
const archiveFilterName = computed<string | null>(() => {
  const a = route.query.archive
  return typeof a === 'string' && a.length > 0 ? a : null
})

function clearArchiveFilter(): void {
  const query = { ...route.query }
  delete query.archive
  void router.replace({ query })
}

// During a full resync the same import progress channel carries the content
// indexing phase; surface it with its own label rather than "Importing".
const isIndexingPhase = computed(() =>
  (repo.value?.import_status_message ?? '').startsWith('Indexing'),
)
const importPhaseVerb = computed(() => (isIndexingPhase.value ? 'Indexing' : 'Importing'))

// The archives tab owns the archive list, its filtering and its deletion
// state; the view only points the shared subscription at it.
const archivesTab = ref<InstanceType<typeof RepoArchivesTab> | null>(null)

useArchiveDeletionEvents({
  target: () => archivesTab.value,
  repoId: () => repoId.value,
  reload: () => archivesTab.value?.loadArchives(true) ?? Promise.resolve(),
})

const isAdmin = computed(() => authStore.isAdmin)

/** The disk-sync cadence in words, for the Overview tile. */
const syncSummary = computed(() =>
  repo.value?.sync_schedule
    ? (cronToHuman(repo.value.sync_schedule) ?? repo.value.sync_schedule)
    : 'Disabled',
)

async function loadRepo(): Promise<void> {
  await run(async () => {
    const data = await getRepo(repoId.value)
    repo.value = data
    currentOp.value = data.current_op ?? null
  })
}

async function refreshRepo(): Promise<void> {
  try {
    const data = await getRepo(repoId.value)
    repo.value = data
    currentOp.value = data.current_op ?? null
  } catch (e: unknown) {
    logger.error('background repo refresh failed', e)
  }
}

watch(
  () => props.id,
  async () => {
    repo.value = null
    error.value = null
    await loadRepo()
  },
)

onMounted(async () => {
  await loadRepo()
})
</script>

<template>
  <div class="repo-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/repos"
        class="crumb-link"
      >
        Repositories
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ repo?.name ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !repo"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="repo">
      <!--
        The danger zone reports its failures up here rather than owning a
        banner. Without this the message was only rendered when the repository
        itself had failed to load, so a failed lock break or relocation
        confirm was set and then shown nowhere.
      -->
      <div
        v-if="error"
        class="error-banner"
      >
        {{ error }}
      </div>

      <RepoHeader
        :repo="repo"
        :is-admin="isAdmin"
        :import-phase-verb="importPhaseVerb"
        @import-reset="refreshRepo"
      />

      <BaseTabs
        v-model="activeTab"
        :tabs="tabs"
        label="Repository sections"
      />

      <!-- Overview Tab -->
      <div
        v-if="activeTab === 'overview'"
        class="tab-content fade-in"
      >
        <div class="tiles">
          <div class="tile">
            <span class="stat-label">Archives</span>
            <span class="stat-value stat-value--lg">{{ repo.archive_count }}</span>
            <span class="stat-sub">across {{ repo.agent_count }} agents</span>
          </div>
          <div class="tile">
            <span class="stat-label">Deduplicated</span>
            <span class="stat-value stat-value--lg">
              {{ formatBytes(repo.total_deduplicated_size) }}
            </span>
            <span class="stat-sub"> of {{ formatBytes(repo.total_original_size) }} original </span>
          </div>
          <div class="tile">
            <span class="stat-label">Last write</span>
            <span class="stat-value stat-value--lg">
              {{ relativeTime(repo.last_backup_at ?? '') }}
            </span>
            <span class="stat-sub"> {{ formatBytes(repo.total_compressed_size) }} compressed </span>
          </div>
          <div class="tile">
            <span class="stat-label">Disk sync</span>
            <span class="stat-value stat-value--lg">{{ syncSummary }}</span>
            <span class="stat-sub">
              last {{ repo.last_synced_at ? relativeTime(repo.last_synced_at) : 'never' }}
            </span>
          </div>
        </div>

        <section>
          <div class="section-head">
            <h2 class="section-title">Schedules</h2>
            <button
              class="btn btn-sm btn-ghost section-link"
              type="button"
              @click="activeTab = 'schedules'"
            >
              View all
            </button>
          </div>
          <RepoSchedulesTab :repo-id="repoId" />
        </section>
      </div>

      <!-- Archives Tab -->
      <div
        v-if="activeTab === 'archives'"
        class="tab-content fade-in"
      >
        <RepoArchivesTab
          ref="archivesTab"
          :repo-id="repoId"
          :repo-name="repo.name"
          :is-admin="isAdmin"
          :filter-name="archiveFilterName"
          :refresh-repo="refreshRepo"
          @clear-filter="clearArchiveFilter"
        />
      </div>

      <!-- Schedules Tab. Self-loads when the tab is first opened. -->
      <div
        v-if="activeTab === 'schedules'"
        class="tab-content fade-in"
      >
        <RepoSchedulesTab :repo-id="repoId" />
      </div>

      <!-- Settings Tab -->
      <div
        v-if="activeTab === 'settings'"
        class="tab-content fade-in"
      >
        <RepoSettingsTab
          v-model:section="settingsSection"
          :repo="repo"
          :is-admin="isAdmin"
          :current-op="currentOp"
          @changed="refreshRepo"
          @error="error = $event"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.repo-detail {
  max-width: 1200px;
}

/* States */

/* Tab bar */
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { useWebSocket } from '../composables/useWebSocket'

import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import BaseSpinner from '../components/BaseSpinner.vue'
import QuotaPanel from '../components/QuotaPanel.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'
import BaseTabs from '../components/BaseTabs.vue'
import RepoSchedulesTab from '../components/RepoSchedulesTab.vue'
import RepoBorgConsole from '../components/RepoBorgConsole.vue'
import RepoArchivesTab from '../components/RepoArchivesTab.vue'
import RepoDangerZone from '../components/RepoDangerZone.vue'
import EntityTags from '../components/EntityTags.vue'
import RepoOverviewCard from '../components/RepoOverviewCard.vue'

type TabId = 'overview' | 'archives' | 'schedules'

const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const repoId = computed(() => Number(props.id))

const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab as string | undefined
    if (t === 'archives' || t === 'schedules') return t
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
]

const repo = ref<RepoWithStats | null>(null)
const { loading, error, run } = useAsyncAction()

const currentOp = ref<ActiveRepoOp | null>(null)

// Re-scan

const { onMessage } = useWebSocket()

onMessage('ArchiveDeleted', (payload) => {
  if (payload.repo_id === repoId.value) {
    archivesTab.value?.onArchiveDeleted(payload.archive_name)
  }
})

onMessage('DataChanged', () => {
  refreshRepo().catch(logger.error)
  // Silent: this refresh runs in the background on every DataChanged event,
  // not just ones the user triggered - blanking the whole list to a loading
  // placeholder while it's in flight would hide unrelated UI state (like an
  // in-progress delete's row) for no reason.
  archivesTab.value
    ?.loadArchives(true)
    .then(() => archivesTab.value?.onDataChanged())
    .catch(logger.error)
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
    // Once this repo's active operation is no longer a delete or the
    // compact that automatically follows it, every archive delete queued
    // for it has finished - success or failure - since repo operations run
    // strictly one at a time. Any name still marked "deleting" at that
    // point is stale (a failed delete that the DataChanged-driven prune
    // above never saw disappear from the list), so it needs sweeping
    // rather than leaving its row disabled forever - but clearing
    // immediately, against whatever `sortedArchives` happens to hold right
    // now, raced that same DataChanged-driven refresh for a delete that in
    // fact just succeeded: this event and DataChanged can arrive in either
    // order, so "not yet in the refreshed list" and "genuinely still there"
    // were indistinguishable without a fetch of our own. Refetching first
    // (silently, so the whole panel doesn't flash a loading placeholder)
    // resolves that: by the time it returns, the delete has already
    // concluded (the op queue was already idle), so the list is
    // authoritative either way - present means genuinely stale/failed,
    // absent means already gone - and it's always correct to clear. Still
    // clears even if the reload fails, so a marker can never get stuck
    // forever.
    //
    // Only the names marked deleting *at the moment this event arrived* are
    // swept, not whatever the set happens to hold once the refetch above
    // resolves: a delete for a different, unrelated archive can be started
    // by the user while that refetch is still in flight, and clearing
    // unconditionally would wipe its just-set marker too, even though it
    // has nothing to do with the op queue draining that triggered this
    // event.
    if (payload.op?.kind !== 'delete_archive' && payload.op?.kind !== 'compact_repo') {
      archivesTab.value?.onRepoIdle()
    }
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
// state; the view only forwards the WebSocket events that touch them.
const archivesTab = ref<InstanceType<typeof RepoArchivesTab> | null>(null)

const isAdmin = computed(() => authStore.isAdmin)

async function loadRepo(): Promise<void> {
  await run(async () => {
    const res = await apiClient.get<RepoWithStats>(`/repos/${repoId.value}`)
    repo.value = res.data
    currentOp.value = res.data.current_op ?? null
  })
}

async function refreshRepo(): Promise<void> {
  try {
    const res = await apiClient.get<RepoWithStats>(`/repos/${repoId.value}`)
    repo.value = res.data
    currentOp.value = res.data.current_op ?? null
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
      class="state-msg state-error"
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
        class="state-msg state-error"
      >
        {{ error }}
      </div>
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
        <RepoOverviewCard
          :repo="repo"
          :is-admin="isAdmin"
          :current-op="currentOp"
          :import-phase-verb="importPhaseVerb"
          @saved="refreshRepo"
          @import-reset="refreshRepo"
        />

        <!-- Tags -->
        <QuotaPanel
          :repo-id="repoId"
          :is-admin="isAdmin"
          :current-usage-bytes="repo.total_deduplicated_size"
        />

        <EntityTags
          v-if="isAdmin"
          scope="repo"
          :entity-path="`/repos/${repoId}`"
        />

        <RepoBorgConsole
          v-if="isAdmin"
          :repo-id="repoId"
        />

        <RepoDangerZone
          v-if="isAdmin"
          :repo="repo"
          :current-op="currentOp"
          @error="error = $event"
          @changed="refreshRepo"
        />
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

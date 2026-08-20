<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import { AlertTriangle } from '@lucide/vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import { useArchiveBrowser, type ArchiveEntry } from '../composables/useArchiveBrowser'
import ArchiveExplorer from './ArchiveExplorer.vue'

const props = defineProps<{
  repoId: number
  repoName: string
  isAdmin: boolean
  /** Set from the ?archive= route query; shows only that archive. */
  filterName: string | null
  /** Re-fetches the repository row after a delete changes its stats. */
  refreshRepo: () => Promise<unknown>
}>()

const emit = defineEmits<{ 'clear-filter': [] }>()

const { success: toastSuccess, error: toastError } = useToast()

const repoIdRef = computed(() => props.repoId)

const { archives, sortedArchives, archivesLoading, archivesError, loadArchives } =
  useArchiveBrowser(repoIdRef)

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

const selectedArchive = ref<ArchiveEntry | null>(null)

const unmatched = computed(() => sortedArchives.value.filter((a) => a.matched !== true))
const unmatchedCount = computed(() => unmatched.value.length)
const unmatchedHostnames = computed(() => [...new Set(unmatched.value.map((a) => a.hostname))])

const archiveFilterName = computed(() => props.filterName)
const hasArchiveFilter = computed(() => props.filterName !== null)

function clearArchiveFilter(): void {
  emit('clear-filter')
}

function selectArchiveFromQuery(): void {
  const name = archiveFilterName.value
  if (!name) return
  const match = sortedArchives.value.find((a) => a.name === name)
  if (match) selectedArchive.value = match
}

const rescanning = ref(false)

async function rescan(): Promise<void> {
  rescanning.value = true
  try {
    const res = await apiClient.post<{ matched: number; remaining_unmatched: number }>(
      `/repos/${props.repoId}/rescan`,
    )
    toastSuccess(
      `Matched ${res.data.matched} archives. ${res.data.remaining_unmatched} remaining unmatched.`,
    )
    await loadArchives()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    rescanning.value = false
  }
}

watch(() => props.filterName, selectArchiveFromQuery)

// `immediate` because this component is mounted lazily, when the user opens
// the Archives tab: without it nothing fetches the list on that first mount
// and the tab sits empty until an unrelated DataChanged event happens to
// arrive. The repository can also change under a mounted tab, which is the
// same work, so both cases share one path.
watch(
  () => props.repoId,
  async () => {
    explorer.value?.resetList()
    selectedArchive.value = null
    await loadArchives()
    selectArchiveFromQuery()
  },
  { immediate: true },
)

/**
 * The parent owns the WebSocket subscription, so it forwards the three events
 * that touch archive state here rather than this component opening its own.
 */
defineExpose({
  loadArchives,
  selectArchiveFromQuery,
  // Exposed for the view's tests, which drive the deletion guard directly:
  // the DOM path only covers the disabled button, not the re-request guard.
  archivePendingDeletion: computed(() => explorer.value?.pending ?? null),
  requestArchiveDeletion(archive: ArchiveEntry): void {
    explorer.value?.request(archive)
  },
  confirmArchiveDeletion(): Promise<void> {
    return explorer.value?.confirm() ?? Promise.resolve()
  },
  // Precise and synchronous: the server names exactly which archive finished
  // deleting, so drop it from the list directly instead of waiting on
  // onDataChanged's full refetch-and-diff to eventually notice it's gone.
  onArchiveDeleted(name: string): void {
    archives.value = archives.value.filter((a) => a.name !== name)
    explorer.value?.onArchiveDeleted(name)
  },
  onDataChanged(): void {
    explorer.value?.onDataChanged()
  },
  onRepoIdle(): void {
    explorer.value?.onRepoIdle()
  },
})
</script>

<template>
  <!-- Unmatched banner -->
  <div
    v-if="!archivesLoading && unmatchedCount > 0"
    class="unmatched-banner"
  >
    <AlertTriangle
      class="match-icon match-warn unmatched-icon"
      :size="16"
    />
    <div class="unmatched-banner-text">
      <span>
        {{ unmatchedCount }} archive{{ unmatchedCount === 1 ? '' : 's' }} from
        {{ unmatchedHostnames.length }} unresolved hostname{{
          unmatchedHostnames.length === 1 ? '' : 's'
        }}:
        <code
          v-for="h in unmatchedHostnames"
          :key="h"
          class="unmatched-hostname"
          >{{ h }}</code
        >
      </span>
      <span class="unmatched-hint">
        Hostnames are read from borg archive metadata, not derived from the archive name. Configure
        hostname patterns on your hosts to match, then re-scan.
      </span>
    </div>
    <button
      class="btn btn-sm btn-primary unmatched-action"
      :disabled="rescanning"
      @click="rescan"
    >
      {{ rescanning ? 'Scanning...' : 'Re-scan' }}
    </button>
  </div>

  <ArchiveExplorer
    ref="explorer"
    v-model:selected="selectedArchive"
    :repo-id="repoId"
    :repo-name="repoName"
    :archives="sortedArchives"
    :loading="archivesLoading"
    :error="archivesError"
    :is-admin="isAdmin"
    :hide-controls="hasArchiveFilter"
    :reload="loadArchives"
    :refresh-after-delete="refreshRepo"
    empty-description="Archives appear here once a backup has run against this repository."
  >
    <template
      v-if="hasArchiveFilter"
      #banner
    >
      <div class="archive-filter-banner">
        <span>
          Showing only <strong>{{ archiveFilterName }}</strong>
        </span>
        <button
          class="btn btn-sm btn-ghost"
          @click="clearArchiveFilter"
        >
          Show all archives
        </button>
      </div>
    </template>
  </ArchiveExplorer>
</template>

<style scoped>
.unmatched-banner {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
  margin-bottom: var(--space-6);
  padding: var(--space-5) var(--space-6);
  background: var(--warning-subtle);
  border: 1px solid var(--warning);
  border-radius: var(--radius);
  font-size: var(--fs-base);
}

.unmatched-icon {
  flex: none;
}

.unmatched-banner-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  color: var(--text-primary);
}

.unmatched-action {
  margin-left: auto;
  flex: none;
}

.unmatched-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.unmatched-hostname {
  display: inline-block;
  margin: 0 var(--space-2);
  padding: var(--space-1) var(--space-3);
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.archive-filter-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  padding: var(--space-4) var(--space-6);
  background: var(--accent-subtle);
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-sm);
  color: var(--text-primary);
}
</style>

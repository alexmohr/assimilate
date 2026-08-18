<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ChevronRight, AlertTriangle, Trash2 } from '@lucide/vue'
import { apiClient } from '../api/client'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import { useArchiveBrowser, type ArchiveEntry } from '../composables/useArchiveBrowser'
import { useArchiveList, ARCHIVE_SORT_OPTIONS } from '../composables/useArchiveList'
import { useArchiveDeletion } from '../composables/useArchiveDeletion'
import ArchiveBrowserLayout from './ArchiveBrowserLayout.vue'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'
import BaseModal from './BaseModal.vue'
import BaseSpinner from './BaseSpinner.vue'
import BaseHostLink from './BaseHostLink.vue'

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

const {
  archives,
  sortedArchives,
  archivesLoading,
  archivesError,
  loadArchives,
  deleteArchiveByName,
} = useArchiveBrowser(repoIdRef)

const {
  filter: archiveFilter,
  sortMode: archiveSortMode,
  groupByHost: groupArchivesByHost,
  ordered: orderedArchives,
  grouped: groupedArchives,
  unmatchedCount,
  unmatchedHostnames,
  toggleGroup,
  isGroupCollapsed,
  reset: resetArchiveList,
} = useArchiveList(sortedArchives)

const selectedArchive = ref<ArchiveEntry | null>(null)

function selectArchive(archive: ArchiveEntry): void {
  selectedArchive.value = archive
}

const {
  pending: archivePendingDeletion,
  deleteLoading: archiveDeleteLoading,
  isDeleting: isArchiveDeleting,
  request: requestArchiveDeletion,
  close: closeArchiveDeleteDialog,
  confirm: confirmArchiveDeletion,
  forget: forgetDeletingArchive,
  pruneToPresent: pruneDeletingArchives,
  sweepIdle: sweepDeletingArchives,
} = useArchiveDeletion({
  sortedArchives,
  deleteArchiveByName,
  reloadArchives: loadArchives,
  refreshRepo: () => props.refreshRepo(),
  onDeleted: (name) => {
    if (selectedArchive.value?.name === name) selectedArchive.value = null
  },
})

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
    resetArchiveList()
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
  archivePendingDeletion,
  requestArchiveDeletion,
  confirmArchiveDeletion,
  // Precise and synchronous: the server names exactly which archive finished
  // deleting, so drop it from the list directly instead of waiting on
  // onDataChanged's full refetch-and-diff to eventually notice it's gone.
  onArchiveDeleted(name: string): void {
    archives.value = archives.value.filter((a) => a.name !== name)
    forgetDeletingArchive(name)
    if (selectedArchive.value?.name === name) selectedArchive.value = null
  },
  onDataChanged: pruneDeletingArchives,
  onRepoIdle: sweepDeletingArchives,
})
</script>

<template>
  <!-- Unmatched banner -->
  <div
    v-if="!archivesLoading && unmatchedCount > 0"
    class="unmatched-banner"
  >
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
      class="btn btn-sm btn-primary"
      :disabled="rescanning"
      @click="rescan"
    >
      {{ rescanning ? 'Scanning...' : 'Re-scan' }}
    </button>
  </div>

  <ArchiveBrowserLayout>
    <template #list>
      <!-- Archive list -->
      <div class="panel panel--sectioned archives-panel">
        <div class="panel-header">
          <span class="panel-title">Archives</span>
        </div>

        <div
          v-if="archivesLoading"
          class="state-msg state-msg-sm"
        >
          <BaseSpinner size="sm" />
          Loading archives...
        </div>
        <div
          v-else-if="archivesError"
          class="state-msg state-msg-sm state-error"
        >
          {{ archivesError }}
        </div>
        <div
          v-else-if="sortedArchives.length === 0"
          class="state-msg state-msg-sm"
        >
          No archives found.
        </div>
        <div
          v-else-if="hasArchiveFilter"
          class="archive-filter-banner"
        >
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
        <template v-else>
          <div class="archive-controls">
            <input
              v-model="archiveFilter"
              class="input filter-input"
              type="text"
              placeholder="Filter archives..."
            />
            <select
              v-model="archiveSortMode"
              class="input select-input archive-sort-select"
            >
              <option
                v-for="option in ARCHIVE_SORT_OPTIONS"
                :key="option.value"
                :value="option.value"
              >
                {{ option.label }}
              </option>
            </select>
            <button
              class="btn btn-sm btn-ghost archive-group-toggle"
              :class="{ active: groupArchivesByHost }"
              @click="groupArchivesByHost = !groupArchivesByHost"
            >
              {{ groupArchivesByHost ? 'Grouped by host' : 'Flat list' }}
            </button>
          </div>
          <div
            v-if="orderedArchives.length === 0"
            class="state-msg state-msg-sm"
          >
            No matching archives.
          </div>
          <div
            v-else-if="groupArchivesByHost"
            class="archive-groups"
          >
            <div
              v-for="group in groupedArchives"
              :key="group.hostname"
              class="archive-group"
            >
              <button
                class="group-header"
                :class="{ collapsed: isGroupCollapsed(group.hostname) }"
                @click="toggleGroup(group.hostname)"
              >
                <ChevronRight
                  class="group-chevron"
                  :size="14"
                />
                <BaseHostLink
                  :hostname="
                    group.agentHostname && group.matched ? group.agentHostname : group.hostname
                  "
                  class="host-link group-hostname"
                  :class="{ 'group-unmatched': !group.matched }"
                  @click.stop
                />
                <span
                  v-if="!group.matched"
                  class="match-icon match-warn"
                  title="Unmatched"
                >
                  <AlertTriangle :size="12" />
                </span>
                <span class="group-count">{{ group.archives.length }}</span>
              </button>
              <div
                v-show="!isGroupCollapsed(group.hostname)"
                class="group-archives"
              >
                <div
                  v-for="archive in group.archives"
                  :key="archive.name"
                  class="archive-row"
                  :class="{ selected: selectedArchive?.name === archive.name }"
                  @click="selectArchive(archive)"
                >
                  <span class="archive-date">{{ formatDate(archive.start) }}</span>
                  <span class="archive-name">{{ archive.name }}</span>
                  <button
                    v-if="isAdmin"
                    class="btn btn-sm btn-ghost archive-row-delete"
                    :disabled="isArchiveDeleting(archive.name)"
                    :title="
                      isArchiveDeleting(archive.name) ? 'Deletion in progress' : 'Delete archive'
                    "
                    @click.stop="requestArchiveDeletion(archive)"
                  >
                    <BaseSpinner
                      v-if="isArchiveDeleting(archive.name)"
                      size="sm"
                    />
                    <Trash2
                      v-else
                      :size="12"
                    />
                  </button>
                </div>
              </div>
            </div>
          </div>
          <div
            v-else
            class="archive-flat-list"
          >
            <div
              v-for="archive in orderedArchives"
              :key="archive.name"
              class="archive-row archive-row-detailed"
              :class="{ selected: selectedArchive?.name === archive.name }"
              @click="selectArchive(archive)"
            >
              <span class="archive-name">{{ archive.name }}</span>
              <span class="archive-host">{{ archive.agent_hostname ?? archive.hostname }}</span>
              <span class="archive-date">{{ formatDate(archive.start) }}</span>
              <span class="archive-size">{{ formatBytes(archive.original_size) }}</span>
              <span class="archive-size">{{ formatBytes(archive.deduplicated_size) }}</span>
              <button
                v-if="isAdmin"
                class="btn btn-sm btn-ghost archive-row-delete"
                :disabled="isArchiveDeleting(archive.name)"
                :title="isArchiveDeleting(archive.name) ? 'Deletion in progress' : 'Delete archive'"
                @click.stop="requestArchiveDeletion(archive)"
              >
                <BaseSpinner
                  v-if="isArchiveDeleting(archive.name)"
                  size="sm"
                />
                <Trash2
                  v-else
                  :size="12"
                />
              </button>
            </div>
          </div>
        </template>
      </div>
    </template>
    <template #browser>
      <div class="panel panel--sectioned browser-panel">
        <ArchiveFileBrowser
          :repo-id="repoId"
          :archive="selectedArchive"
          :is-admin="isAdmin"
          :deleting="selectedArchive !== null && isArchiveDeleting(selectedArchive.name)"
          @delete-archive="requestArchiveDeletion"
        />
      </div>
    </template>
  </ArchiveBrowserLayout>

  <BaseModal
    :open="archivePendingDeletion !== null"
    title="Delete Archive"
    size="sm"
    @close="closeArchiveDeleteDialog"
  >
    <div class="archive-delete-message">
      <div class="archive-delete-icon">
        <Trash2 :size="20" />
      </div>
      <div>
        <p>
          Permanently delete <strong>{{ archivePendingDeletion?.name }}</strong> from
          <strong>{{ repoName }}</strong
          >?
        </p>
        <p class="muted">This archive and its stored backup data cannot be recovered.</p>
      </div>
    </div>
    <template #footer>
      <button
        class="btn btn-ghost"
        :disabled="archiveDeleteLoading"
        @click="closeArchiveDeleteDialog"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="archiveDeleteLoading"
        @click="confirmArchiveDeletion"
      >
        {{ archiveDeleteLoading ? 'Deleting...' : 'Delete Archive' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.unmatched-banner {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
  padding: 0.75rem 1rem;
  background: var(--warning-subtle);
  border: 1px solid var(--warning);
  border-radius: var(--radius);
  font-size: var(--fs-base);
}

.unmatched-banner-text {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  color: var(--text-primary);
}

.unmatched-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.unmatched-hostname {
  display: inline-block;
  margin: 0 0.25rem;
  padding: 0.1rem 0.4rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.archive-controls {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border);
}

.archive-sort-select {
  min-width: 13rem;
}

.archive-filter-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  background: var(--accent-subtle);
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-sm);
  color: var(--text-primary);
}

.archive-group {
  border-bottom: 1px solid var(--border-subtle);
}

.archive-group-toggle {
  flex-shrink: 0;
  white-space: nowrap;
}

.group-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-hover);
  border: none;
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
  transition: background var(--duration-fast);
}

.group-header:hover {
  background: var(--bg-hover);
}

.group-header.collapsed .group-chevron {
  transform: rotate(0deg);
}

.group-chevron {
  display: inline-block;
  font-size: var(--fs-lg);
  transition: transform var(--duration-base);
  transform: rotate(90deg);
}

.group-hostname {
  flex: 1;
}

.group-count {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  background: var(--bg-card);
  border-radius: var(--radius-pill);
  padding: 0.1rem 0.5rem;
  min-width: 1.4rem;
  text-align: center;
}

.group-unmatched {
  color: var(--warning);
}

.group-archives {
  display: flex;
  flex-direction: column;
}

.archive-groups,
.archive-flat-list {
  max-height: 500px;
  overflow-y: auto;
}

.archive-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem 0.4rem 1.5rem;
  border: none;
  background: none;
  cursor: pointer;
  text-align: left;
  transition: background var(--duration-fast);
  border-bottom: 1px solid var(--border-subtle);
}

.archive-row:hover .archive-row-delete,
.archive-row.selected .archive-row-delete {
  opacity: 1;
}

.archive-row-detailed {
  display: grid;
  grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr) 9.5rem 4.25rem 4.25rem auto;
  gap: 0.75rem;
  padding-left: 0.75rem;
}

.archive-row-delete {
  margin-left: auto;
  opacity: 0;
  transition: opacity var(--duration-fast);
  flex-shrink: 0;
}

.archive-name {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-host {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-date {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}

.archive-size {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  white-space: nowrap;
}

.archive-delete-message {
  display: flex;
  gap: 1rem;
  align-items: flex-start;
}

.archive-delete-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2.75rem;
  height: 2.75rem;
  flex: 0 0 auto;
  border-radius: 50%;
  color: var(--danger);
  background: var(--danger-subtle);
}

/* Carried over with the markup from RepoDetailView: scoped rules do not
   follow moved templates, so without these the filter input, host links and
   match icons rendered unstyled. */
.archive-controls .filter-input {
  flex: 1;
}
</style>

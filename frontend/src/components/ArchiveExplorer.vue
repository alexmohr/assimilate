<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, toRef, useTemplateRef } from 'vue'
import { Trash2 } from '@lucide/vue'
import ArchiveBrowserLayout from './ArchiveBrowserLayout.vue'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'
import ArchiveSelector from './ArchiveSelector.vue'
import BaseModal from './BaseModal.vue'
import { requestArchiveDelete, type ArchiveEntry } from '../composables/useArchiveBrowser'
import { useArchiveDeletion } from '../composables/useArchiveDeletion'

/**
 * The archive screen: a list of archives on the left, the selected archive's
 * files on the right, and one deletion flow shared by both panes.
 *
 * Every screen that browses borg archives renders this - the repository's
 * Archives tab, a schedule's Backups tab and the standalone Archives page.
 * They differ only in where the archive list comes from, which is why it
 * arrives as a prop rather than being fetched here.
 */
const props = withDefaults(
  defineProps<{
    /** Null while the caller has not resolved a repository yet. */
    repoId: number | null
    archives: ArchiveEntry[]
    loading?: boolean
    error?: string | null
    /** Gates restore and delete; browsing and downloading are open to any viewer. */
    isAdmin?: boolean
    /** Named in the delete confirmation, so it says what the archive is being removed from. */
    repoName?: string
    /** Silent refetch of `archives`, used to clear stale deletion markers. */
    reload?: (silent: boolean) => Promise<unknown>
    /** Refreshes whatever else a delete invalidates, e.g. the repository's stats. */
    refreshAfterDelete?: () => Promise<unknown>
    /** Drops the toolbar and rows, leaving the `banner` slot in their place. */
    hideControls?: boolean
    emptyTitle?: string
    emptyDescription?: string
  }>(),
  {
    loading: false,
    error: null,
    isAdmin: false,
    repoName: '',
    reload: () => Promise.resolve(),
    refreshAfterDelete: () => Promise.resolve(),
    hideControls: false,
    emptyTitle: 'No archives',
    emptyDescription: 'Archives appear here once a backup has run against this repository.',
  },
)

const selected = defineModel<ArchiveEntry | null>('selected', { required: true })

const selector = useTemplateRef<InstanceType<typeof ArchiveSelector>>('selector')

/** Deleting needs both the right to do it and a repository to do it against. */
const canDelete = computed(() => props.isAdmin && props.repoId !== null)

const archivesRef = toRef(props, 'archives')

const {
  pending,
  deleteLoading,
  isDeleting,
  request,
  close,
  confirm,
  forget,
  pruneToPresent,
  sweepIdle,
} = useArchiveDeletion({
  sortedArchives: archivesRef,
  deleteArchiveByName: async (archive) => {
    if (props.repoId === null) throw new Error('No repository selected')
    await requestArchiveDelete(props.repoId, archive.name)
  },
  reloadArchives: (silent) => props.reload(silent),
  refreshRepo: () => props.refreshAfterDelete(),
  onDeleted: (name) => {
    if (selected.value?.name === name) selected.value = null
  },
})

// `isDeleting` is a function over a private ref, so the selector - which needs
// the whole set to render its rows - gets the names it covers instead.
const deletingList = computed(() =>
  props.archives.filter((a) => isDeleting(a.name)).map((a) => a.name),
)

const selectedDeleting = computed(() => selected.value !== null && isDeleting(selected.value.name))

/**
 * The parent owns the WebSocket subscription, so it forwards the events that
 * touch archive state here rather than this component opening its own.
 */
defineExpose({
  isDeleting,
  pending,
  request,
  confirm,
  /** Clears the search and collapse state, e.g. when the repository changes. */
  resetList(): void {
    selector.value?.reset()
  },
  /** The server names the archive that finished deleting, so drop its marker. */
  onArchiveDeleted(name: string): void {
    forget(name)
    if (selected.value?.name === name) selected.value = null
  },
  onDataChanged: pruneToPresent,
  onRepoIdle: sweepIdle,
})
</script>

<template>
  <ArchiveBrowserLayout narrow-list>
    <template #list>
      <ArchiveSelector
        ref="selector"
        v-model:selected="selected"
        :archives="archives"
        :loading="loading"
        :error="error"
        :can-delete="canDelete"
        :deleting-names="deletingList"
        :hide-controls="hideControls"
        :empty-title="emptyTitle"
        :empty-description="emptyDescription"
        @delete="request"
      >
        <template
          v-if="$slots.actions"
          #actions
        >
          <slot name="actions" />
        </template>
        <template
          v-if="$slots.banner"
          #banner
        >
          <slot name="banner" />
        </template>
      </ArchiveSelector>
    </template>

    <template #browser>
      <div class="panel panel--sectioned browser-panel">
        <ArchiveFileBrowser
          :repo-id="repoId"
          :archive="selected"
          :is-admin="isAdmin"
          :can-delete="canDelete"
          :deleting="selectedDeleting"
          @delete-archive="request"
        />
      </div>
    </template>
  </ArchiveBrowserLayout>

  <BaseModal
    :open="pending !== null"
    title="Delete archive"
    size="sm"
    @close="close"
  >
    <div class="archive-delete-message">
      <div class="archive-delete-icon">
        <Trash2 :size="20" />
      </div>
      <div>
        <p>
          Permanently delete <strong>{{ pending?.name }}</strong>
          <template v-if="repoName">
            from <strong>{{ repoName }}</strong>
          </template>
          ?
        </p>
        <p class="muted">This archive and its stored backup data cannot be recovered.</p>
      </div>
    </div>
    <template #footer>
      <button
        class="btn btn-ghost"
        :disabled="deleteLoading"
        @click="close"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="deleteLoading"
        @click="confirm"
      >
        {{ deleteLoading ? 'Deleting...' : 'Delete archive' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.browser-panel {
  min-height: 18rem;
}

.archive-delete-message {
  display: flex;
  gap: var(--space-6);
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
</style>

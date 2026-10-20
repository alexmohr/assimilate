<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, toRef, useTemplateRef, watch } from 'vue'
import { Trash2 } from '@lucide/vue'
import ArchiveBrowserLayout from './ArchiveBrowserLayout.vue'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'
import ArchiveSelector from './ArchiveSelector.vue'
import BaseModal from './BaseModal.vue'
import { requestArchiveDelete } from '../composables/useArchiveBrowser'
import { useArchiveDeletion } from '../composables/useArchiveDeletion'
import {
  DEFAULT_ARCHIVE_NAME_QUERY_KEY,
  DEFAULT_ARCHIVE_PATH_QUERY_KEY,
  useArchiveSelectionRoute,
} from '../composables/useArchivePathRoute'
import { archiveKey, sameArchive, type RepoArchive } from '../utils/repoArchive'

/**
 * The archive screen: a list of archives on the left, the selected archive's
 * files on the right, and one deletion flow shared by both panes. On a phone
 * the two take turns: opening an archive replaces the list, and the browser's
 * back button brings it back.
 *
 * Every screen that browses borg archives renders this - the repository's
 * Archives tab, a schedule's and an agent's Backups tab and the standalone
 * Archives page. They differ only in where the archive list comes from, which
 * is why it arrives as a prop rather than being fetched here. The list may
 * span several repositories (an agent backing up to two, a schedule with two
 * targets); each archive names its own, and browsing, restoring and deleting
 * it all act against that one.
 */
const props = withDefaults(
  defineProps<{
    archives: RepoArchive[]
    loading?: boolean
    error?: string | null
    /** Gates restore and delete; browsing and downloading are open to any viewer. */
    isAdmin?: boolean
    /** Silent refetch of `archives`, used to clear stale deletion markers. */
    reload?: (silent: boolean) => Promise<unknown>
    /** Refreshes whatever else a delete invalidates, e.g. the repository's stats. */
    refreshAfterDelete?: () => Promise<unknown>
    /** Drops the toolbar and rows, leaving the `banner` slot in their place. */
    hideControls?: boolean
    emptyTitle?: string
    emptyDescription?: string
    /**
     * The route query parameter the browser keeps its open directory under.
     * Only a page that renders more than one explorer needs to set it, so
     * each keeps its own.
     */
    pathQueryKey?: string
    /**
     * The route query parameter the open archive is kept under. Set alongside
     * `pathQueryKey` by a page that renders more than one explorer.
     */
    archiveQueryKey?: string
  }>(),
  {
    loading: false,
    error: null,
    isAdmin: false,
    reload: () => Promise.resolve(),
    refreshAfterDelete: () => Promise.resolve(),
    hideControls: false,
    emptyTitle: 'No archives',
    emptyDescription: 'Archives appear here once a backup has run against this repository.',
    pathQueryKey: undefined,
    archiveQueryKey: DEFAULT_ARCHIVE_NAME_QUERY_KEY,
  },
)

const selected = defineModel<RepoArchive | null>('selected', { required: true })

const selector = useTemplateRef<InstanceType<typeof ArchiveSelector>>('selector')

/** Whether the list holds more than one repository's archives. */
const multiRepo = computed(() => new Set(props.archives.map((a) => a.repo_id)).size > 1)

/**
 * What the route keeps for `archive`: its name, the way a single
 * repository's screens always have, or - once the list spans several
 * repositories, where one name can be two archives - `<repo id>/<name>`.
 * borg refuses a slash in an archive name, so the two forms never collide.
 */
function routeValue(archive: RepoArchive): string {
  return multiRepo.value ? archiveKey(archive) : archive.name
}

/** The archive a route value names, in either form. */
function named(value: string): RepoArchive | undefined {
  return (
    props.archives.find((a) => archiveKey(a) === value) ??
    props.archives.find((a) => a.name === value)
  )
}

const archiveRoute = useArchiveSelectionRoute(
  () => props.archiveQueryKey,
  () => props.pathQueryKey ?? DEFAULT_ARCHIVE_PATH_QUERY_KEY,
)

/**
 * The archive named in the route, handed to the browser so it knows when the
 * route's directory is the open archive's. Undefined without a router, which
 * leaves the browser managing its directory on its own.
 */
const routeArchiveName = computed(() => {
  if (!archiveRoute.enabled) return undefined
  const value = archiveRoute.name.value
  return value === null ? null : (named(value)?.name ?? value)
})

/**
 * A pick in the list is a new history entry, so Back returns to the archive
 * and directory open before it. The selection changes at once; the route
 * catches up when the navigation lands.
 */
let pushedName: string | null = null
/** Whether the archive the route named on arrival has been read or replaced. */
let routeRead = false

function pick(archive: RepoArchive | null): void {
  if (archiveRoute.enabled && archive !== null && routeValue(archive) !== archiveRoute.name.value) {
    pushedName = routeValue(archive)
    routeRead = true
    archiveRoute.push(pushedName)
  }
  selected.value = archive
}

// Back, Forward and links name the archive, so the selection follows the
// route. Only a change of the route moves it: the list reloads on its own
// schedule, and the route may still name the archive a pending navigation is
// about to replace.
watch(archiveRoute.name, (name) => {
  routeRead = true
  // The pick's entry has landed; a later move to another archive is not it.
  if (name === pushedName) pushedName = null
  if (name === null) {
    selected.value = null
    return
  }
  const match = named(name)
  if (match && !sameArchive(selected.value, match)) selected.value = match
})

// A link names its archive before the list has loaded, so the first list that
// holds it opens it - once; after that the route watcher above is in charge.
watch(
  () => props.archives,
  () => {
    const name = archiveRoute.name.value
    if (routeRead || name === null) return
    const match = named(name)
    if (match) {
      routeRead = true
      selected.value = match
    }
  },
  { immediate: true },
)

// The page can change the selection itself (a repository switch, a delete, a
// deep link to one archive), and the route follows without a history entry.
watch(
  () => (selected.value === null ? null : routeValue(selected.value)),
  (name) => {
    // A pick already pushed its own entry; replacing now would cancel it.
    if (name !== null && name === pushedName) return
    const pushInFlight = pushedName !== null
    pushedName = null
    if (!archiveRoute.enabled) return
    routeRead = true
    if (pushInFlight || name !== archiveRoute.name.value) archiveRoute.replace(name)
  },
)

onBeforeUnmount(() => archiveRoute.release())

const archivesRef = toRef(props, 'archives')

/**
 * The archives the open one can be compared with: its own repository's. A
 * name is only unique within a repository, and the comparison names the other
 * archive by name alone.
 */
const sameRepoArchives = computed<RepoArchive[]>(() => {
  const open = selected.value
  return open === null ? [] : props.archives.filter((a) => a.repo_id === open.repo_id)
})

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
  deleteArchiveByName: (archive) => requestArchiveDelete(archive.repo_id, archive.name),
  reloadArchives: (silent) => props.reload(silent),
  refreshRepo: () => props.refreshAfterDelete(),
  onDeleted: (archive) => {
    if (sameArchive(selected.value, archive)) selected.value = null
  },
})

// `isDeleting` is a function over a private ref, so the selector - which needs
// the whole set to render its rows - gets the copies it covers instead.
const deletingList = computed(() => props.archives.filter(isDeleting).map(archiveKey))

const selectedDeleting = computed(() => selected.value !== null && isDeleting(selected.value))

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
  // Forwarded rather than recomputed by the caller: the selector's list already
  // decides what "unmatched" means, and a second definition beside it is free
  // to drift the day that rule changes.
  unmatchedCount: computed(() => selector.value?.unmatchedCount ?? 0),
  unmatchedHostnames: computed<string[]>(() => selector.value?.unmatchedHostnames ?? []),
  /** The server names the archive that finished deleting, so drop its marker. */
  onArchiveDeleted(name: string, repoId: number): void {
    forget(name, repoId)
    // Only when the cleared copy is the one on screen: the same name exists in
    // every repository a schedule targets, and another repository's delete
    // says nothing about what this pane is showing.
    if (sameArchive(selected.value, { repo_id: repoId, name })) selected.value = null
  },
  onDataChanged: pruneToPresent,
  onRepoIdle: sweepIdle,
})
</script>

<template>
  <ArchiveBrowserLayout
    narrow-list
    :detail-open="selected !== null"
  >
    <template #list>
      <ArchiveSelector
        ref="selector"
        :selected="selected"
        :archives="archives"
        :loading="loading"
        :error="error"
        :can-delete="isAdmin"
        :deleting-keys="deletingList"
        :hide-controls="hideControls"
        :empty-title="emptyTitle"
        :empty-description="emptyDescription"
        @update:selected="pick"
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
          :repo-id="selected?.repo_id ?? null"
          :repo-name="selected?.repo_name ?? ''"
          :archive="selected"
          :archives="sameRepoArchives"
          :is-admin="isAdmin"
          :deleting="selectedDeleting"
          :path-query-key="pathQueryKey"
          :route-archive-name="routeArchiveName"
          @delete-archive="selected !== null && request(selected)"
          @back="pick(null)"
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
          <template v-if="pending?.repo_name">
            from <strong>{{ pending.repo_name }}</strong>
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

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch, onBeforeUnmount } from 'vue'
import { formatBytes, formatDate } from '../utils/format'
import { extractBlobError } from '../utils/error'
import { resolveArchiveHost } from '../utils/archiveHost'
import { useToast } from '../composables/useToast'
import { useRestoreToasts } from '../composables/useRestoreToasts'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import {
  ArrowLeft,
  ArrowRight,
  CornerLeftUp,
  Download,
  File,
  FileDiff,
  FileSymlink,
  Folder,
  FolderSymlink,
  History,
  RotateCcw,
  Trash2,
} from '@lucide/vue'
import axios from 'axios'
import { downloadArchiveFiles, getArchiveContents } from '../api/archives'
import BaseSpinner from './BaseSpinner.vue'
import BaseHostLink from './BaseHostLink.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import ArchiveDatabaseDumps from './ArchiveDatabaseDumps.vue'
import FileDiffDialog from './FileDiffDialog.vue'
import FileHistoryDialog from './FileHistoryDialog.vue'
import RestoreDialog from './RestoreDialog.vue'
import ArchiveTreeSearch from './ArchiveTreeSearch.vue'
import { comparableArchives, entryKind } from '../utils/fileDiff'
import { linkFolder, linksToFolder, linkTitle } from '../utils/archiveLink'
import { extractError } from '../utils/error'
import {
  directChildren,
  isDirectoryEntry,
  tickAll,
  tickState,
  toggle,
  untick,
  type ChildrenOf,
  type TickState,
  type UntickResult,
} from '../utils/archiveSelection'
import { useArchiveTreeTotals } from '../composables/useArchiveTreeTotals'
import { useArchiveChanges } from '../composables/useArchiveChanges'
import type { ChangeKind } from '../utils/archiveChanges'
import {
  useArchiveBrowser,
  type ArchiveEntry,
  type ContentEntry,
  type DisplayEntry,
} from '../composables/useArchiveBrowser'
import {
  DEFAULT_ARCHIVE_PATH_QUERY_KEY,
  parseArchivePath,
  useArchivePathRoute,
} from '../composables/useArchivePathRoute'
import type { RestoreRun } from '../types/generated'

const CURRENT_DIR_MARKER = '.'
const PARENT_DIR_MARKER = '..'

/**
 * How many entries the browser's directory listing holds at most. A listing
 * that long may have been cut short, so it is not taken as the folder's full
 * contents.
 */
const BROWSER_LISTING_LIMIT = 100

/**
 * Entries asked for when a folder's full listing is needed: unticking one
 * entry inside a ticked folder replaces the folder with everything else in it.
 */
const SPLIT_LISTING_LIMIT = 10_000

/** What the browser shows of the archive: its files, or the database dumps it holds. */
type BrowserView = 'files' | 'databases'

const VIEW_LABELS: Record<BrowserView, string> = { files: 'Files', databases: 'Databases' }

const VIEW_OPTIONS: readonly SegmentedOption<BrowserView>[] = [
  { value: 'files', label: VIEW_LABELS.files },
  { value: 'databases', label: VIEW_LABELS.databases },
]

const props = withDefaults(
  defineProps<{
    repoId: number | null
    archive: ArchiveEntry | null
    /**
     * The repository `repoId` names. Browsing, restoring and - above all -
     * the Delete button in this header all act against one repository, and a
     * schedule that copies into several writes an archive of the same name
     * into each of them, so the header has to say which copy is open.
     */
    repoName?: string
    isAdmin?: boolean
    // Whether `archive` already has a delete in flight - deletion is async
    // (the request just enqueues the borg job), so without this the delete
    // button stays clickable for an archive that's already being removed.
    deleting?: boolean
    /**
     * The route query parameter the open directory is kept under, so Back
     * walks back up the directories visited. A page that shows more than one
     * browser at once gives each its own.
     */
    pathQueryKey?: string
    /**
     * The archive the route names, when the explorer around this browser keeps
     * its selection in the route too. The explorer then owns both keys: it
     * drops the directory whenever the archive changes, and the directory in
     * the route only applies once the route names the archive shown here.
     * Left undefined, the browser clears the directory itself on a change.
     */
    routeArchiveName?: string | null
    /**
     * The other archives a file can be compared with - the list the archive
     * was picked from. Without a second archive there is nothing to compare,
     * so the Compare button stays hidden.
     */
    archives?: readonly ArchiveEntry[]
  }>(),
  {
    repoName: '',
    isAdmin: false,
    deleting: false,
    pathQueryKey: DEFAULT_ARCHIVE_PATH_QUERY_KEY,
    archives: () => [],
  },
)

const emit = defineEmits<{
  'delete-archive': [archive: ArchiveEntry]
  /** Back to the archive list - the phone layout shows one pane at a time. */
  back: []
}>()

const view = ref<BrowserView>('files')
/** The file whose comparison dialog is open, or null. */
const comparePath = ref<string | null>(null)
const historyPath = ref<string | null>(null)

/**
 * The entries picked for a restore or download, by path, across every folder
 * of the archive: picking a file, going up a level and picking another keeps
 * both, so one restore can bring back things that sit in different places.
 *
 * It is kept as the fewest paths that cover what is ticked: a ticked folder
 * stands for everything under it, and a folder whose every entry is ticked
 * takes their place.
 */
const picked = ref<Map<string, ContentEntry>>(new Map())

/** Every folder of the open archive whose full listing has been seen, its entries by folder. */
const listings = ref<Map<string, ContentEntry[]>>(new Map())

/**
 * Every entry of the open archive the browser has shown, by path, complete
 * listing or not: what a picked path is, so a file stays a file.
 */
const seenEntries = ref<Map<string, ContentEntry>>(new Map())

/** Whether a ticked folder is being split, which first loads the listings it needs. */
const splitting = ref(false)

/** Whether a search is entered, so its results stand in for the folder's listing. */
const searching = ref(false)

/** The paths the open restore dialog restores (empty: the whole archive), or null. */
const restorePaths = ref<readonly string[] | null>(null)

const viewLabel = computed<string>(() => VIEW_LABELS[view.value])

const { error: toastError } = useToast()
const { track: trackRestore } = useRestoreToasts()

const repoIdRef = computed(() => props.repoId ?? 0)
const browser = useArchiveBrowser(repoIdRef)

const breadcrumbs = browser.breadcrumbs
const contents = browser.contents
const contentsLoading = browser.contentsLoading
const contentsError = browser.contentsError
const indexing = browser.indexing
const downloadEntry = browser.downloadEntry
const browserEntries = browser.browserEntries
const browserFilters = browser.browserFilters

const pathRoute = useArchivePathRoute(() => props.pathQueryKey)

/**
 * Opens a directory as a new history entry, so Back returns to the one being
 * left. The route watcher below does the loading; reopening the directory
 * already in the route, or browsing without a router, just reloads it.
 */
function navigateTo(path: string): void {
  const target = parseArchivePath(path)
  if (pathRoute.enabled && target !== pathRoute.path.value) {
    pathRoute.push(target)
  } else {
    browser.navigateTo(target)
  }
}

function handleRowClick(entry: DisplayEntry): void {
  if (entry.isDir && entry.displayName !== CURRENT_DIR_MARKER) {
    navigateTo(entry.path)
  } else if (entry.link !== null && linksToFolder(entry.link)) {
    openLinkTarget(entry)
  }
}

/** Opens the folder showing a link's target, when the archive holds it. */
function openLinkTarget(entry: DisplayEntry): void {
  const folder = entry.link === null ? null : linkFolder(entry.link)
  if (folder !== null) navigateTo(folder)
}

function isClickable(entry: DisplayEntry): boolean {
  return entry.isDir || (entry.link !== null && linksToFolder(entry.link))
}

function reset(): void {
  browser.stopPolling()
  browser.selectedArchive.value = null
  browser.currentPath.value = '/'
  browser.contents.value = []
  browser.contentsError.value = null
  browser.indexing.value = false
  browser.contentsLoading.value = false
}

/** Whether the route's directory belongs to the archive shown here. */
function routePathApplies(): boolean {
  return props.routeArchiveName === undefined || props.routeArchiveName === props.archive?.name
}

// Keyed by repository and name, not by the object: a background refetch hands
// the open archive back as a new object, and that must not throw away what is
// picked or close the restore dialog.
watch(
  () => (props.archive === null ? null : `${props.repoId}/${props.archive.name}`),
  (_key, previous) => {
    const archive = props.archive
    // Another archive opens on its files, as a fresh selection does.
    view.value = 'files'
    comparePath.value = null
    historyPath.value = null
    picked.value = new Map()
    listings.value = new Map()
    seenEntries.value = new Map()
    restorePaths.value = null
    if (archive && props.routeArchiveName !== undefined) {
      // The explorer drops the directory in the same navigation that names a
      // new archive, so until the route names this one its directory is stale.
      browser.selectArchive(archive, routePathApplies() ? pathRoute.path.value : '/')
    } else if (archive) {
      if (previous) {
        // Another archive replaced the open one: it opens at its root, and the
        // directory the route held belonged to the archive just left.
        pathRoute.clear()
        browser.selectArchive(archive)
      } else {
        // The first archive opened here, or one reopened by Back: start where
        // the route points, which is how a link to a directory lands in it.
        browser.selectArchive(archive, pathRoute.path.value)
      }
    } else {
      reset()
      if (previous && props.routeArchiveName === undefined) pathRoute.clear()
    }
  },
  { immediate: true },
)

// Back and Forward change the route without a click here, so the directory
// follows the route rather than the other way round.
watch(pathRoute.path, (path) => {
  if (props.archive && routePathApplies() && path !== browser.currentPath.value) {
    browser.loadContents(path)
  }
})

onBeforeUnmount(() => {
  browser.stopPolling()
  // The explorer releases both keys at once when it owns the archive key: two
  // replaces in one tick would cancel each other.
  if (props.routeArchiveName === undefined) pathRoute.release()
  downloadController.value?.abort()
})

/**
 * The current folder's own "." row stands for the folder, and the ".." row
 * for the one above it: neither is an entry in it, and picking ".." would
 * restore or download the whole parent.
 */
function isPickable(entry: DisplayEntry): boolean {
  return (
    entry.displayName !== CURRENT_DIR_MARKER &&
    entry.displayName !== PARENT_DIR_MARKER &&
    entry.path.length > 0
  )
}

/** The archive path of the folder being shown: `''` at the archive root. */
const currentDir = computed(() => browser.currentPath.value.replace(/^\/+/, ''))

/** Archive `dir`'s entries are `entries`, all of them. */
function recordListing(dir: string, entries: readonly ContentEntry[]): void {
  const next = new Map(listings.value)
  next.set(dir, directChildren(entries, dir))
  listings.value = next
}

function rememberEntries(entries: readonly ContentEntry[]): void {
  const seen = new Map(seenEntries.value)
  entries.forEach((entry) => seen.set(entry.path, entry))
  seenEntries.value = seen
}

watch(contents, (entries) => {
  rememberEntries(entries)
  if (entries.length > 0 && entries.length < BROWSER_LISTING_LIMIT) {
    recordListing(currentDir.value, entries)
  }
})

const childrenOf: ChildrenOf = (dir) => listings.value.get(dir)?.map((entry) => entry.path) ?? null

const pickedPaths = computed(() => [...picked.value.keys()])

function tickStateOf(entry: DisplayEntry): TickState {
  return tickState(pickedPaths.value, entry.path)
}

/**
 * Makes `paths` the selection. Every path is looked up among the entries
 * seen: the one ticked, a folder a collapse produced, or an entry a split
 * uncovered. A folder never shown is all a collapse can name, so that is
 * what an unknown path is.
 */
function applySelection(paths: readonly string[]): void {
  picked.value = new Map(
    paths.map((path): [string, ContentEntry] => [
      path,
      seenEntries.value.get(path) ??
        picked.value.get(path) ?? { type: 'd', path, size: 0, mtime: '', mode: '' },
    ]),
  )
}

/** Loads `dir`'s full listing, unless it holds more than one request returns. */
async function loadListing(repoId: number, archiveName: string, dir: string): Promise<void> {
  const response = await getArchiveContents(repoId, archiveName, {
    path: dir,
    limit: SPLIT_LISTING_LIMIT,
  })
  rememberEntries(response.entries)
  if (response.entries.length < SPLIT_LISTING_LIMIT) recordListing(dir, response.entries)
}

/**
 * Applies a change to the selection. Unticking an entry inside a ticked
 * folder needs the listings of the folders on the way down to it; they are
 * loaded first, then the change is worked out again.
 */
async function changeSelection(change: () => UntickResult): Promise<void> {
  const result = change()
  if ('selection' in result) {
    applySelection(result.selection)
    return
  }
  const archive = props.archive
  const repoId = props.repoId
  if (archive === null || repoId === null) return
  splitting.value = true
  try {
    await Promise.all(result.missing.map((dir) => loadListing(repoId, archive.name, dir)))
  } catch (e: unknown) {
    toastError(extractError(e))
    return
  } finally {
    splitting.value = false
  }
  // Another archive may have opened while the listings loaded.
  if (props.archive?.name !== archive.name || props.repoId !== repoId) return
  const retried = change()
  if ('selection' in retried) applySelection(retried.selection)
  else toastError('This folder holds too many entries to untick one of them. Untick the folder.')
}

function togglePicked(entry: DisplayEntry): void {
  void changeSelection(() => toggle(pickedPaths.value, entry.path, childrenOf))
}

function toggleHit(entry: ContentEntry): void {
  // A match may sit in a folder the browser never listed.
  rememberEntries([entry])
  void changeSelection(() => toggle(pickedPaths.value, entry.path, childrenOf))
}

function tickStateOfPath(path: string): TickState {
  return tickState(pickedPaths.value, path)
}

const pickableHere = computed(() => browserEntries.value.filter(isPickable))

/** The folder's own tick state, shown by the header checkbox. */
const hereState = computed<TickState>(() => {
  const states = pickableHere.value.map(tickStateOf)
  if (states.length > 0 && states.every((state) => state === 'checked')) return 'checked'
  return states.some((state) => state !== 'unchecked') ? 'partial' : 'unchecked'
})

function toggleAllHere(): void {
  if (hereState.value === 'checked') {
    void changeSelection(() => untick(pickedPaths.value, currentDir.value, childrenOf))
  } else {
    applySelection(
      tickAll(
        pickedPaths.value,
        pickableHere.value.map((entry) => entry.path),
        childrenOf,
      ),
    )
  }
}

function clearPicked(): void {
  picked.value = new Map()
}

const treeTotals = useArchiveTreeTotals(
  () => props.repoId,
  () => props.archive?.name ?? null,
)
const totals = treeTotals.totals

// A folder's size is what is under it, which the listing does not say.
watch(
  () => (props.archive === null || contents.value.length === 0 ? [] : pickableHere.value),
  (entries) => {
    void treeTotals.ensure(entries.filter((entry) => entry.isDir).map((entry) => entry.path))
  },
)
watch(picked, (entries) => {
  void treeTotals.ensure([...entries.values()].filter(isDirectoryEntry).map((entry) => entry.path))
})

const changes = useArchiveChanges(
  () => props.repoId,
  () => props.archive,
  () => props.archives,
)

// Each folder shown is compared with the same folder in the previous archive.
watch(
  () => (props.archive === null || contents.value.length === 0 ? null : currentDir.value),
  (dir) => {
    if (dir !== null) void changes.load(dir)
  },
)

/** How a row's change badge reads and looks. */
interface ChangeBadge {
  label: string
  tone: string
  title: string
}

const CHANGE_BADGES: Record<ChangeKind, Omit<ChangeBadge, 'title'>> = {
  new: { label: 'New', tone: 'badge--success' },
  changed: { label: 'Changed', tone: 'badge--info' },
}

/** A file's change against the previous archive, for its row; never for the `.` and `..` rows. */
function changeBadge(entry: DisplayEntry): ChangeBadge | null {
  const change = isPickable(entry) ? changes.changeOf(entry) : null
  if (change === null) return null
  const badge = CHANGE_BADGES[change]
  return { ...badge, title: `${badge.label} since ${changes.previous.value?.name ?? ''}` }
}

/** A file's own size; a folder's, what the index says is under it, once it has said. */
function sizeLabel(entry: DisplayEntry): string {
  if (entry.link !== null) return '-'
  if (!entry.isDir) return entry.displaySize
  const total = isPickable(entry) ? totals.value.get(entry.path) : undefined
  return total === undefined ? '-' : formatBytes(total.bytes)
}

function filesLabel(files: number): string {
  return `${files} ${files === 1 ? 'file' : 'files'}`
}

/**
 * What the selection amounts to: its files and their size once the content
 * index has added up every ticked folder, just its count until then.
 */
const pickedSummary = computed(() => {
  const entries = [...picked.value.values()]
  const count = `${entries.length} selected`
  const sizes = entries.map((entry) =>
    isDirectoryEntry(entry)
      ? (totals.value.get(entry.path) ?? null)
      : { files: 1, bytes: Number(entry.size) },
  )
  if (sizes.some((size) => size === null)) return count
  const files = sizes.reduce((sum, size) => sum + (size?.files ?? 0), 0)
  const bytes = sizes.reduce((sum, size) => sum + (size?.bytes ?? 0), 0)
  return `${count} · ${formatBytes(bytes)} · ${filesLabel(files)}`
})

function openRestore(paths: readonly string[]): void {
  restorePaths.value = paths
}

function onRestored(run: RestoreRun): void {
  trackRestore(run)
  picked.value = new Map()
}

/** The picked-files download in flight, if any; aborting it stops the borg behind it too. */
const downloadController = ref<AbortController | null>(null)
const downloading = computed(() => downloadController.value !== null)

/** One file downloads as itself; anything more as one tar.lz4 the server streams. */
async function downloadPicked(): Promise<void> {
  const archive = props.archive
  if (archive === null || props.repoId === null) return
  const entries = [...picked.value.values()]
  const [only] = entries
  if (entries.length === 1 && only !== undefined && !isDirectoryEntry(only)) {
    downloadEntry(only)
    return
  }
  const controller = new AbortController()
  downloadController.value = controller
  try {
    const blob = await downloadArchiveFiles(
      props.repoId,
      archive.name,
      pickedPaths.value,
      controller.signal,
    )
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${archive.name}.tar.lz4`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
  } catch (e: unknown) {
    // The body of a failed blob request is a Blob too; reading it is what
    // turns it back into the server's message and error id.
    if (!axios.isCancel(e)) toastError(await extractBlobError(e))
  } finally {
    downloadController.value = null
  }
}

function cancelDownload(): void {
  downloadController.value?.abort()
}

const canCompare = computed(
  () =>
    props.repoId !== null &&
    props.archive !== null &&
    comparableArchives(props.archives, props.archive).length > 0,
)

function isComparable(entry: DisplayEntry): boolean {
  return canCompare.value && entryKind(entry.type) === 'file'
}

/** A file's history needs only the archive it is in: "now" is always one more version. */
function hasHistory(entry: DisplayEntry): boolean {
  return props.repoId !== null && props.archive !== null && entryKind(entry.type) === 'file'
}

function handleDeleteWholeArchive(): void {
  if (props.archive && !props.deleting) emit('delete-archive', props.archive)
}

/**
 * The archive root as a directory entry, so the header's whole-archive
 * download and restore reuse exactly the paths the table rows use. An empty
 * path is what both endpoints read as "everything".
 */
const ROOT_ENTRY: ContentEntry = { type: 'd', path: '', size: 0, mtime: '', mode: '' }

const archiveHost = computed(() =>
  props.archive === null ? null : resolveArchiveHost(props.archive),
)

/** The parent of the directory being shown, or null at the archive root. */
const parentCrumb = computed(() => {
  const trail = breadcrumbs.value
  return trail.length > 1 ? trail[trail.length - 2] : null
})
</script>

<template>
  <div class="archive-file-browser">
    <div
      v-if="!archive"
      class="browser-placeholder"
    >
      <Folder
        :size="40"
        class="browser-placeholder-icon"
      />
      <span class="browser-placeholder-title">No archive selected</span>
      <span class="browser-placeholder-hint">Select an archive to browse its contents.</span>
    </div>

    <template v-else>
      <div class="browser-header">
        <button
          class="btn btn-sm btn-ghost browser-back"
          type="button"
          @click="emit('back')"
        >
          <ArrowLeft :size="14" />
          Archives
        </button>
        <span class="browser-title">
          <span class="browser-title-label">{{ viewLabel }}</span>
          <span class="browser-title-name">{{ archive.name }}</span>
        </span>
        <!--
          Whole-archive actions live here, spelled out. Download and restore
          used to be reachable only through the file table's "." row, and
          delete only through an icon on that row that was transparent until
          the pointer happened to land on it - so the repository screen had a
          delete nobody could find and the schedule screen had none at all.
        -->
        <span class="browser-actions">
          <button
            class="btn btn-sm btn-ghost"
            type="button"
            title="Download whole archive"
            @click="downloadEntry(ROOT_ENTRY)"
          >
            <Download :size="14" />
            Download
          </button>
          <button
            v-if="isAdmin"
            class="btn btn-sm btn-ghost"
            type="button"
            title="Restore whole archive"
            @click="openRestore([])"
          >
            <RotateCcw :size="14" />
            Restore
          </button>
          <button
            v-if="isAdmin"
            class="btn btn-sm btn-danger-text"
            type="button"
            :disabled="deleting"
            :title="deleting ? 'Deletion in progress' : 'Delete whole archive'"
            @click="handleDeleteWholeArchive"
          >
            <BaseSpinner
              v-if="deleting"
              size="sm"
            />
            <Trash2
              v-else
              :size="14"
            />
            {{ deleting ? 'Deleting...' : 'Delete' }}
          </button>
        </span>
      </div>

      <div
        v-if="archive.start"
        class="archive-meta-bar"
      >
        <span
          v-if="archiveHost"
          class="archive-meta-item"
        >
          <span class="archive-meta-label">Host</span>
          <BaseHostLink
            class="host-link"
            :hostname="archiveHost"
          />
        </span>
        <span
          v-if="repoName"
          class="archive-meta-item"
        >
          <span class="archive-meta-label">Repo</span>
          <RouterLink
            v-if="repoId !== null"
            class="repo-link"
            :to="`/repos/${repoId}`"
            >{{ repoName }}</RouterLink
          >
          <span
            v-else
            class="archive-meta-value"
            >{{ repoName }}</span
          >
        </span>
        <span class="archive-meta-item">
          <span class="archive-meta-label">Date</span>
          <span class="archive-meta-value">{{ formatDate(archive.start) }}</span>
        </span>
        <span class="archive-meta-item">
          <span class="archive-meta-label">Original</span>
          <span class="archive-meta-value">{{ formatBytes(archive.original_size) }}</span>
        </span>
        <span class="archive-meta-item">
          <span class="archive-meta-label">Dedup</span>
          <span class="archive-meta-value">{{ formatBytes(archive.deduplicated_size) }}</span>
        </span>
      </div>

      <div
        v-if="repoId !== null"
        class="browser-view"
      >
        <BaseSegmented
          v-model="view"
          :options="VIEW_OPTIONS"
          label="What to show of the archive"
        />
      </div>

      <ArchiveDatabaseDumps
        v-if="view === 'databases' && repoId !== null"
        :repo-id="repoId"
        :archive="archive"
        :can-restore="isAdmin"
      />

      <template v-else>
        <div class="browser-path">
          <button
            class="btn btn-xs btn-ghost browser-up"
            type="button"
            :disabled="parentCrumb === null"
            title="Up one level"
            aria-label="Up one level"
            @click="parentCrumb && navigateTo(parentCrumb.path)"
          >
            <CornerLeftUp :size="14" />
          </button>
          <div class="path-crumbs">
            <button
              v-for="(seg, i) in breadcrumbs"
              :key="seg.path"
              class="crumb"
              :class="{ 'crumb-last': i === breadcrumbs.length - 1 }"
              @click="navigateTo(seg.path)"
            >
              {{ seg.label }}
            </button>
          </div>
        </div>

        <ArchiveTreeSearch
          v-model:active="searching"
          :repo-id="repoId"
          :archive-name="archive.name"
          :tick-state="tickStateOfPath"
          :disabled="splitting"
          @toggle="toggleHit"
          @open="(path: string) => navigateTo(`/${path}`)"
        />

        <template v-if="searching" />
        <BaseSpinner
          v-else-if="contentsLoading"
          size="sm"
        />
        <div
          v-else-if="indexing"
          class="state-msg state-msg--inline"
        >
          <BaseSpinner size="sm" />
          Indexing archive contents -- this only happens once...
        </div>
        <div
          v-else-if="contentsError"
          class="error-banner"
        >
          {{ contentsError }}
        </div>
        <div
          v-else-if="contents.length === 0"
          class="state-msg state-msg--inline"
        >
          Empty directory.
        </div>
        <DataTable
          v-else
          v-model:filters="browserFilters"
          :value="browserEntries"
          :row-class="(data: DisplayEntry) => (isClickable(data) ? 'clickable' : '')"
          filter-display="row"
          table-class="data-table browser-table"
          resizable-columns
          column-resize-mode="expand"
          @row-click="(e: { data: DisplayEntry }) => handleRowClick(e.data)"
        >
          <Column
            header=""
            style="width: 2.5rem"
          >
            <template #header>
              <input
                type="checkbox"
                class="browser-pick"
                :checked="hereState === 'checked'"
                :indeterminate="hereState === 'partial'"
                :disabled="pickableHere.length === 0 || splitting"
                aria-label="Select everything in this folder"
                data-testid="pick-all"
                @change="toggleAllHere"
              />
            </template>
            <template #body="{ data }">
              <input
                v-if="isPickable(data)"
                type="checkbox"
                class="browser-pick"
                :checked="tickStateOf(data) === 'checked'"
                :indeterminate="tickStateOf(data) === 'partial'"
                :disabled="splitting"
                :aria-label="`Select ${data.displayName}`"
                data-testid="pick-entry"
                @click.stop
                @change="togglePicked(data)"
              />
            </template>
          </Column>
          <Column
            field="displayName"
            header="Name"
            :sortable="true"
            :show-filter-menu="false"
            style="min-width: 12rem"
          >
            <template #filter="{ filterModel, filterCallback }">
              <input
                v-model="filterModel.value"
                class="input filter-input"
                type="text"
                placeholder="Filter name..."
                @input="filterCallback()"
              />
            </template>
            <template #body="{ data }">
              <span
                class="td-name"
                :class="{ 'td-name--link': data.link }"
                :title="data.link ? linkTitle(data.path, data.link) : data.displayName"
              >
                <FolderSymlink
                  v-if="data.link && linksToFolder(data.link)"
                  :size="16"
                  class="entry-icon"
                />
                <FileSymlink
                  v-else-if="data.link"
                  :size="16"
                  class="entry-icon"
                />
                <Folder
                  v-else-if="data.isDir"
                  :size="16"
                  class="entry-icon"
                />
                <File
                  v-else
                  :size="16"
                  class="entry-icon"
                />
                <span class="name-text">{{ data.displayName }}</span>
                <span
                  v-if="changeBadge(data)"
                  class="badge entry-change"
                  :class="changeBadge(data)?.tone"
                  :title="changeBadge(data)?.title"
                  data-testid="change-badge"
                  >{{ changeBadge(data)?.label }}</span
                >
                <span
                  v-if="data.link"
                  class="link-info"
                  data-testid="entry-link"
                >
                  <span class="badge badge--info">Link</span>
                  <span
                    v-if="data.link.target !== undefined"
                    class="link-to"
                  >
                    <ArrowRight
                      :size="12"
                      class="entry-icon"
                      aria-hidden="true"
                    />
                    <button
                      v-if="linkFolder(data.link) !== null"
                      type="button"
                      class="link-target"
                      :aria-label="`Open link target ${data.link.target}`"
                      data-testid="link-target"
                      @click.stop="openLinkTarget(data)"
                    >
                      {{ data.link.target }}
                    </button>
                    <span
                      v-else
                      class="link-target link-target--away"
                      data-testid="link-target"
                      >{{ data.link.target }}</span
                    >
                  </span>
                  <span
                    v-if="data.link.resolved.state === 'not_in_archive'"
                    class="link-note"
                    >not in archive</span
                  >
                </span>
                <span
                  v-if="data.isDir && isPickable(data) && totals.get(data.path)"
                  class="muted entry-totals"
                  data-testid="folder-files"
                  >{{ filesLabel(totals.get(data.path)?.files ?? 0) }}</span
                >
              </span>
            </template>
          </Column>
          <Column
            field="displaySize"
            header="Size"
            :sortable="true"
            :show-filter-menu="false"
            style="width: 6rem"
          >
            <template #filter="{ filterModel, filterCallback }">
              <input
                v-model="filterModel.value"
                class="input filter-input"
                type="text"
                placeholder="Filter size..."
                @input="filterCallback()"
              />
            </template>
            <template #body="{ data }">
              <span class="td-size">{{ sizeLabel(data) }}</span>
            </template>
          </Column>
          <Column
            field="displayMtime"
            header="Modified"
            :sortable="true"
            :show-filter-menu="false"
            style="width: 10rem"
          >
            <template #filter="{ filterModel, filterCallback }">
              <input
                v-model="filterModel.value"
                class="input filter-input"
                type="text"
                placeholder="Filter date..."
                @input="filterCallback()"
              />
            </template>
            <template #body="{ data }">
              <span class="td-date">{{ data.displayMtime }}</span>
            </template>
          </Column>
          <Column
            header=""
            style="width: 9rem"
          >
            <template #body="{ data }">
              <span class="td-action">
                <button
                  class="btn btn-sm btn-ghost"
                  :title="
                    data.isDir
                      ? data.path
                        ? 'Download as .tar.lz4'
                        : 'Download whole archive'
                      : 'Download'
                  "
                  @click.stop="downloadEntry(data)"
                >
                  <Download :size="14" />
                </button>
                <button
                  v-if="isComparable(data)"
                  class="btn btn-sm btn-ghost"
                  title="Compare with another archive"
                  :aria-label="`Compare ${data.displayName} with another archive`"
                  data-testid="compare-file"
                  @click.stop="comparePath = data.path"
                >
                  <FileDiff :size="14" />
                </button>
                <button
                  v-if="hasHistory(data)"
                  class="btn btn-sm btn-ghost"
                  title="History, and compare with the file now"
                  :aria-label="`History of ${data.displayName}`"
                  data-testid="file-history"
                  @click.stop="historyPath = data.path"
                >
                  <History :size="14" />
                </button>
              </span>
            </template>
          </Column>
        </DataTable>

        <!--
          Restoring and downloading act on what is ticked, from any folder of
          the archive, and say where a restore goes before it is started: the
          agent the archive came from, which the dialog behind the button can
          change, together with where on it.
        -->
        <div
          v-if="picked.size > 0"
          class="browser-selection"
          role="region"
          aria-label="Selection"
          data-testid="browser-selection"
        >
          <span class="browser-selection-count">{{ pickedSummary }}</span>
          <button
            class="btn btn-xs btn-ghost"
            type="button"
            @click="clearPicked"
          >
            Clear
          </button>
          <span
            v-if="isAdmin && archiveHost"
            class="muted browser-selection-target"
            >to their original paths on {{ archiveHost }}, or elsewhere</span
          >
          <span class="browser-selection-actions">
            <button
              class="btn btn-sm btn-ghost"
              type="button"
              :disabled="downloading"
              @click="downloadPicked"
            >
              <BaseSpinner
                v-if="downloading"
                size="sm"
              />
              <Download
                v-else
                :size="14"
              />
              Download
            </button>
            <button
              v-if="downloading"
              class="btn btn-sm btn-ghost"
              type="button"
              data-testid="cancel-download"
              @click="cancelDownload"
            >
              Cancel download
            </button>
            <button
              v-if="isAdmin"
              class="btn btn-sm btn-primary"
              type="button"
              data-testid="restore-picked"
              @click="openRestore(pickedPaths)"
            >
              <RotateCcw :size="14" />
              Restore{{ archiveHost ? ` to ${archiveHost}` : '' }}
            </button>
          </span>
        </div>
      </template>
    </template>

    <RestoreDialog
      v-if="archive !== null && repoId !== null && restorePaths !== null"
      :open="restorePaths !== null"
      :repo-id="repoId"
      :archive="archive"
      :paths="restorePaths"
      @close="restorePaths = null"
      @restored="onRestored"
    />

    <FileDiffDialog
      v-if="archive !== null && repoId !== null && comparePath !== null"
      :open="comparePath !== null"
      :repo-id="repoId"
      :archive="archive"
      :archives="archives"
      :path="comparePath"
      @close="comparePath = null"
    />

    <FileHistoryDialog
      v-if="archive !== null && repoId !== null && historyPath !== null"
      :open="historyPath !== null"
      :repo-id="repoId"
      :archive="archive"
      :path="historyPath"
      @close="historyPath = null"
    />
  </div>
</template>

<style scoped>
.archive-file-browser {
  color: var(--text-primary);
  /* The table's layout follows the pane it sits in, not the viewport: beside
     a 380px archive list a 1280px desktop leaves this pane narrower than a
     phone-width breakpoint would ever notice. */
  container-type: inline-size;
}

.browser-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-5);
  min-height: 15rem;
  padding: var(--space-9);
  text-align: center;
  font-size: var(--fs-base);
  color: var(--text-muted);
}

.browser-placeholder-icon {
  opacity: 0.45;
}

.browser-placeholder-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-primary);
}

.browser-placeholder-hint {
  max-width: 32ch;
  font-size: var(--fs-sm);
}

.browser-header {
  display: flex;
  align-items: flex-start;
  flex-wrap: wrap;
  gap: var(--space-5);
  padding: var(--space-5) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.browser-title {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}

.browser-title-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.browser-title-name {
  font-family: var(--mono);
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.browser-actions {
  margin-left: auto;
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.archive-meta-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

/* Each fact reads as its own chip, so the bar stays legible when it wraps on
   a narrow pane instead of running together across a single line. */
.archive-meta-item {
  display: inline-flex;
  align-items: baseline;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-5);
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
}

.archive-meta-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.archive-meta-value {
  font-size: var(--fs-2xs);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

/* The host chip is a link into the agent; size it like its neighbours rather
   than at `.host-link`'s table-cell size. */
.archive-meta-item .host-link {
  font-size: var(--fs-2xs);
}

/* The repo chip is the same shape aimed at the repository; `.repo-link`
   carries no size of its own, so it takes the bar's. */
.archive-meta-item .repo-link {
  font-size: var(--fs-2xs);
}

.browser-view {
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.browser-path {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
}

.browser-up {
  flex: none;
}

:deep(.browser-table) {
  table-layout: fixed;
}

:deep(.data-table) {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-base);
}

/* No border-bottom here: the global PrimeVue passthrough config
   (primevue-pt.ts) already draws row separators via `tbody: divide-y` and
   `headerRow: border-b`. Adding a second border on th/td doubled the line. */
:deep(.data-table th) {
  text-align: left;
  padding: var(--space-4) var(--space-6);
  color: var(--text-muted);
  font-weight: 600;
  font-size: var(--fs-xs);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

:deep(.data-table td) {
  padding: var(--space-4) var(--space-6);
  color: var(--text-secondary);
}

:deep(.data-table tr.clickable) {
  cursor: pointer;
  transition: background var(--duration-fast);
}

:deep(.data-table tr.clickable:hover) {
  background: var(--bg-hover);
}

/* 37.5rem is the full table's own floor: the pick column (2.5rem), the Name
   column's 12rem minimum, and the fixed Size (6rem), Modified (10rem) and
   action (7rem) columns. A pane narrower than that used to keep every column
   and scroll sideways, which pushed each row's restore button past the
   panel's edge - so the table drops the Modified column (the 4th) and narrows
   Size (the 3rd) instead, keeping every row's actions in view. */
@container (max-width: 37.5rem) {
  :deep(.browser-table th:nth-child(4)),
  :deep(.browser-table td:nth-child(4)) {
    display: none;
  }

  :deep(.browser-table th:nth-child(3)),
  :deep(.browser-table td:nth-child(3)) {
    width: 4rem;
  }
}

@media (max-width: 640px) {
  .td-name {
    flex-wrap: wrap;
    align-items: flex-start;
  }

  .name-text {
    white-space: normal;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .td-size {
    white-space: normal;
  }
}

/* Only the phone layout shows one pane at a time, so only there is there a
   list to go back to. */
.browser-back {
  display: none;
}

@media (max-width: 768px) {
  .browser-back {
    display: inline-flex;
    align-self: flex-start;
  }
}

/* A link's badge and target sit on their own line under its name, lined up
   with the name past the icon: beside the archive list the Name column is
   too narrow to hold the name, the badge and the target on one line. */
.td-name--link {
  flex-wrap: wrap;
  row-gap: var(--space-2);
}

.link-info {
  display: flex;
  flex-wrap: wrap;
  flex-basis: 100%;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  padding-inline-start: var(--space-8);
  font-size: var(--fs-xs);
}

/* The arrow travels with its target, so a wrapped line never leaves it
   behind. */
.link-to {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.link-target {
  overflow: hidden;
  min-width: 0;
  padding: 0;
  border: none;
  background: none;
  color: var(--accent);
  font: inherit;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}

.link-target:hover {
  text-decoration: underline;
}

.link-target--away {
  color: var(--text-secondary);
  cursor: default;
}

.link-target--away:hover {
  text-decoration: none;
}

.link-note {
  flex: none;
  color: var(--text-muted);
  font-family: var(--sans);
}

.entry-change {
  flex: none;
}

.entry-totals {
  flex: none;
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
}

.browser-pick {
  width: 1rem;
  height: 1rem;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

/* Pinned to the bottom of the screen while the table scrolls under it, so
   what a restore will act on stays in sight. */
.browser-selection {
  position: sticky;
  bottom: 0;
  z-index: 1;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-5);
  padding: var(--space-5) var(--space-6);
  border-top: 1px solid var(--border);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-lg);
}

.browser-selection-count {
  font-weight: 600;
}

.browser-selection-target {
  font-size: var(--fs-sm);
}

.browser-selection-actions {
  display: flex;
  gap: var(--space-4);
  margin-left: auto;
}

@media (max-width: 768px) {
  .browser-selection-actions {
    flex: 1 1 100%;
    margin-left: 0;
  }

  .browser-selection-actions > .btn {
    flex: 1;
    justify-content: center;
    min-height: 2.75rem;
  }
}
</style>

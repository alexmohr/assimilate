<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch, onBeforeUnmount } from 'vue'
import { filesLabel, formatBytes, formatDate, formatDayYear } from '../utils/format'
import { extractBlobError } from '../utils/error'
import { resolveArchiveHost } from '../utils/archiveHost'
import { useToast } from '../composables/useToast'
import { useRestoreToasts } from '../composables/useRestoreToasts'
import { useClipboard } from '../composables/useClipboard'
import {
  ArrowRight,
  Copy,
  Download,
  File,
  FileDiff,
  FileSymlink,
  Folder,
  FolderSymlink,
  History,
  RotateCcw,
  Undo2,
  Search,
  Trash2,
  X,
} from '@lucide/vue'
import axios from 'axios'
import { downloadArchiveFiles, getArchiveContents } from '../api/archives'
import BaseSpinner from './BaseSpinner.vue'
import BaseHostLink from './BaseHostLink.vue'
import BaseTabs, { type TabOption } from './BaseTabs.vue'
import BrowserRow from './BrowserRow.vue'
import EmptyState from './EmptyState.vue'
import OverflowMenu from './OverflowMenu.vue'
import ArchiveDatabaseDumps from './ArchiveDatabaseDumps.vue'
import ArchiveDiff from './ArchiveDiff.vue'
import ArchiveGrowth from './ArchiveGrowth.vue'
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
  parentOf,
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

/**
 * The open archive, as one card: its name over one muted line of facts, a
 * menu of what acts on the whole archive, and four tabs - its files, what
 * changed against another archive, the database dumps it holds and where its
 * size went.
 *
 * The files tab lists a folder as two-line rows. Ticking rows (from any
 * folder, and from search results) brings up the selection bar under the
 * list, which carries the one main action - restore - and, for a single file,
 * its history and a comparison with another archive.
 */

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

/** What the card shows of the archive. */
type BrowserView = 'files' | 'changes' | 'databases' | 'growth'

const VIEW_TABS: readonly TabOption<BrowserView>[] = [
  { id: 'files', label: 'Files' },
  { id: 'changes', label: 'Changes' },
  { id: 'databases', label: 'Databases' },
  { id: 'growth', label: 'Growth' },
]

/** A file or folder to show and tick once its archive opens, e.g. one a search found. */
export interface ArchiveReveal {
  /** `archiveKey` of the archive it is in. */
  archiveKey: string
  /** Its archive path, without a leading slash. */
  path: string
  isDir: boolean
}

const props = withDefaults(
  defineProps<{
    repoId: number | null
    archive: ArchiveEntry | null
    /**
     * The repository `repoId` names. Browsing, restoring and deleting all act
     * against one repository, and a schedule that copies into several writes
     * an archive of the same name into each of them, so the card has to say
     * which copy is open.
     */
    repoName?: string
    isAdmin?: boolean
    // Whether `archive` already has a delete in flight - deletion is async
    // (the request just enqueues the borg job), so without this the delete
    // entry stays clickable for an archive that's already being removed.
    deleting?: boolean
    /**
     * Whether `archive` is waiting for its append-only repository's next
     * maintenance run: the menu then offers to keep it instead of deleting it.
     */
    queued?: boolean
    /** A Keep is in flight. */
    keeping?: boolean
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
     * The other archives of the open one's repository - the list it was
     * picked from. Changes, Compare and Growth compare with these; without a
     * second archive there is nothing to compare.
     */
    archives?: readonly ArchiveEntry[]
    /** A path to show and tick once the archive it names is open. */
    reveal?: ArchiveReveal | null
    /** `archiveKey` of `archive`, to match `reveal` against. */
    archiveKey?: string | null
  }>(),
  {
    repoName: '',
    isAdmin: false,
    deleting: false,
    queued: false,
    keeping: false,
    pathQueryKey: DEFAULT_ARCHIVE_PATH_QUERY_KEY,
    archives: () => [],
    reveal: null,
    archiveKey: null,
  },
)

const emit = defineEmits<{
  'delete-archive': [archive: ArchiveEntry]
  /** Takes the queued deletion of `archive` back. */
  'keep-archive': [archive: ArchiveEntry]
  /** `reveal` has been shown, so the caller can let go of it. */
  revealed: []
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

/** Whether the search box under the path bar is open. */
const searchOpen = ref(false)

/** Whether a search is entered, so its results stand in for the folder's listing. */
const searching = ref(false)

/** The paths the open restore dialog restores (empty: the whole archive), or null. */
const restorePaths = ref<readonly string[] | null>(null)

const { success: toastSuccess, error: toastError } = useToast()
const { track: trackRestore } = useRestoreToasts()
const { copy } = useClipboard()

const repoIdRef = computed(() => props.repoId ?? 0)
const browser = useArchiveBrowser(repoIdRef)

const breadcrumbs = browser.breadcrumbs
const contents = browser.contents
const contentsLoading = browser.contentsLoading
const contentsError = browser.contentsError
const indexing = browser.indexing
const downloadEntry = browser.downloadEntry
const browserEntries = browser.browserEntries

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

/** A folder opens, a link to a folder opens its target, a file ticks. */
function handleRowClick(entry: DisplayEntry): void {
  if (entry.isDir) {
    navigateTo(entry.path)
  } else if (entry.link !== null && linksToFolder(entry.link)) {
    openLinkTarget(entry)
  } else if (!splitting.value) {
    togglePicked(entry)
  }
}

/** Opens the folder showing a link's target, when the archive holds it. */
function openLinkTarget(entry: DisplayEntry): void {
  const folder = entry.link === null ? null : linkFolder(entry.link)
  if (folder !== null) navigateTo(folder)
}

function opensFolder(entry: DisplayEntry): boolean {
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
    searchOpen.value = false
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

/**
 * A file found by the list's search: once its archive is the open one, its
 * folder opens with it ticked. Runs after the archive watcher above, which
 * clears the selection a new archive starts with.
 */
watch(
  () => [props.reveal, props.archiveKey] as const,
  ([target, key]) => {
    if (target === null || target.archiveKey !== key || props.archive === null) return
    view.value = 'files'
    const folder = target.isDir ? target.path : parentOf(target.path)
    navigateTo(`/${folder}`)
    if (!target.isDir) {
      const entry: ContentEntry = { type: '-', path: target.path, size: 0, mtime: '', mode: '' }
      rememberEntries([entry])
      applySelection([target.path])
    }
    emit('revealed')
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  browser.stopPolling()
  // The explorer releases both keys at once when it owns the archive key: two
  // replaces in one tick would cancel each other.
  if (props.routeArchiveName === undefined) pathRoute.release()
  downloadController.value?.abort()
})

/**
 * The current folder's own "." row stands for the folder, and the ".." row
 * for the one above it: neither is an entry in it - the path bar above the
 * rows does their job - and picking ".." would restore the whole parent.
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
  // A file picked before its folder was listed (a search hit) learns its size.
  const known = new Map(picked.value)
  let changed = false
  for (const entry of entries) {
    if (known.has(entry.path) && known.get(entry.path) !== entry) {
      known.set(entry.path, entry)
      changed = true
    }
  }
  if (changed) picked.value = known
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

/** The rows of the folder shown: its folders first, then its files, each by name. */
const rows = computed(() => browserEntries.value.filter(isPickable))

/** The folder's own tick state, shown by the path bar's checkbox. */
const hereState = computed<TickState>(() => {
  const states = rows.value.map(tickStateOf)
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
        rows.value.map((entry) => entry.path),
        childrenOf,
      ),
    )
  }
}

function clearPicked(): void {
  picked.value = new Map()
}

function toggleSearch(): void {
  searchOpen.value = !searchOpen.value
}

const treeTotals = useArchiveTreeTotals(
  () => props.repoId,
  () => props.archive?.name ?? null,
)
const totals = treeTotals.totals

// A folder's size is what is under it, which the listing does not say.
watch(
  () => (props.archive === null || contents.value.length === 0 ? [] : rows.value),
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

/** How a row's change against the previous archive reads and looks. */
interface ChangeMark {
  label: string
  dot: string
  title: string
}

const CHANGE_MARKS: Record<ChangeKind, Omit<ChangeMark, 'title'>> = {
  new: { label: 'new', dot: 'status-dot--success' },
  changed: { label: 'changed', dot: 'status-dot--warning' },
}

/** A file's change against the previous archive, for its row. */
function changeMark(entry: DisplayEntry): ChangeMark | null {
  const change = changes.changeOf(entry)
  if (change === null) return null
  const mark = CHANGE_MARKS[change]
  return { ...mark, title: `${mark.label} since ${changes.previous.value?.name ?? ''}` }
}

/** A file's own size; a folder's, what the index says is under it, once it has said. */
function sizeLabel(entry: DisplayEntry): string | null {
  if (entry.link !== null) return null
  if (!entry.isDir) return entry.displaySize
  const total = totals.value.get(entry.path)
  return total === undefined ? null : formatBytes(total.bytes)
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

/** The one file ticked, when exactly one is: what History and Compare act on. */
const onlyFile = computed<ContentEntry | null>(() => {
  const entries = [...picked.value.values()]
  const [only] = entries
  if (entries.length !== 1 || only === undefined) return null
  // Ticking the only file of a folder collapses the selection into the
  // folder, and up through every folder that holds nothing else; that is
  // still one file, so walk back down to it for its comparison and history.
  let entry: ContentEntry | undefined = only
  while (entry !== undefined && entryKind(entry.type) !== 'file') {
    const children = childrenOf(entry.path)
    const [child] = children ?? []
    if (children?.length !== 1 || child === undefined) return null
    entry = seenEntries.value.get(child)
  }
  return entry ?? null
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
  const only = onlyFile.value
  if (only !== null) {
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

function handleDeleteWholeArchive(): void {
  if (props.archive && !props.deleting) emit('delete-archive', props.archive)
}

function handleKeepWholeArchive(): void {
  if (props.archive && !props.keeping) emit('keep-archive', props.archive)
}

async function copyName(): Promise<void> {
  if (props.archive === null) return
  try {
    await copy(props.archive.name)
    toastSuccess('Archive name copied')
  } catch (e: unknown) {
    toastError(extractError(e))
  }
}

/**
 * The archive root as a directory entry, so the menu's whole-archive download
 * reuses exactly the paths the rows use. An empty path is what the endpoint
 * reads as "everything".
 */
const ROOT_ENTRY: ContentEntry = { type: 'd', path: '', size: 0, mtime: '', mode: '' }

const archiveHost = computed(() =>
  props.archive === null ? null : resolveArchiveHost(props.archive),
)

/** The facts under a row's name: a folder's file count, a file's modification day. */
function rowFacts(entry: DisplayEntry): string | null {
  if (entry.isDir) {
    const total = totals.value.get(entry.path)
    return total === undefined ? 'folder' : filesLabel(total.files)
  }
  return entry.mtime ? `modified ${formatDayYear(entry.mtime)}` : null
}
</script>

<template>
  <div class="archive-file-browser">
    <EmptyState
      v-if="!archive"
      :icon="Folder"
      title="Pick an archive"
      description="Its files, the changes since the archive before it, its database dumps and its growth open here."
    />

    <template v-else>
      <div class="archive-head">
        <div class="archive-head-text">
          <h2 class="archive-head-name">{{ archive.name }}</h2>
          <p class="archive-head-facts">
            <template v-if="archiveHost">
              <BaseHostLink
                class="archive-head-link"
                :hostname="archiveHost"
              />
              <span aria-hidden="true">&middot;</span>
            </template>
            <template v-if="repoName">
              <RouterLink
                v-if="repoId !== null"
                class="archive-head-link"
                :to="`/repos/${repoId}`"
                >{{ repoName }}</RouterLink
              >
              <span v-else>{{ repoName }}</span>
              <span aria-hidden="true">&middot;</span>
            </template>
            <span>{{ formatDate(archive.start) }}</span>
            <span aria-hidden="true">&middot;</span>
            <span>{{ formatBytes(archive.original_size) }}</span>
            <span aria-hidden="true">&middot;</span>
            <span>+{{ formatBytes(archive.deduplicated_size) }} new</span>
            <template v-if="deleting">
              <span aria-hidden="true">&middot;</span>
              <span class="archive-head-deleting">
                <BaseSpinner size="sm" />
                Deleting
              </span>
            </template>
            <template v-else-if="queued">
              <span aria-hidden="true">&middot;</span>
              <span class="archive-head-queued">Removed at next maintenance</span>
            </template>
          </p>
        </div>
        <!--
          Everything that acts on the whole archive is in here, so the open
          archive carries one menu instead of three header buttons and the list
          a bin on every row. Restoring single files is the selection bar's.
        -->
        <OverflowMenu
          v-slot="{ run }"
          label="Archive actions"
        >
          <button
            v-if="isAdmin"
            class="overflow-menu-item archive-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => openRestore([]))"
          >
            <RotateCcw :size="14" />
            Restore whole archive...
          </button>
          <button
            class="overflow-menu-item archive-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => downloadEntry(ROOT_ENTRY))"
          >
            <Download :size="14" />
            Download
          </button>
          <button
            v-if="repoId !== null"
            class="overflow-menu-item archive-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => (view = 'changes'))"
          >
            <FileDiff :size="14" />
            Compare with another archive
          </button>
          <button
            class="overflow-menu-item archive-menu-item"
            role="menuitem"
            type="button"
            @click="run(copyName)"
          >
            <Copy :size="14" />
            Copy archive name
          </button>
          <button
            v-if="isAdmin && queued"
            class="overflow-menu-item archive-menu-item"
            role="menuitem"
            type="button"
            :disabled="keeping"
            @click="run(handleKeepWholeArchive)"
          >
            <Undo2 :size="14" />
            Keep archive
          </button>
          <button
            v-else-if="isAdmin"
            class="overflow-menu-item overflow-menu-item--danger archive-menu-item"
            role="menuitem"
            type="button"
            :disabled="deleting"
            :title="deleting ? 'Deletion in progress' : undefined"
            @click="run(handleDeleteWholeArchive)"
          >
            <Trash2 :size="14" />
            {{ deleting ? 'Deleting...' : 'Delete archive...' }}
          </button>
        </OverflowMenu>
      </div>

      <BaseTabs
        v-if="repoId !== null"
        v-model="view"
        class="archive-tabs"
        :tabs="VIEW_TABS"
        label="What to show of the archive"
      />

      <ArchiveDiff
        v-if="view === 'changes' && repoId !== null"
        :repo-id="repoId"
        :archive="archive"
        :archives="archives"
      />

      <ArchiveDatabaseDumps
        v-else-if="view === 'databases' && repoId !== null"
        :repo-id="repoId"
        :archive="archive"
        :can-restore="isAdmin"
      />

      <ArchiveGrowth
        v-else-if="view === 'growth' && repoId !== null"
        :repo-id="repoId"
        :archive="archive"
        :archives="archives"
      />

      <template v-else>
        <div class="archive-pathbar">
          <input
            type="checkbox"
            class="browser-pick"
            :checked="hereState === 'checked'"
            :indeterminate="hereState === 'partial'"
            :disabled="rows.length === 0 || splitting || searching"
            aria-label="Select everything in this folder"
            title="Select everything in this folder"
            data-testid="pick-all"
            @change="toggleAllHere"
          />
          <nav
            class="path-crumbs archive-path"
            aria-label="Folder"
          >
            <button
              v-for="(seg, i) in breadcrumbs"
              :key="seg.path"
              type="button"
              class="crumb"
              :class="{
                'crumb-last': i === breadcrumbs.length - 1,
                'crumb-root': seg.path === '/',
              }"
              :aria-current="i === breadcrumbs.length - 1 ? 'location' : undefined"
              @click="navigateTo(seg.path)"
            >
              {{ seg.path === '/' ? '/' : seg.label }}
            </button>
          </nav>
          <button
            class="btn btn-sm btn-ghost archive-search-toggle"
            :class="{ 'archive-search-toggle--open': searchOpen }"
            type="button"
            :aria-expanded="searchOpen"
            :aria-label="searchOpen ? 'Close the search' : 'Search this archive'"
            :title="searchOpen ? 'Close the search' : 'Search this archive'"
            data-testid="tree-search-toggle"
            @click="toggleSearch"
          >
            <X
              v-if="searchOpen"
              :size="14"
            />
            <Search
              v-else
              :size="14"
            />
          </button>
        </div>

        <ArchiveTreeSearch
          v-if="searchOpen"
          v-model:active="searching"
          :repo-id="repoId"
          :archive-name="archive.name"
          :tick-state="tickStateOfPath"
          :disabled="splitting"
          @toggle="toggleHit"
          @open="(path: string) => navigateTo(`/${path}`)"
        />

        <div
          v-if="!searching"
          class="archive-rows"
        >
          <div
            v-if="contentsLoading"
            class="loading-row"
          >
            <BaseSpinner size="sm" />
          </div>
          <div
            v-else-if="indexing"
            class="state-msg state-msg--inline archive-rows-msg"
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
            v-else-if="rows.length === 0"
            class="state-msg state-msg--inline archive-rows-msg"
          >
            This folder is empty.
          </div>
          <template v-else>
            <BrowserRow
              v-for="entry in rows"
              :key="entry.path"
              clickable
              :selected="tickStateOf(entry) === 'checked'"
              :size="sizeLabel(entry)"
              data-testid="browser-entry"
              @click="handleRowClick(entry)"
            >
              <template #lead>
                <input
                  type="checkbox"
                  class="browser-pick"
                  :checked="tickStateOf(entry) === 'checked'"
                  :indeterminate="tickStateOf(entry) === 'partial'"
                  :disabled="splitting"
                  :aria-label="`Select ${entry.displayName}`"
                  data-testid="pick-entry"
                  @click.stop
                  @change="togglePicked(entry)"
                />
                <FolderSymlink
                  v-if="entry.link && linksToFolder(entry.link)"
                  :size="16"
                  class="browser-icon browser-icon--dir"
                />
                <FileSymlink
                  v-else-if="entry.link"
                  :size="16"
                  class="browser-icon"
                />
                <Folder
                  v-else-if="entry.isDir"
                  :size="16"
                  class="browser-icon browser-icon--dir"
                />
                <File
                  v-else
                  :size="16"
                  class="browser-icon"
                />
              </template>
              <template #title>
                <button
                  v-if="opensFolder(entry)"
                  type="button"
                  class="browser-open"
                  :title="entry.link ? linkTitle(entry.path, entry.link) : undefined"
                  @click.stop="handleRowClick(entry)"
                >
                  {{ entry.displayName }}
                </button>
                <span
                  v-else
                  :title="entry.link ? linkTitle(entry.path, entry.link) : undefined"
                  >{{ entry.displayName }}</span
                >
              </template>
              <template #facts>
                <template v-if="changeMark(entry)">
                  <span
                    class="browser-change"
                    :title="changeMark(entry)?.title"
                    data-testid="change-badge"
                  >
                    <span
                      class="status-dot"
                      :class="changeMark(entry)?.dot"
                    />
                    {{ changeMark(entry)?.label }}
                  </span>
                  <span aria-hidden="true">&middot;</span>
                </template>
                <span
                  v-if="entry.link"
                  class="browser-link"
                  data-testid="entry-link"
                >
                  link
                  <template v-if="entry.link.target !== undefined">
                    <ArrowRight
                      :size="12"
                      aria-hidden="true"
                    />
                    <button
                      v-if="linkFolder(entry.link) !== null"
                      type="button"
                      class="browser-link-target"
                      :aria-label="`Open link target ${entry.link.target}`"
                      data-testid="link-target"
                      @click.stop="openLinkTarget(entry)"
                    >
                      {{ entry.link.target }}
                    </button>
                    <span
                      v-else
                      class="mono"
                      data-testid="link-target"
                      >{{ entry.link.target }}</span
                    >
                  </template>
                  <template v-if="entry.link.resolved.state === 'not_in_archive'">
                    (not in archive)
                  </template>
                </span>
                <span
                  v-else-if="rowFacts(entry)"
                  :title="entry.mtime ? formatDate(entry.mtime) : undefined"
                  :data-testid="entry.isDir ? 'folder-files' : undefined"
                  >{{ rowFacts(entry) }}</span
                >
              </template>
            </BrowserRow>
          </template>
        </div>

        <!--
          Restoring and downloading act on what is ticked, from any folder of
          the archive, and the bar says where a restore goes before it is
          started: the machine the archive came from, which the dialog behind
          the button can change, together with where on it.
        -->
        <div
          v-if="picked.size > 0"
          class="selection-bar"
          role="region"
          aria-label="Selection"
          data-testid="browser-selection"
        >
          <span class="selection-text">
            <span class="selection-count">{{ pickedSummary }}</span>
            <span
              v-if="isAdmin && archiveHost"
              class="selection-target"
              >Restores to {{ archiveHost }}, at the original paths or elsewhere</span
            >
          </span>
          <span class="selection-actions">
            <button
              class="btn btn-sm btn-ghost"
              type="button"
              @click="clearPicked"
            >
              Clear
            </button>
            <button
              v-if="onlyFile !== null && repoId !== null"
              class="btn btn-sm btn-ghost"
              type="button"
              title="Versions of this file across archives, and compare with the file now"
              data-testid="file-history"
              @click="historyPath = onlyFile?.path ?? null"
            >
              <History :size="14" />
              History
            </button>
            <button
              v-if="onlyFile !== null && canCompare"
              class="btn btn-sm btn-ghost"
              type="button"
              title="Compare with another archive"
              data-testid="compare-file"
              @click="comparePath = onlyFile?.path ?? null"
            >
              <FileDiff :size="14" />
              Compare
            </button>
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
              Restore...
            </button>
          </span>
        </div>
        <p
          v-else-if="!searching"
          class="selection-hint"
        >
          Tick files or folders to restore or download them. Open a folder by clicking its name.
        </p>
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
}

.archive-head {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
  padding: var(--space-5) var(--space-7);
}

.archive-head-text {
  flex: 1;
  min-width: 0;
}

.archive-head-name {
  margin: 0;
  font-family: var(--mono);
  font-size: var(--fs-base);
  font-weight: 600;
  overflow-wrap: anywhere;
}

.archive-head-facts {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-3);
  margin: var(--space-2) 0 0;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.archive-head-link {
  color: inherit;
  text-decoration: none;
}

.archive-head-link:hover {
  color: var(--accent);
  text-decoration: underline;
}

.archive-head-deleting {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  color: var(--danger);
}

.archive-head-queued {
  color: var(--violet);
}

.archive-menu-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  white-space: nowrap;
}

/* The tabs sit inside the card, ruled off from the head above them, and
   wrap rather than scroll on a phone: there are only four. */
.archive-tabs {
  flex-wrap: wrap;
  padding: 0 var(--space-4);
  border-top: 1px solid var(--border);
}

.archive-tabs :deep(.tablist) {
  flex-wrap: wrap;
}

.archive-tabs :deep(.tab) {
  padding: var(--space-4) var(--space-3);
  margin: 0 var(--space-3);
  font-size: var(--fs-sm);
}

@media (max-width: 768px) {
  .archive-tabs {
    overflow-x: visible;
  }
}

.archive-pathbar {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: var(--space-3) var(--space-5) var(--space-3) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.archive-path {
  flex: 1;
  min-width: 0;
  padding: 0;
  border: 0;
  background: none;
}

.archive-search-toggle {
  flex: none;
}

.archive-search-toggle--open {
  color: var(--accent);
}

/* The listing scrolls inside the card beside the archive list, so the
   selection bar under it stays in view. On a phone the page scrolls. */
.archive-rows {
  max-height: 36rem;
  overflow-y: auto;
}

@media (max-width: 768px) {
  .archive-rows {
    max-height: none;
  }
}

.archive-rows-msg {
  padding: var(--space-5) var(--space-7);
}

.browser-pick {
  flex: none;
  width: 1rem;
  height: 1rem;
  margin: 0;
  accent-color: var(--accent);
  cursor: pointer;
}

.browser-icon {
  flex: none;
  color: var(--text-muted);
}

.browser-icon--dir {
  color: var(--accent);
}

/* A folder's name is the button that opens it, so the keyboard can too. */
.browser-open {
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  color: inherit;
  text-align: left;
  overflow-wrap: anywhere;
  cursor: pointer;
}

.browser-open:hover {
  color: var(--accent);
}

.browser-change {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
}

.browser-link {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
  min-width: 0;
}

.browser-link-target {
  padding: 0;
  border: 0;
  background: none;
  font-family: var(--mono);
  font-size: inherit;
  color: var(--accent);
  overflow-wrap: anywhere;
  cursor: pointer;
}

.browser-link-target:hover {
  text-decoration: underline;
}

/* Pinned to the bottom of the screen while a long folder scrolls the page
   on a phone, so what a restore will act on stays in sight. */
.selection-bar {
  position: sticky;
  bottom: 0;
  z-index: 1;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-5);
  padding: var(--space-5) var(--space-7);
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius) var(--radius);
  background: var(--bg-card);
}

.selection-text {
  display: flex;
  flex: 1 1 12rem;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
  font-size: var(--fs-sm);
}

.selection-count {
  font-weight: 600;
}

.selection-target {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.selection-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-3);
  margin-left: auto;
}

.selection-hint {
  margin: 0;
  padding: var(--space-4) var(--space-7);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>

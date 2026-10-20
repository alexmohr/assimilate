<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch, onBeforeUnmount } from 'vue'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { resolveArchiveHost } from '../utils/archiveHost'
import { useToast } from '../composables/useToast'
import { useRestoreToasts } from '../composables/useRestoreToasts'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import {
  ArrowLeft,
  CornerLeftUp,
  Download,
  File,
  FileDiff,
  Folder,
  RotateCcw,
  Trash2,
} from '@lucide/vue'
import axios from 'axios'
import { downloadArchiveFiles } from '../api/archives'
import BaseSpinner from './BaseSpinner.vue'
import BaseHostLink from './BaseHostLink.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import ArchiveDatabaseDumps from './ArchiveDatabaseDumps.vue'
import FileDiffDialog from './FileDiffDialog.vue'
import RestoreDialog from './RestoreDialog.vue'
import { comparableArchives, entryKind } from '../utils/fileDiff'
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

/**
 * The entries picked for a restore or download, by path, across every folder
 * of the archive: picking a file, going up a level and picking another keeps
 * both, so one restore can bring back things that sit in different places.
 */
const picked = ref<Map<string, DisplayEntry>>(new Map())

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
  }
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

watch(
  () => props.archive,
  (archive, previous) => {
    // Another archive opens on its files, as a fresh selection does.
    view.value = 'files'
    comparePath.value = null
    picked.value = new Map()
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
})

/** The current folder's own "." row stands for the folder, not an entry in it. */
function isPickable(entry: DisplayEntry): boolean {
  return entry.displayName !== CURRENT_DIR_MARKER && entry.path.length > 0
}

function isPicked(entry: DisplayEntry): boolean {
  return picked.value.has(entry.path)
}

function togglePicked(entry: DisplayEntry): void {
  const next = new Map(picked.value)
  if (next.has(entry.path)) next.delete(entry.path)
  else next.set(entry.path, entry)
  picked.value = next
}

const pickableHere = computed(() => browserEntries.value.filter(isPickable))

const allHerePicked = computed(
  () => pickableHere.value.length > 0 && pickableHere.value.every(isPicked),
)

function toggleAllHere(): void {
  const next = new Map(picked.value)
  if (allHerePicked.value) pickableHere.value.forEach((entry) => next.delete(entry.path))
  else pickableHere.value.forEach((entry) => next.set(entry.path, entry))
  picked.value = next
}

function clearPicked(): void {
  picked.value = new Map()
}

const pickedPaths = computed(() => [...picked.value.keys()])

/**
 * What the selection amounts to. A folder's size is what is inside it, which
 * the listing does not say, so a selection holding one only names its count.
 */
const pickedSummary = computed(() => {
  const entries = [...picked.value.values()]
  const count = `${entries.length} selected`
  if (entries.some((entry) => entry.isDir)) return count
  return `${count} · ${formatBytes(entries.reduce((total, entry) => total + entry.size, 0))}`
})

function openRestore(paths: readonly string[]): void {
  restorePaths.value = paths
}

function onRestored(run: RestoreRun): void {
  trackRestore(run)
  picked.value = new Map()
}

const downloading = ref(false)

/** One file downloads as itself; anything more as one tar.lz4 the server streams. */
async function downloadPicked(): Promise<void> {
  const archive = props.archive
  if (archive === null || props.repoId === null) return
  const entries = [...picked.value.values()]
  const [only] = entries
  if (entries.length === 1 && only !== undefined && !only.isDir) {
    downloadEntry(only)
    return
  }
  downloading.value = true
  try {
    const blob = await downloadArchiveFiles(props.repoId, archive.name, pickedPaths.value)
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${archive.name}.tar.lz4`
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
  } catch (e: unknown) {
    if (!axios.isCancel(e)) toastError(extractError(e))
  } finally {
    downloading.value = false
  }
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

        <BaseSpinner
          v-if="contentsLoading"
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
          :row-class="(data: DisplayEntry) => (data.isDir ? 'clickable' : '')"
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
                :checked="allHerePicked"
                :disabled="pickableHere.length === 0"
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
                :checked="isPicked(data)"
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
                :title="data.displayName"
              >
                <Folder
                  v-if="data.isDir"
                  :size="16"
                  class="entry-icon"
                />
                <File
                  v-else
                  :size="16"
                  class="entry-icon"
                />
                <span class="name-text">{{ data.displayName }}</span>
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
              <span class="td-size">{{ data.isDir ? '-' : data.displaySize }}</span>
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
            style="width: 7rem"
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

/* 35rem is the full table's own floor: the Name column's 12rem minimum plus
   the fixed Size (6rem), Modified (10rem) and action (7rem) columns. A pane
   narrower than that used to keep all four columns and scroll sideways, which
   pushed each row's restore button past the panel's edge - so the table drops
   the Modified column and narrows Size instead, keeping every row's actions
   in view. */
@container (max-width: 35rem) {
  :deep(.browser-table th:nth-child(3)),
  :deep(.browser-table td:nth-child(3)) {
    display: none;
  }

  :deep(.browser-table th:nth-child(2)),
  :deep(.browser-table td:nth-child(2)) {
    width: 4rem;
  }
}

@media (max-width: 640px) {
  .td-name {
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

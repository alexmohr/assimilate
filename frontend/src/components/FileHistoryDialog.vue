<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { FileDiff, History, Lock } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import FileDiffView from './FileDiffView.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import {
  diffArchiveFile,
  diffArchiveFileWithLive,
  getFileHistory,
  indexFileHistory,
} from '../api/archives'
import { extractError } from '../utils/error'
import { formatBytes, formatDateShort } from '../utils/format'
import { unavailableReason, type DiffLayout } from '../utils/fileDiff'
import {
  LIVE,
  archiveCount,
  archiveSpan,
  defaultPair,
  listedEntries,
  versionChoices,
  type ListedEntry,
} from '../utils/fileHistory'
import type {
  ArchiveEntryResponse,
  FileDiffResponse,
  FileHistoryResponse,
} from '../types/generated'

/**
 * Every version of one file across the archives its host wrote into the
 * repository, plus the file as it is on the host now. Any two can be
 * compared: the "From" side's lines are the removed ones, the "To" side's
 * the added ones.
 */
const props = defineProps<{
  open: boolean
  repoId: number
  /** The archive the history was opened from. */
  archive: ArchiveEntryResponse
  /** The file's path within the archive. */
  path: string
}>()

const emit = defineEmits<{ close: [] }>()

const LAYOUT_OPTIONS: readonly SegmentedOption<DiffLayout>[] = [
  { value: 'split', label: 'Side by side' },
  { value: 'inline', label: 'Inline' },
]

const WIDE_ENOUGH_FOR_SPLIT = '(min-width: 768px)'
/** How often, and how many times, the history is read again while indexing runs. */
const INDEX_POLL_MS = 3000
const INDEX_POLL_LIMIT = 40

const history = ref<FileHistoryResponse | null>(null)
const historyLoading = ref(false)
const historyError = ref<string | null>(null)
const from = ref<string | null>(null)
const to = ref<string | null>(null)
const layout = ref<DiffLayout>('inline')
const wholeFile = ref(false)
const diffLoading = ref(false)
const diffError = ref<string | null>(null)
const result = ref<FileDiffResponse | null>(null)
const indexing = ref(false)
const indexError = ref<string | null>(null)

const fileName = computed(() => props.path.split('/').pop() ?? props.path)
const entries = computed<ListedEntry[]>(() =>
  history.value === null ? [] : listedEntries(history.value),
)
const choices = computed(() =>
  history.value === null ? [] : versionChoices(history.value, props.archive.name),
)
const hostname = computed(() => history.value?.hostname ?? props.archive.hostname)
const notIndexed = computed(() =>
  entries.value
    .filter((entry) => entry.kind === 'not_indexed')
    .reduce((count, entry) => count + entry.archives.length, 0),
)
/** The opened archive, when the index cannot place it in the history yet. */
const openedExtra = computed(() =>
  choices.value.find(
    (choice) =>
      choice.key === props.archive.name &&
      !entries.value.some((entry) => entry.kind === 'version' && entry.key === choice.key),
  ),
)

const textResult = computed(() =>
  result.value !== null && result.value.status === 'text' ? result.value : null,
)

function sideLabel(key: string | null): string {
  if (key === null) return ''
  if (key === LIVE) return `Now, on ${hostname.value}`
  const entry = entries.value.find((candidate) => candidate.key === key)
  return entry === undefined ? formatDateShort(props.archive.start) : archiveSpan(entry)
}

const unavailable = computed<string | null>(() => {
  const response = result.value
  if (response === null || response.status === 'text') return null
  return unavailableReason(
    response,
    { base: sideLabel(from.value), target: sideLabel(to.value) },
    hostname.value,
  )
})

function archiveFor(key: string | null): string | null {
  return choices.value.find((choice) => choice.key === key)?.archiveName ?? null
}

// Answers to an earlier request (another pair picked meanwhile) are dropped.
let diffSeq = 0

async function loadDiff(): Promise<void> {
  const fromArchive = archiveFor(from.value)
  if (!props.open || fromArchive === null || to.value === null || from.value === to.value) {
    result.value = null
    return
  }
  const seq = ++diffSeq
  diffLoading.value = true
  diffError.value = null
  try {
    const options = { path: props.path, wholeFile: wholeFile.value }
    const toArchive = archiveFor(to.value)
    const response =
      to.value === LIVE || toArchive === null
        ? await diffArchiveFileWithLive(props.repoId, fromArchive, options)
        : await diffArchiveFile(props.repoId, toArchive, { ...options, against: fromArchive })
    if (seq !== diffSeq) return
    result.value = response
  } catch (e: unknown) {
    if (seq !== diffSeq) return
    result.value = null
    diffError.value = extractError(e)
  } finally {
    if (seq === diffSeq) diffLoading.value = false
  }
}

async function loadHistory(pickDefaults: boolean): Promise<void> {
  historyLoading.value = history.value === null
  historyError.value = null
  try {
    history.value = await getFileHistory(props.repoId, props.archive.name, props.path)
    if (pickDefaults) {
      const pair = defaultPair(choices.value)
      from.value = pair.from
      to.value = pair.to
      void loadDiff()
    }
  } catch (e: unknown) {
    historyError.value = extractError(e)
  } finally {
    historyLoading.value = false
  }
}

let pollTimer: ReturnType<typeof setTimeout> | null = null
let polls = 0

function stopPolling(): void {
  if (pollTimer !== null) clearTimeout(pollTimer)
  pollTimer = null
}

function pollWhileIndexing(): void {
  stopPolling()
  if (notIndexed.value === 0 || polls >= INDEX_POLL_LIMIT) {
    indexing.value = false
    return
  }
  pollTimer = setTimeout(() => {
    polls += 1
    void loadHistory(false).then(pollWhileIndexing)
  }, INDEX_POLL_MS)
}

async function indexRemaining(): Promise<void> {
  indexError.value = null
  indexing.value = true
  try {
    await indexFileHistory(props.repoId, props.archive.name)
    polls = 0
    pollWhileIndexing()
  } catch (e: unknown) {
    indexing.value = false
    indexError.value = extractError(e)
  }
}

function pick(side: 'from' | 'to', key: string): void {
  if (side === 'from') from.value = key
  else to.value = key
}

function isPicked(side: 'from' | 'to', key: string): boolean {
  return (side === 'from' ? from.value : to.value) === key
}

function entryTitle(entry: ListedEntry): string {
  if (entry.kind === 'absent') return 'Not in these archives'
  if (entry.kind === 'not_indexed') return 'Not indexed yet'
  return archiveSpan(entry)
}

watch(
  () => [props.open, props.path, props.archive.name] as const,
  ([open]) => {
    stopPolling()
    if (!open) return
    history.value = null
    result.value = null
    from.value = null
    to.value = null
    wholeFile.value = false
    indexing.value = false
    layout.value = window.matchMedia(WIDE_ENOUGH_FOR_SPLIT).matches ? 'split' : 'inline'
    void loadHistory(true)
  },
  { immediate: true },
)

watch([from, to, wholeFile], () => {
  void loadDiff()
})

onBeforeUnmount(stopPolling)
</script>

<template>
  <BaseModal
    :open="open"
    :title="`History of ${fileName}`"
    size="lg"
    @close="emit('close')"
  >
    <div class="file-history">
      <p class="mono muted file-history-path">{{ path }}</p>

      <div
        v-if="historyLoading"
        class="loading-row"
      >
        <BaseSpinner size="md" />
      </div>
      <div
        v-else-if="historyError"
        class="error-banner"
      >
        {{ historyError }}
      </div>
      <template v-else-if="history !== null">
        <ul
          class="file-history-list"
          aria-label="Versions"
          data-testid="file-history-list"
        >
          <li
            class="file-history-item"
            :class="{ 'file-history-item--off': !history.agent_online }"
            data-testid="file-history-live"
          >
            <span class="file-history-what">
              <span class="file-history-title">
                <Lock
                  v-if="history.agent_online"
                  :size="14"
                  class="file-history-lock"
                />
                Now
              </span>
              <span class="muted">
                {{
                  history.agent_online
                    ? `Read from ${history.hostname} over the encrypted agent channel`
                    : `${history.hostname} is offline`
                }}
              </span>
            </span>
            <span class="file-history-picks">
              <button
                type="button"
                class="btn btn-sm btn-ghost"
                :aria-pressed="isPicked('to', LIVE)"
                :disabled="!history.agent_online"
                data-testid="file-history-to-live"
                @click="pick('to', LIVE)"
              >
                To
              </button>
            </span>
          </li>
          <li
            v-for="entry in entries"
            :key="entry.key"
            class="file-history-item"
            :class="{ 'file-history-item--off': entry.kind !== 'version' }"
            :data-testid="`file-history-${entry.kind}`"
            :data-archives="entry.archives.map((archive) => archive.name).join(' ')"
          >
            <span class="file-history-what">
              <span class="file-history-title">{{ entryTitle(entry) }}</span>
              <span class="muted">
                {{ archiveCount(entry) }}
                <template v-if="entry.kind === 'version'">
                  &middot; {{ formatBytes(entry.size) }}
                </template>
                <template v-else-if="entry.kind !== 'absent'">
                  &middot; {{ archiveSpan(entry) }}
                </template>
              </span>
            </span>
            <span
              v-if="entry.kind === 'version'"
              class="file-history-picks"
            >
              <button
                type="button"
                class="btn btn-sm btn-ghost"
                :aria-pressed="isPicked('from', entry.key)"
                @click="pick('from', entry.key)"
              >
                From
              </button>
              <button
                type="button"
                class="btn btn-sm btn-ghost"
                :aria-pressed="isPicked('to', entry.key)"
                @click="pick('to', entry.key)"
              >
                To
              </button>
            </span>
          </li>
          <li
            v-if="openedExtra"
            class="file-history-item"
          >
            <span class="file-history-what">
              <span class="file-history-title">{{ formatDateShort(archive.start) }}</span>
              <span class="muted">This archive, not indexed yet</span>
            </span>
            <span class="file-history-picks">
              <button
                type="button"
                class="btn btn-sm btn-ghost"
                :aria-pressed="isPicked('from', archive.name)"
                @click="pick('from', archive.name)"
              >
                From
              </button>
            </span>
          </li>
        </ul>

        <div
          v-if="notIndexed > 0"
          class="file-history-index"
        >
          <span class="muted">
            {{ notIndexed === 1 ? '1 archive is' : `${notIndexed} archives are` }} not indexed, so
            the history may miss versions.
          </span>
          <button
            type="button"
            class="btn btn-sm btn-ghost"
            :disabled="indexing"
            data-testid="file-history-index"
            @click="indexRemaining"
          >
            {{ indexing ? 'Indexing...' : 'Index them' }}
          </button>
        </div>
        <div
          v-if="indexError"
          class="error-banner"
        >
          {{ indexError }}
        </div>

        <div class="file-history-controls">
          <BaseSegmented
            v-model="layout"
            :options="LAYOUT_OPTIONS"
            label="Layout"
          />
          <span class="file-history-whole">
            <ToggleSwitch
              v-model="wholeFile"
              label="Show the whole file"
            />
            <span class="muted">Whole file</span>
          </span>
        </div>

        <div class="file-history-sides">
          <span class="file-history-side file-history-side--from">
            <span class="group-label">From</span>
            <span data-testid="file-history-from-label">{{ sideLabel(from) }}</span>
          </span>
          <span class="file-history-side file-history-side--to">
            <span class="group-label">To</span>
            <span data-testid="file-history-to-label">{{ sideLabel(to) }}</span>
          </span>
          <span
            v-if="textResult !== null"
            class="file-history-stats"
            data-testid="file-history-stats"
          >
            <span class="file-history-count--removed">-{{ textResult.removed }}</span>
            <span class="file-history-count--added">+{{ textResult.added }}</span>
            lines
          </span>
        </div>

        <div
          v-if="diffLoading"
          class="loading-row"
        >
          <BaseSpinner size="md" />
        </div>
        <div
          v-else-if="diffError"
          class="error-banner"
        >
          {{ diffError }}
        </div>
        <EmptyState
          v-else-if="from === null || to === null || from === to"
          :icon="History"
          title="Pick two versions"
          description="Choose a From and a To version above to see what changed between them."
          data-testid="file-history-pick"
        />
        <EmptyState
          v-else-if="unavailable"
          :icon="FileDiff"
          title="These versions cannot be compared"
          :description="unavailable"
          data-testid="file-history-unavailable"
        />
        <EmptyState
          v-else-if="textResult !== null && textResult.hunks.length === 0"
          :icon="FileDiff"
          title="No differences"
          :description="`${fileName} is the same in both versions.`"
        />
        <FileDiffView
          v-else-if="textResult !== null"
          :result="textResult"
          :layout="layout"
        />
      </template>
    </div>
  </BaseModal>
</template>

<style scoped>
.file-history {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  min-width: 0;
}

.file-history-path {
  margin: 0;
  overflow-wrap: anywhere;
}

.file-history-list {
  display: flex;
  flex-direction: column;
  max-height: 16rem;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.file-history-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--border-subtle);
  font-size: var(--fs-sm);
}

.file-history-item:last-child {
  border-bottom: 0;
}

.file-history-item--off .file-history-title {
  color: var(--text-secondary);
}

.file-history-what {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.file-history-title {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  font-weight: 500;
}

.file-history-lock {
  color: var(--success);
}

.file-history-picks {
  display: inline-flex;
  flex-shrink: 0;
  gap: var(--space-2);
}

.file-history-picks .btn[aria-pressed='true'] {
  background: var(--accent-subtle);
  color: var(--accent);
}

.file-history-index {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  font-size: var(--fs-sm);
}

.file-history-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-5) var(--space-6);
}

.file-history-whole {
  display: inline-flex;
  align-items: center;
  gap: var(--space-4);
}

.file-history-sides {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-8);
  font-size: var(--fs-sm);
}

.file-history-side {
  display: inline-flex;
  align-items: baseline;
  gap: var(--space-4);
}

.file-history-side--from .group-label,
.file-history-count--removed {
  color: var(--danger);
}

.file-history-side--to .group-label,
.file-history-count--added {
  color: var(--success);
}

.file-history-stats {
  display: inline-flex;
  gap: var(--space-3);
  margin-left: auto;
  color: var(--text-secondary);
}
</style>

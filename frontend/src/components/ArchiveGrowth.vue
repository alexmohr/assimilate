<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { File, FileDiff, Files, Folder, PieChart, RefreshCw } from '@lucide/vue'
import { getArchiveGrowth, recomputeArchiveGrowth } from '../api/archives'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ArchiveGrowthResponse, GrowthNode } from '../types/generated'
import { folderAt, growthFactor, shareLabel, usualGrowth } from '../utils/archiveGrowth'
import { comparableArchives } from '../utils/fileDiff'
import { extractError } from '../utils/error'
import { filesLabel, formatBytes, formatDate } from '../utils/format'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import FileDiffDialog from './FileDiffDialog.vue'

/**
 * Where an archive's deduplicated size went: the data no other archive in
 * its repository holds, per folder and per file. Worked out on the server
 * the first time it is asked for, which takes a while on a big archive, so
 * the view polls until it is ready.
 */
const props = withDefaults(
  defineProps<{
    repoId: number
    archive: ArchiveEntry
    /** The repository's other archives, to compare with this host's usual growth. */
    archives?: readonly ArchiveEntry[]
  }>(),
  { archives: () => [] },
)

/** How often a breakdown being worked out is checked on. */
const POLL_INTERVAL_MS = 2000

/**
 * Below this share of what the archive added at backup time, the breakdown
 * says the rest is shared now. borg counts a little archive metadata that
 * belongs to no file, so a breakdown is never quite the whole.
 */
const SHARED_SINCE_RATIO = 0.9

/**
 * From how many times the host's usual growth on an archive's growth is
 * called out. Below it the usual is only shown for reference: "0.4 times
 * usual" reads as a finding when it is just a quiet night.
 */
const SPIKE_FACTOR = 2

type GrowthView = 'folders' | 'files'

const VIEW_OPTIONS: readonly SegmentedOption<GrowthView>[] = [
  { value: 'folders', label: 'By folder' },
  { value: 'files', label: 'Largest files' },
]

const response = ref<ArchiveGrowthResponse | null>(null)
const error = ref<string | null>(null)
const recomputing = ref(false)
const view = ref<GrowthView>('folders')
/** The open folder, as node names from the top. */
const path = ref<string[]>([])

const growth = computed(() => response.value?.growth ?? null)
const inProgress = computed<boolean>(() => {
  const status = response.value?.status
  return status === undefined || status === 'pending' || status === 'computing'
})
const failure = computed<string | null>(() =>
  response.value?.status === 'failed' ? (response.value.error ?? 'Working it out failed') : null,
)

const folder = computed(() => folderAt(growth.value?.children ?? [], path.value))

/** The file whose diff against another archive is open, as its path in the archive. */
const comparePath = ref<string | null>(null)
const canCompare = computed(() => comparableArchives(props.archives, props.archive).length > 0)

function filePath(node: GrowthNode): string {
  return [...folder.value.trail.map((crumb) => crumb.name), node.name].join('/')
}

const usual = computed<number | null>(() => usualGrowth(props.archive, props.archives))
const factor = computed<number | null>(() =>
  growthFactor(props.archive.deduplicated_size, usual.value),
)
const spike = computed<boolean>(() => factor.value !== null && factor.value >= SPIKE_FACTOR)

/** Whether later archives now share much of what this one added. */
const sharedSince = computed<boolean>(() => {
  const total = growth.value?.total_bytes
  return total !== undefined && total < props.archive.deduplicated_size * SHARED_SINCE_RATIO
})

function nodeKey(node: GrowthNode, index: number): string {
  return `${node.kind}:${node.name}:${index}`
}

function barWidth(node: GrowthNode): string {
  const total = growth.value?.total_bytes ?? 0
  return total > 0 ? `${Math.max((node.bytes / total) * 100, 0.5)}%` : '0%'
}

function open(node: GrowthNode): void {
  if (node.kind !== 'directory') return
  path.value = [...path.value, node.name]
}

function openDepth(depth: number): void {
  path.value = path.value.slice(0, depth)
}

let pollTimer: ReturnType<typeof setTimeout> | null = null
// Archives can be switched while a request is in flight; only the latest lands.
let loadSeq = 0

function stopPolling(): void {
  if (pollTimer !== null) {
    clearTimeout(pollTimer)
    pollTimer = null
  }
}

async function load(seq: number): Promise<void> {
  try {
    const loaded = await getArchiveGrowth(props.repoId, props.archive.name)
    if (seq !== loadSeq) return
    response.value = loaded
    error.value = null
    if (inProgress.value) pollTimer = setTimeout(() => void load(seq), POLL_INTERVAL_MS)
  } catch (e: unknown) {
    if (seq === loadSeq) error.value = extractError(e)
  }
}

async function recompute(): Promise<void> {
  stopPolling()
  const seq = ++loadSeq
  recomputing.value = true
  error.value = null
  try {
    const started = await recomputeArchiveGrowth(props.repoId, props.archive.name)
    if (seq !== loadSeq) return
    response.value = started
    if (inProgress.value) pollTimer = setTimeout(() => void load(seq), POLL_INTERVAL_MS)
  } catch (e: unknown) {
    if (seq === loadSeq) error.value = extractError(e)
  } finally {
    if (seq === loadSeq) recomputing.value = false
  }
}

watch(
  () => [props.repoId, props.archive.name],
  () => {
    stopPolling()
    response.value = null
    error.value = null
    recomputing.value = false
    path.value = []
    comparePath.value = null
    void load(++loadSeq)
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  loadSeq++
  stopPolling()
})
</script>

<template>
  <div data-testid="archive-growth">
    <div
      v-if="error"
      class="error-banner growth-section"
    >
      {{ error }}
    </div>

    <div
      v-else-if="failure"
      class="growth-section growth-failed"
    >
      <div class="error-banner">{{ failure }}</div>
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        :disabled="recomputing"
        @click="recompute"
      >
        <RefreshCw :size="14" />
        Try again
      </button>
    </div>

    <div
      v-else-if="inProgress || !growth"
      class="loading-row growth-loading"
    >
      <BaseSpinner
        size="md"
        label="Working out where this archive's data is"
      />
      <span class="muted">
        Working out where this archive's data is. borg reads every file against the repository,
        which on a large archive takes a few minutes.
      </span>
    </div>

    <template v-else>
      <div class="growth-section">
        <div class="tiles">
          <div class="tile">
            <span class="stat-label">Only in this archive</span>
            <span
              class="stat-value stat-value--lg"
              data-testid="archive-growth-total"
              >{{ formatBytes(growth.total_bytes) }}</span
            >
            <span class="stat-sub">{{ filesLabel(growth.files) }}</span>
          </div>
          <div class="tile">
            <span class="stat-label">Added at backup</span>
            <span
              class="stat-value stat-value--lg"
              :class="{ 'stat-value--warning': spike }"
              >{{ formatBytes(archive.deduplicated_size) }}</span
            >
            <span
              v-if="usual === null"
              class="stat-sub"
              >No earlier archive of this host to compare with</span
            >
            <span
              v-else-if="spike"
              class="stat-sub"
              >{{ factor }} times this host's usual {{ formatBytes(usual) }}</span
            >
            <span
              v-else
              class="stat-sub"
              >This host's usual: {{ formatBytes(usual) }}</span
            >
          </div>
        </div>

        <p class="growth-note muted">
          <template v-if="sharedSince">
            Archives written since share the rest of what this one added, so only the part no other
            archive holds is broken down.
          </template>
          Worked out {{ formatDate(response?.computed_at ?? null) }}.
          <button
            type="button"
            class="btn btn-xs btn-ghost"
            :disabled="recomputing"
            @click="recompute"
          >
            <RefreshCw :size="12" />
            Recompute
          </button>
        </p>
      </div>

      <EmptyState
        v-if="growth.total_bytes === 0"
        :icon="PieChart"
        title="Nothing only this archive holds"
        description="Every chunk of this archive is shared with another archive in the repository."
      />

      <template v-else>
        <div class="growth-section">
          <BaseSegmented
            v-model="view"
            :options="VIEW_OPTIONS"
            label="How to break the data down"
          />
        </div>

        <template v-if="view === 'folders'">
          <nav
            class="path-crumbs"
            aria-label="Folder"
          >
            <button
              type="button"
              class="crumb crumb-root"
              :class="{ 'crumb-last': folder.trail.length === 0 }"
              @click="openDepth(0)"
            >
              /
            </button>
            <button
              v-for="(crumb, depth) in folder.trail"
              :key="`${depth}:${crumb.name}`"
              type="button"
              class="crumb"
              :class="{ 'crumb-last': depth === folder.trail.length - 1 }"
              @click="openDepth(depth + 1)"
            >
              {{ crumb.name }}
            </button>
          </nav>

          <div class="table-wrap">
            <table
              class="data-table"
              data-testid="archive-growth-folders"
            >
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Share of the archive</th>
                  <th class="cell-size">Only here</th>
                  <th class="cell-size">Files</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="(node, index) in folder.entries"
                  :key="nodeKey(node, index)"
                  :class="{ clickable: node.kind === 'directory' }"
                  @click="open(node)"
                >
                  <td>
                    <span class="growth-name">
                      <template v-if="node.kind === 'directory'">
                        <Folder
                          :size="14"
                          class="growth-icon"
                        />
                        <button
                          type="button"
                          class="growth-folder cell-mono"
                          @click.stop="open(node)"
                        >
                          {{ node.name }}
                        </button>
                      </template>
                      <template v-else-if="node.kind === 'file'">
                        <File
                          :size="14"
                          class="growth-icon"
                        />
                        <span class="cell-mono">{{ node.name }}</span>
                        <button
                          v-if="canCompare"
                          type="button"
                          class="btn btn-sm btn-ghost"
                          title="Compare with another archive"
                          :aria-label="`Compare ${node.name} with another archive`"
                          data-testid="growth-compare-file"
                          @click.stop="comparePath = filePath(node)"
                        >
                          <FileDiff :size="14" />
                        </button>
                      </template>
                      <template v-else>
                        <Files
                          :size="14"
                          class="growth-icon"
                        />
                        <span class="cell-muted">{{ filesLabel(node.files) }} more</span>
                      </template>
                    </span>
                  </td>
                  <td>
                    <span class="progress-row">
                      <span class="progress-track">
                        <span
                          class="progress-bar progress-bar--muted growth-bar"
                          :style="{ width: barWidth(node) }"
                        />
                      </span>
                      <span class="progress-label">{{
                        shareLabel(node.bytes, growth.total_bytes)
                      }}</span>
                    </span>
                  </td>
                  <td class="cell-size">{{ formatBytes(node.bytes) }}</td>
                  <td class="cell-size cell-muted">
                    {{ node.kind === 'file' ? '' : node.files.toLocaleString() }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </template>

        <div
          v-else
          class="table-wrap"
        >
          <table
            class="data-table"
            data-testid="archive-growth-files"
          >
            <thead>
              <tr>
                <th>Path</th>
                <th class="cell-size">Only here</th>
                <th class="cell-size">File size</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="largest in growth.largest_files"
                :key="largest.path"
              >
                <td class="cell-mono growth-path">
                  {{ largest.path }}
                  <button
                    v-if="canCompare"
                    type="button"
                    class="btn btn-sm btn-ghost"
                    title="Compare with another archive"
                    :aria-label="`Compare ${largest.path} with another archive`"
                    data-testid="growth-compare-file"
                    @click="comparePath = largest.path"
                  >
                    <FileDiff :size="14" />
                  </button>
                </td>
                <td class="cell-size">{{ formatBytes(largest.bytes) }}</td>
                <td class="cell-size cell-muted">{{ formatBytes(largest.size) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </template>

    <FileDiffDialog
      v-if="comparePath !== null"
      open
      :repo-id="repoId"
      :archive="archive"
      :archives="archives"
      :path="comparePath"
      @close="comparePath = null"
    />
  </div>
</template>

<style scoped>
.growth-section {
  padding: var(--space-5) var(--space-6) 0;
}

.growth-section:last-child {
  padding-bottom: var(--space-5);
}

.growth-failed {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-4);
}

.growth-loading {
  flex-direction: column;
  gap: var(--space-4);
  text-align: center;
}

.growth-note {
  margin: var(--space-4) 0 0;
  font-size: var(--fs-sm);
}

.growth-name {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}

.growth-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.growth-folder {
  background: none;
  border: none;
  padding: 0;
  color: var(--accent);
  cursor: pointer;
  text-align: left;
  overflow-wrap: anywhere;
}

.growth-folder:hover {
  color: var(--accent-hover);
  text-decoration: underline;
}

.growth-bar {
  display: block;
  min-width: 2px;
}

.growth-path {
  overflow-wrap: anywhere;
}
</style>

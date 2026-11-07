<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { File, Folder, Search, X } from '@lucide/vue'
import axios from 'axios'
import { searchAcrossArchives } from '../api/archives'
import { isDirectoryEntry } from '../utils/archiveSelection'
import { extractError } from '../utils/error'
import { fileNameGlob } from '../utils/fileSearchPattern'
import { formatBytes, formatDate, formatDay } from '../utils/format'
import { sameArchive, type RepoArchive } from '../utils/repoArchive'
import BaseSpinner from './BaseSpinner.vue'
import BrowserRow from './BrowserRow.vue'
import EmptyState from './EmptyState.vue'
import type { CrossSearchEntryResponse } from '../types/generated'

/**
 * Files found by name across every archive the list holds, in the card the
 * open archive otherwise takes. Opening one browses it in the newest archive
 * that has it, at its folder, with it ticked.
 *
 * borg answers one repository at a time, so a list spanning several asks each
 * of them; each answer names a path once, in the newest of its archives that
 * holds it.
 */
const props = defineProps<{
  /** What was typed into the list's search. */
  query: string
  /** The archives to look in: the list, narrowed by its repository pills. */
  archives: readonly RepoArchive[]
}>()

const emit = defineEmits<{
  close: []
  /** Browse `path` in `archive`. */
  open: [archive: RepoArchive, entry: CrossSearchEntryResponse]
}>()

/** The most archives one repository's search looks through, as the server caps it. */
const MAX_ARCHIVES = 100

interface Hit {
  entry: CrossSearchEntryResponse
  archive: RepoArchive
}

const hits = ref<Hit[]>([])
const searched = ref(0)
const loading = ref(false)
const error = ref<string | null>(null)

const repoIds = computed(() => [...new Set(props.archives.map((a) => a.repo_id))])

const repoLabel = computed(() =>
  repoIds.value.length === 1 ? (props.archives[0]?.repo_name ?? null) : null,
)

let controller: AbortController | null = null

async function run(): Promise<void> {
  controller?.abort()
  const current = new AbortController()
  controller = current
  hits.value = []
  searched.value = 0
  error.value = null
  loading.value = true
  const pattern = fileNameGlob(props.query)
  try {
    const answers = await Promise.all(
      repoIds.value.map(async (repoId) => {
        const own = props.archives.filter((a) => a.repo_id === repoId)
        const response = await searchAcrossArchives(repoId, {
          pattern,
          maxArchives: Math.min(own.length, MAX_ARCHIVES),
        })
        return { repoId, own, response }
      }),
    )
    if (controller !== current) return
    // One row per path: a list spanning several repositories opens the copy
    // in whichever of them holds the newest archive with it.
    const found = new Map<string, Hit>()
    for (const { repoId, own, response } of answers) {
      searched.value += response.total_archives_searched
      for (const entry of response.items) {
        // The server searches the repository's newest archives, which may
        // include another host's; only an archive this list holds can open.
        const archive = own.find((a) =>
          sameArchive(a, { repo_id: repoId, name: entry.archive_name }),
        )
        const seen = found.get(entry.path)
        if (archive && (!seen || Date.parse(archive.start) > Date.parse(seen.archive.start))) {
          found.set(entry.path, { entry, archive })
        }
      }
    }
    hits.value = [...found.values()].sort((a, b) => a.entry.path.localeCompare(b.entry.path))
  } catch (e: unknown) {
    if (controller === current && !axios.isCancel(e)) error.value = extractError(e)
  } finally {
    if (controller === current) loading.value = false
  }
}

watch(
  () => [props.query, repoIds.value.join(',')],
  () => void run(),
  { immediate: true },
)

function parentLabel(path: string): string {
  const slash = path.lastIndexOf('/')
  return slash <= 0 ? '/' : `/${path.slice(0, slash)}`
}

function nameOf(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}
</script>

<template>
  <div
    class="panel panel--sectioned file-search"
    data-testid="file-search"
  >
    <div class="file-search-head">
      <div class="file-search-heading">
        <h2 class="file-search-title">Files matching &ldquo;{{ query }}&rdquo;</h2>
        <p class="file-search-facts">
          <template v-if="loading">Searching {{ archives.length }} archives...</template>
          <template v-else>
            {{ hits.length }} {{ hits.length === 1 ? 'path' : 'paths' }} &middot; {{ searched }}
            {{ searched === 1 ? 'archive' : 'archives' }} searched
            <template v-if="repoLabel"> &middot; {{ repoLabel }}</template>
          </template>
        </p>
      </div>
      <button
        class="btn btn-sm btn-ghost"
        type="button"
        aria-label="Close search"
        title="Close search"
        @click="emit('close')"
      >
        <X :size="14" />
      </button>
    </div>

    <div
      v-if="loading"
      class="loading-row"
    >
      <BaseSpinner
        size="md"
        label="Searching the archives"
      />
    </div>
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="hits.length === 0"
      :icon="Search"
      :title="`No file names contain “${query}”`"
      description="Search matches part of a name, e.g. nginx or .pdf, or a glob such as *.sql."
    />
    <template v-else>
      <div class="file-search-rows">
        <BrowserRow
          v-for="hit in hits"
          :key="hit.entry.path"
          as="button"
          :title="nameOf(hit.entry.path)"
          :size="isDirectoryEntry(hit.entry) ? null : formatBytes(hit.entry.size)"
          data-testid="file-search-hit"
          @click="emit('open', hit.archive, hit.entry)"
        >
          <template #lead>
            <Folder
              v-if="isDirectoryEntry(hit.entry)"
              :size="16"
              class="file-search-icon file-search-icon--dir"
            />
            <File
              v-else
              :size="16"
              class="file-search-icon"
            />
          </template>
          <template #facts>
            <span class="mono">{{ parentLabel(hit.entry.path) }}</span>
            <span aria-hidden="true">&middot;</span>
            <span :title="formatDate(hit.archive.start)"
              >newest in {{ hit.archive.name }}, {{ formatDay(hit.archive.start) }}</span
            >
            <template v-if="!repoLabel">
              <span aria-hidden="true">&middot;</span>
              <span>{{ hit.archive.repo_name }}</span>
            </template>
          </template>
        </BrowserRow>
      </div>
      <p class="file-search-note">Open a result to browse it in the newest archive that has it.</p>
    </template>
  </div>
</template>

<style scoped>
/* Nothing drops out of this card, but its corners still round the rows. */
.file-search {
  min-height: 18rem;
}

.file-search-head {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.file-search-heading {
  flex: 1;
  min-width: 0;
}

.file-search-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  overflow-wrap: anywhere;
}

.file-search-facts {
  margin: var(--space-1) 0 0;
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

.file-search-rows {
  max-height: min(40rem, 75vh);
  overflow-y: auto;
}

@media (max-width: 768px) {
  .file-search-rows {
    max-height: none;
  }
}

.file-search-icon {
  flex: none;
  color: var(--text-muted);
}

.file-search-icon--dir {
  color: var(--accent);
}

.file-search-note {
  margin: 0;
  padding: var(--space-4) var(--space-7);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>

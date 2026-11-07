<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { GitCompare } from '@lucide/vue'
import { diffArchives } from '../api/archives'
import { previousArchive } from '../utils/archiveChanges'
import { comparableArchives } from '../utils/fileDiff'
import { extractError } from '../utils/error'
import { formatDate, formatDay } from '../utils/format'
import BaseSpinner from './BaseSpinner.vue'
import BrowserRow from './BrowserRow.vue'
import BrowserRowGroup from './BrowserRowGroup.vue'
import EmptyState from './EmptyState.vue'
import type { ArchiveEntryResponse, DiffResponse } from '../types/generated'

/**
 * The Changes tab of an open archive: which paths it added, changed and
 * removed against another archive of the same machine - the previous one
 * unless the reader picks a week earlier or any other.
 */
const props = defineProps<{
  repoId: number
  archive: ArchiveEntryResponse
  /** The open archive's repository's archives, to compare with. */
  archives: readonly ArchiveEntryResponse[]
}>()

/** Which archive the open one is compared with. */
type Against = 'previous' | 'week' | 'other'

type ChangeKind = 'added' | 'changed' | 'removed'

interface Change {
  path: string
  kind: ChangeKind
}

const DOT: Record<ChangeKind, string> = {
  added: 'status-dot--success',
  changed: 'status-dot--warning',
  removed: 'status-dot--danger',
}

const WEEK_MS = 7 * 86_400_000

const against = ref<Against>('previous')
const otherName = ref<string | null>(null)
const result = ref<DiffResponse | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)

const candidates = computed(() => comparableArchives(props.archives, props.archive))

const previous = computed(() => previousArchive(props.archives, props.archive))

const weekEarlier = computed<ArchiveEntryResponse | null>(() => {
  const before = Date.parse(props.archive.start) - WEEK_MS
  return candidates.value.find((a) => Date.parse(a.start) <= before) ?? null
})

const base = computed<ArchiveEntryResponse | null>(() => {
  switch (against.value) {
    case 'previous':
      return previous.value
    case 'week':
      return weekEarlier.value
    case 'other':
      return candidates.value.find((a) => a.name === otherName.value) ?? null
  }
  return null
})

const changes = computed<Change[]>(() => {
  const r = result.value
  if (r === null) return []
  return [
    ...r.added.map((path): Change => ({ path, kind: 'added' })),
    ...r.modified.map((path): Change => ({ path, kind: 'changed' })),
    ...r.removed.map((path): Change => ({ path, kind: 'removed' })),
  ]
})

const summary = computed(() => {
  const r = result.value
  if (r === null) return ''
  const shown = `${r.added.length} added · ${r.modified.length} changed · ${r.removed.length} removed`
  return r.total_changes > changes.value.length
    ? `${shown} (the first ${changes.value.length} of ${r.total_changes})`
    : shown
})

// Archives can be switched while a comparison runs; only the latest lands.
let seq = 0

async function load(): Promise<void> {
  const mine = ++seq
  result.value = null
  error.value = null
  const other = base.value
  if (other === null) {
    loading.value = false
    return
  }
  loading.value = true
  try {
    const diff = await diffArchives(props.repoId, {
      archive1: other.name,
      archive2: props.archive.name,
    })
    if (mine === seq) result.value = diff
  } catch (e: unknown) {
    if (mine === seq) error.value = extractError(e)
  } finally {
    if (mine === seq) loading.value = false
  }
}

watch(
  () => `${props.repoId}/${props.archive.name}`,
  () => {
    against.value = 'previous'
    otherName.value = null
  },
)

watch(
  () => `${props.repoId}/${props.archive.name}/${base.value?.name ?? ''}`,
  () => void load(),
  { immediate: true },
)

function pickOther(): void {
  against.value = 'other'
  if (otherName.value === null) otherName.value = candidates.value[0]?.name ?? null
}
</script>

<template>
  <div data-testid="archive-diff">
    <div
      class="archive-diff-controls"
      role="group"
      aria-label="Compare with"
    >
      <span class="archive-diff-key">Compare with</span>
      <span class="filter-pills">
        <button
          type="button"
          class="filter-pill"
          :class="{ active: against === 'previous' }"
          :aria-pressed="against === 'previous'"
          @click="against = 'previous'"
        >
          Previous{{ previous ? ` · ${formatDay(previous.start)}` : '' }}
        </button>
        <button
          type="button"
          class="filter-pill"
          :class="{ active: against === 'week' }"
          :aria-pressed="against === 'week'"
          :disabled="weekEarlier === null"
          @click="against = 'week'"
        >
          A week earlier
        </button>
        <button
          type="button"
          class="filter-pill"
          :class="{ active: against === 'other' }"
          :aria-pressed="against === 'other'"
          :disabled="candidates.length === 0"
          @click="pickOther"
        >
          Other archive
        </button>
      </span>
      <select
        v-if="against === 'other'"
        v-model="otherName"
        class="input select-input select-input--sm archive-diff-select"
        aria-label="Archive to compare with"
      >
        <option
          v-for="option in candidates"
          :key="option.name"
          :value="option.name"
        >
          {{ option.name }}
        </option>
      </select>
    </div>

    <EmptyState
      v-if="base === null"
      :icon="GitCompare"
      :title="
        against === 'previous' ? 'The oldest archive of this machine' : 'Nothing to compare with'
      "
      :description="
        against === 'previous'
          ? 'There is no earlier archive to compare it with. Pick another archive instead.'
          : 'No archive of this machine matches. Pick another one.'
      "
    />
    <div
      v-else-if="loading"
      class="loading-row"
    >
      <BaseSpinner
        size="md"
        label="Comparing the archives"
      />
    </div>
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <template v-else-if="result">
      <p
        v-if="changes.length === 0"
        class="state-msg state-msg--inline archive-diff-msg"
      >
        Nothing changed since {{ base.name }}.
      </p>
      <template v-else>
        <BrowserRowGroup :label="summary">
          <template #aside>
            <span :title="formatDate(base.start)">against {{ base.name }}</span>
          </template>
        </BrowserRowGroup>
        <div class="archive-diff-rows">
          <BrowserRow
            v-for="change in changes"
            :key="`${change.kind}:${change.path}`"
            :title="change.path"
            data-testid="diff-row"
          >
            <template #lead>
              <span
                class="status-dot"
                :class="DOT[change.kind]"
              />
            </template>
            <template #facts>{{ change.kind }}</template>
          </BrowserRow>
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.archive-diff-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-5);
  padding: var(--space-5) var(--space-7);
  border-bottom: 1px solid var(--border);
}

.archive-diff-key {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.archive-diff-select {
  max-width: 100%;
}

.archive-diff-msg {
  padding: var(--space-5) var(--space-7);
}

.archive-diff-rows {
  max-height: 36rem;
  overflow-y: auto;
}

@media (max-width: 768px) {
  .archive-diff-rows {
    max-height: none;
  }
}
</style>

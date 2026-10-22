<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FileDiff } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import FileDiffView from './FileDiffView.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import { diffArchiveFile } from '../api/archives'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import {
  comparableArchives,
  defaultComparison,
  unavailableReason,
  type DiffLayout,
} from '../utils/fileDiff'
import type { ArchiveEntryResponse, FileDiffResponse } from '../types/generated'

/**
 * One file of an archive compared, line by line, with the same path in
 * another archive of the repository. The other archive is the base (its
 * lines are the removed ones), this one the target, so by default - against
 * the archive just before it - the diff reads as what changed in this one.
 */
const props = defineProps<{
  open: boolean
  repoId: number
  /** The archive the file was opened from: the target side. */
  archive: ArchiveEntryResponse
  /** Every archive the browser lists, to pick the base from. */
  archives: readonly ArchiveEntryResponse[]
  /** The file's path within the archive. */
  path: string
}>()

const emit = defineEmits<{ close: [] }>()

const LAYOUT_OPTIONS: readonly SegmentedOption<DiffLayout>[] = [
  { value: 'split', label: 'Side by side' },
  { value: 'inline', label: 'Inline' },
]

/** Side by side needs the width; a phone opens on the inline view. */
const WIDE_ENOUGH_FOR_SPLIT = '(min-width: 768px)'

const againstName = ref('')
const layout = ref<DiffLayout>('inline')
const wholeFile = ref(false)
const loading = ref(false)
const error = ref<string | null>(null)
const result = ref<FileDiffResponse | null>(null)

const choices = computed(() => comparableArchives(props.archives, props.archive))
const against = computed(
  () => choices.value.find((archive) => archive.name === againstName.value) ?? null,
)
const fileName = computed(() => props.path.split('/').pop() ?? props.path)

function archiveLabel(archive: ArchiveEntryResponse): string {
  return formatDate(archive.start)
}

const unavailable = computed<string | null>(() => {
  const response = result.value
  if (response === null || response.status === 'text') return null
  return unavailableReason(response, {
    base: against.value === null ? 'the other archive' : archiveLabel(against.value),
    target: 'this archive',
  })
})

const textResult = computed(() =>
  result.value !== null && result.value.status === 'text' ? result.value : null,
)

// Answers to an earlier request (another archive picked meanwhile) are dropped.
let requestSeq = 0

async function load(): Promise<void> {
  if (!props.open || againstName.value === '') return
  const seq = ++requestSeq
  loading.value = true
  error.value = null
  try {
    const response = await diffArchiveFile(props.repoId, props.archive.name, {
      path: props.path,
      against: againstName.value,
      wholeFile: wholeFile.value,
    })
    if (seq !== requestSeq) return
    result.value = response
  } catch (e: unknown) {
    if (seq !== requestSeq) return
    result.value = null
    error.value = extractError(e)
  } finally {
    if (seq === requestSeq) loading.value = false
  }
}

watch(
  () => [props.open, props.path, props.archive.name] as const,
  ([open]) => {
    if (!open) return
    againstName.value = defaultComparison(props.archives, props.archive)?.name ?? ''
    wholeFile.value = false
    layout.value = window.matchMedia(WIDE_ENOUGH_FOR_SPLIT).matches ? 'split' : 'inline'
    result.value = null
    void load()
  },
  { immediate: true },
)

watch([againstName, wholeFile], () => {
  void load()
})
</script>

<template>
  <BaseModal
    :open="open"
    :title="`Compare ${fileName}`"
    size="lg"
    @close="emit('close')"
  >
    <div class="file-diff">
      <p class="mono muted file-diff-path">{{ path }}</p>

      <div class="file-diff-controls">
        <label class="field file-diff-against">
          <span class="field-label">Compare with</span>
          <select
            v-model="againstName"
            class="input select-input"
            data-testid="file-diff-against"
          >
            <option
              v-for="choice in choices"
              :key="choice.name"
              :value="choice.name"
              :title="choice.name"
            >
              {{ archiveLabel(choice) }}
            </option>
          </select>
        </label>
        <BaseSegmented
          v-model="layout"
          :options="LAYOUT_OPTIONS"
          label="Layout"
        />
        <span class="file-diff-whole">
          <ToggleSwitch
            v-model="wholeFile"
            label="Show the whole file"
          />
          <span class="muted">Whole file</span>
        </span>
      </div>

      <div class="file-diff-sides">
        <span class="file-diff-side file-diff-side--base">
          <span class="group-label">Removed from</span>
          <span>{{ against === null ? '' : archiveLabel(against) }}</span>
        </span>
        <span class="file-diff-side file-diff-side--target">
          <span class="group-label">Added in</span>
          <span>{{ archiveLabel(archive) }} (this archive)</span>
        </span>
        <span
          v-if="textResult !== null"
          class="file-diff-stats"
          data-testid="file-diff-stats"
        >
          <span class="file-diff-count--removed">-{{ textResult.removed }}</span>
          <span class="file-diff-count--added">+{{ textResult.added }}</span>
          lines
        </span>
      </div>

      <div
        v-if="loading"
        class="loading-row"
      >
        <BaseSpinner size="md" />
      </div>
      <div
        v-else-if="error"
        class="error-banner"
      >
        {{ error }}
      </div>
      <EmptyState
        v-else-if="unavailable"
        :icon="FileDiff"
        title="These versions cannot be compared"
        :description="unavailable"
        data-testid="file-diff-unavailable"
      />
      <EmptyState
        v-else-if="textResult !== null && textResult.hunks.length === 0"
        :icon="FileDiff"
        title="No differences"
        :description="`${fileName} is the same in both archives.`"
      />
      <FileDiffView
        v-else-if="textResult !== null"
        :result="textResult"
        :layout="layout"
      />
    </div>
  </BaseModal>
</template>

<style scoped>
.file-diff {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  min-width: 0;
}

.file-diff-path {
  margin: 0;
  overflow-wrap: anywhere;
}

.file-diff-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: var(--space-5) var(--space-6);
}

.file-diff-against {
  margin: 0;
}

.file-diff-whole {
  display: inline-flex;
  align-items: center;
  gap: var(--space-4);
}

.file-diff-sides {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4) var(--space-8);
  font-size: var(--fs-sm);
}

.file-diff-side {
  display: inline-flex;
  align-items: baseline;
  gap: var(--space-4);
}

.file-diff-side--base .group-label,
.file-diff-count--removed {
  color: var(--danger);
}

.file-diff-side--target .group-label,
.file-diff-count--added {
  color: var(--success);
}

.file-diff-stats {
  display: inline-flex;
  gap: var(--space-3);
  margin-left: auto;
  color: var(--text-secondary);
}
</style>

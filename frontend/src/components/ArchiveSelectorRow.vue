<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BrowserRow from './BrowserRow.vue'
import { formatBytes, formatClock, formatDate, formatDay } from '../utils/format'
import { resolveArchiveHost } from '../utils/archiveHost'
import type { ArchiveEntryResponse as ArchiveEntry } from '../types/generated'

/**
 * One archive in the list: its name over the facts that tell two archives
 * apart, its size and time on the right.
 *
 * The full date is not repeated on the row - the day heading above it already
 * says it - and nothing on the row acts on the archive: deleting, downloading
 * and comparing live in the open archive's menu, so a stray click on a long
 * list can only ever open an archive.
 */
const props = withDefaults(
  defineProps<{
    archive: ArchiveEntry
    selected: boolean
    /** A borg delete is already queued for this archive. */
    deleting?: boolean
    /** Waiting for its append-only repository's next maintenance run. */
    queued?: boolean
    /**
     * Whether to name the host: a list of one machine's archives - an agent's
     * Backups tab - would repeat the same name on every row.
     */
    showHost?: boolean
    /** The repository to name on the row, when the list spans several and none is picked. */
    repoName?: string | null
    /**
     * The time of day rather than the date, for a row under a heading that
     * already names its day ("Today", "This week").
     */
    clock?: boolean
  }>(),
  { deleting: false, queued: false, showHost: false, repoName: null, clock: false },
)

defineEmits<{ select: [] }>()

const isUnmatched = computed(() => props.archive.matched !== true)

const hostLabel = computed(() => resolveArchiveHost(props.archive))

const time = computed(() =>
  props.clock ? formatClock(props.archive.start) : formatDay(props.archive.start),
)
</script>

<template>
  <BrowserRow
    as="button"
    :title="archive.name"
    :size="formatBytes(archive.original_size)"
    :time="time"
    :time-title="formatDate(archive.start)"
    :selected="selected"
    :dim="deleting || queued"
    :aria-current="selected"
    :aria-label="`Open archive ${archive.name}`"
    data-testid="archive-row"
    @click="$emit('select')"
  >
    <template #facts>
      <span
        v-if="showHost"
        :class="{ 'archive-row-unmatched': isUnmatched }"
        >{{ hostLabel }}</span
      >
      <span
        v-if="showHost"
        aria-hidden="true"
        >&middot;</span
      >
      <span v-if="repoName">{{ repoName }}</span>
      <span
        v-if="repoName"
        aria-hidden="true"
        >&middot;</span
      >
      <span>+{{ formatBytes(archive.deduplicated_size) }} new</span>
      <span
        v-if="deleting"
        class="archive-row-deleting"
        >Deleting</span
      >
      <span
        v-if="queued"
        class="archive-row-queued"
        >Removed at next maintenance</span
      >
    </template>
  </BrowserRow>
</template>

<style scoped>
/* borg recorded a hostname no agent claims. */
.archive-row-unmatched {
  color: var(--warning);
}

.archive-row-deleting {
  color: var(--danger);
}

.archive-row-queued {
  color: var(--violet);
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { Trash2 } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import { formatBytes, formatDate } from '../utils/format'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'

/**
 * One archive in the selector list.
 *
 * Two lines rather than a five-column grid: the name and the size are what a
 * row is picked by, and host/date/dedup follow underneath. The same row then
 * fits the 360px pane the schedule view gives it and the full-width one the
 * repository view does, so both screens can share it.
 *
 * The delete control is always rendered for a user who may delete. It used to
 * be revealed by `opacity` on row hover, which made it invisible until the
 * pointer happened to land on the right row and unreachable by keyboard.
 */
const props = defineProps<{
  archive: ArchiveEntry
  selected: boolean
  /** Flat-list rows carry no group header above them, so they are not inset. */
  flat: boolean
  canDelete: boolean
  /** A borg delete is already queued for this archive. */
  deleting: boolean
}>()

const emit = defineEmits<{ select: []; delete: [] }>()

const isUnmatched = computed(() => props.archive.matched !== true)

/** The agent's hostname where borg's own metadata could be attributed to one. */
const hostLabel = computed(() => props.archive.agent_hostname ?? props.archive.hostname)
</script>

<template>
  <div
    class="archive-row"
    :class="{
      selected,
      'archive-row-detailed': flat,
      'archive-row--unmatched': isUnmatched,
      'archive-row--deleting': deleting,
    }"
  >
    <button
      class="archive-row-select"
      type="button"
      :aria-current="selected"
      @click="emit('select')"
    >
      <span class="archive-line">
        <span class="archive-name">{{ archive.name }}</span>
        <span class="archive-size">{{ formatBytes(archive.original_size) }}</span>
      </span>
      <span class="archive-line archive-row-sub">
        <span class="archive-host">{{ hostLabel }}</span>
        <span
          class="archive-sep"
          aria-hidden="true"
          >/</span
        >
        <span class="archive-date">{{ formatDate(archive.start) }}</span>
        <span
          class="archive-sep"
          aria-hidden="true"
          >/</span
        >
        <span class="archive-dedup">{{ formatBytes(archive.deduplicated_size) }} dedup</span>
      </span>
    </button>

    <span
      v-if="canDelete"
      class="archive-row-actions"
    >
      <span
        v-if="deleting"
        class="archive-row-pending"
        >Deleting</span
      >
      <button
        class="btn btn-xs btn-ghost archive-row-delete"
        type="button"
        :disabled="deleting"
        :title="deleting ? 'Deletion in progress' : 'Delete archive'"
        :aria-label="deleting ? 'Deletion in progress' : `Delete archive ${archive.name}`"
        @click="emit('delete')"
      >
        <BaseSpinner
          v-if="deleting"
          size="sm"
        />
        <Trash2
          v-else
          :size="12"
        />
      </button>
    </span>
  </div>
</template>

<style scoped>
.archive-row {
  display: flex;
  align-items: stretch;
  gap: var(--space-2);
  padding-right: var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  border-left: 3px solid transparent;
  transition: background var(--duration-fast);
}

.archive-row:last-child {
  border-bottom: none;
}

.archive-row:hover {
  background: var(--bg-hover);
}

.archive-row.selected {
  background: var(--accent-subtle);
  border-left-color: var(--accent);
}

/* borg recorded a hostname no agent claims, so nothing on this row links
   anywhere - the stripe is the only place left to say so in the flat list,
   where there is no group header carrying the warning. */
.archive-row--unmatched {
  border-left-color: var(--warning);
}

.archive-row--deleting .archive-row-select {
  opacity: 0.55;
}

.archive-row-select {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-5) var(--space-4) var(--space-5) var(--space-8);
  background: none;
  border: none;
  cursor: pointer;
  font: inherit;
  color: inherit;
  text-align: left;
}

/* A flat row has no group header above it to indent under. */
.archive-row-detailed .archive-row-select {
  padding-left: var(--space-6);
}

.archive-line {
  display: flex;
  align-items: baseline;
  gap: var(--space-4);
  min-width: 0;
}

.archive-name {
  flex: 1;
  min-width: 0;
  font-family: var(--mono);
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-size {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.archive-row-sub {
  gap: var(--space-3);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

.archive-host {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-row--unmatched .archive-host {
  color: var(--warning);
}

.archive-date,
.archive-dedup {
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.archive-sep {
  color: var(--border);
}

.archive-row-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex: none;
}

.archive-row-pending {
  font-size: var(--fs-2xs);
  color: var(--danger);
  white-space: nowrap;
}

.archive-row-delete:hover:not(:disabled) {
  color: var(--danger);
  border-color: var(--danger);
}
</style>

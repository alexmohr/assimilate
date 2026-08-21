<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { Trash2 } from '@lucide/vue'
import BaseHostLink from './BaseHostLink.vue'
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
 *
 * The row itself handles the pointer and carries a stretched button that never
 * takes pointer events. That is what lets the host name stay a link to the
 * agent: an anchor cannot be nested inside a button, but a button laid over the
 * row would sit between every click and the text, link or delete control
 * beneath it. Here the button only makes the row focusable and gives it an
 * accessible name; its own Enter and Space still bubble up as a click.
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
    @click="emit('select')"
  >
    <button
      class="archive-row-select"
      type="button"
      :aria-current="selected"
      :aria-label="`Select archive ${archive.name}`"
    ></button>

    <span class="archive-row-body">
      <span class="archive-line">
        <span class="archive-name">{{ archive.name }}</span>
        <span class="archive-size">{{ formatBytes(archive.original_size) }}</span>
      </span>
      <span class="archive-line archive-row-sub">
        <BaseHostLink
          :hostname="hostLabel"
          class="archive-host"
          @click.stop
        />
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
    </span>

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
        @click.stop="emit('delete')"
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
  position: relative;
  display: flex;
  cursor: pointer;
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

/* borg recorded a hostname no agent claims. The stripe is the only place left
   to say so in the flat list, where there is no group header carrying the
   warning. */
.archive-row--unmatched {
  border-left-color: var(--warning);
}

.archive-row--deleting .archive-row-body {
  opacity: 0.55;
}

/* Focus and the accessible name only - the row handles the pointer. Taking
   pointer events here would put this button in front of the archive name, the
   host link and the delete control, which is both wrong for a real click and
   an interception error for anything driving the page. */
.archive-row-select {
  position: absolute;
  inset: 0;
  padding: 0;
  background: none;
  border: none;
  pointer-events: none;
}

.archive-row-select:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

.archive-row-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-5) var(--space-4) var(--space-5) var(--space-8);
  text-align: left;
}

/* A flat row has no group header above it to indent under. */
.archive-row-detailed .archive-row-body {
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
  color: inherit;
}

.archive-host:hover {
  color: var(--accent);
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

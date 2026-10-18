<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, nextTick, ref, useTemplateRef, watch } from 'vue'
import { ChevronDown, ChevronRight, File, Folder } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import { formatBytes, formatDate } from '../utils/format'
import { matchParts } from '../utils/textMatch'
import { tickState, type TickState } from '../utils/restoreSelection'
import type { ChangeKind, EntryRow, TreeRow } from '../composables/useRestoreTree'
import type { ContentEntryResponse as ContentEntry, PathTotals } from '../types/generated'

const props = withDefaults(
  defineProps<{
    rows: TreeRow[]
    selection: string[]
    totals: Record<string, PathTotals>
    changeOf: (entry: ContentEntry) => ChangeKind | null
    /** Search text to highlight in names. */
    highlight?: string
  }>(),
  { highlight: '' },
)

const emit = defineEmits<{
  tick: [path: string]
  expand: [path: string]
  more: [dir: string]
}>()

const treeEl = useTemplateRef<HTMLElement>('tree')
const focusedKey = ref<string | null>(null)

// The focused row is the one Tab lands on; it falls back to the first row
// when the rows change under it (a search, another archive).
const activeKey = computed(() => {
  const keys = props.rows.map((row) => row.key)
  return focusedKey.value !== null && keys.includes(focusedKey.value)
    ? focusedKey.value
    : (keys[0] ?? null)
})

watch(
  () => props.rows.length,
  () => {
    if (focusedKey.value !== null && !props.rows.some((row) => row.key === focusedKey.value)) {
      focusedKey.value = null
    }
  },
)

function stateOf(row: EntryRow): TickState {
  return tickState(props.selection, row.path)
}

function ariaChecked(row: EntryRow): 'true' | 'false' | 'mixed' {
  const state = stateOf(row)
  if (state === 'checked') return 'true'
  if (state === 'partial') return 'mixed'
  return 'false'
}

function changeLabel(kind: ChangeKind): string {
  return kind === 'new' ? 'New' : 'Changed'
}

function folderSummary(path: string): string | null {
  const total = props.totals[path]
  if (total === undefined) return null
  return `${total.files} ${total.files === 1 ? 'file' : 'files'}, ${formatBytes(total.bytes)}`
}

async function focusRow(key: string): Promise<void> {
  focusedKey.value = key
  await nextTick()
  const el = treeEl.value?.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(key)}"]`)
  el?.focus()
}

function activate(row: TreeRow): void {
  if (row.kind === 'more') emit('more', row.dir)
  else emit('tick', row.path)
}

/** What a key does on the focused row; false when the tree does not use the key. */
function handleKey(event: KeyboardEvent, row: TreeRow, index: number): boolean {
  const rows = props.rows
  const move = (to: number): void => {
    const target = rows[Math.max(0, Math.min(rows.length - 1, to))]
    if (target !== undefined) void focusRow(target.key)
  }
  if (event.key === 'ArrowDown') move(index + 1)
  else if (event.key === 'ArrowUp') move(index - 1)
  else if (event.key === 'Home') move(0)
  else if (event.key === 'End') move(rows.length - 1)
  else if (event.key === ' ') activate(row)
  else if (event.key === 'Enter') {
    if (row.kind === 'entry' && row.expandable) emit('expand', row.path)
    else activate(row)
  } else if (event.key === 'ArrowRight') {
    if (row.kind !== 'entry' || !row.expandable) return false
    if (row.expanded) move(index + 1)
    else emit('expand', row.path)
  } else if (event.key === 'ArrowLeft') {
    if (row.kind === 'entry' && row.expanded) {
      emit('expand', row.path)
      return true
    }
    const parent = rows
      .slice(0, index)
      .reverse()
      .find((candidate) => candidate.depth < row.depth)
    if (parent === undefined) return false
    void focusRow(parent.key)
  } else return false
  return true
}

function onKeydown(event: KeyboardEvent): void {
  const index = props.rows.findIndex((row) => row.key === activeKey.value)
  const row = props.rows[index]
  if (row !== undefined && handleKey(event, row, index)) event.preventDefault()
}

function indent(depth: number): Record<string, string> {
  return { paddingLeft: `calc(var(--space-7) * ${depth} + var(--space-3))` }
}
</script>

<template>
  <div
    ref="tree"
    class="restore-tree"
    role="tree"
    aria-label="Archive contents"
    aria-multiselectable="true"
    data-testid="restore-tree"
    @keydown="onKeydown"
  >
    <template
      v-for="row in rows"
      :key="row.key"
    >
      <div
        v-if="row.kind === 'entry'"
        class="tree-row"
        :class="{ 'tree-row--match': row.match }"
        role="treeitem"
        :aria-level="row.depth + 1"
        :aria-expanded="row.expandable ? row.expanded : undefined"
        :aria-checked="ariaChecked(row)"
        :aria-label="row.name"
        :data-path="row.path"
        :data-row-key="row.key"
        :tabindex="row.key === activeKey ? 0 : -1"
        :style="indent(row.depth)"
        @focus="focusedKey = row.key"
      >
        <button
          v-if="row.expandable"
          type="button"
          class="btn btn-xs btn-ghost tree-chevron"
          tabindex="-1"
          :aria-label="row.expanded ? `Collapse ${row.name}` : `Expand ${row.name}`"
          @click="emit('expand', row.path)"
        >
          <BaseSpinner
            v-if="row.loading"
            size="sm"
          />
          <ChevronDown
            v-else-if="row.expanded"
            :size="14"
          />
          <ChevronRight
            v-else
            :size="14"
          />
        </button>
        <span
          v-else
          class="tree-chevron-spacer"
        />
        <input
          type="checkbox"
          class="tree-check"
          tabindex="-1"
          :checked="stateOf(row) === 'checked'"
          :indeterminate="stateOf(row) === 'partial'"
          :aria-label="`Select ${row.name}`"
          @click.prevent="emit('tick', row.path)"
        />
        <Folder
          v-if="row.isDir"
          :size="14"
          class="tree-icon tree-icon--dir"
        />
        <File
          v-else
          :size="14"
          class="tree-icon"
        />
        <button
          type="button"
          class="tree-name"
          tabindex="-1"
          :title="row.path"
          @click="row.expandable ? emit('expand', row.path) : emit('tick', row.path)"
        >
          <template
            v-for="(part, i) in matchParts(row.name, highlight)"
            :key="i"
          >
            <mark
              v-if="part.match"
              class="tree-highlight"
              >{{ part.text }}</mark
            ><template v-else>{{ part.text }}</template>
          </template>
        </button>
        <span
          v-if="!row.isDir && changeOf(row.entry) !== null"
          class="badge badge--info tree-change"
          >{{ changeLabel(changeOf(row.entry) ?? 'changed') }}</span
        >
        <span
          v-if="row.isDir"
          class="tree-meta"
          >{{ folderSummary(row.path) ?? '' }}</span
        >
        <template v-else>
          <span class="tree-meta tree-mtime">{{ formatDate(row.entry.mtime, '') }}</span>
          <span class="tree-meta tree-size">{{ formatBytes(row.entry.size) }}</span>
        </template>
      </div>
      <div
        v-else
        class="tree-row tree-row--more"
        role="treeitem"
        :aria-level="row.depth + 1"
        aria-label="Load more"
        :data-row-key="row.key"
        :tabindex="row.key === activeKey ? 0 : -1"
        :style="indent(row.depth)"
        @focus="focusedKey = row.key"
      >
        <button
          type="button"
          class="btn btn-xs btn-ghost"
          tabindex="-1"
          :disabled="row.loading"
          @click="emit('more', row.dir)"
        >
          {{ row.loading ? 'Loading...' : 'Load more' }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.restore-tree {
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  max-height: 60vh;
  font-size: var(--fs-sm);
}

.tree-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 2rem;
  padding-right: var(--space-4);
  border-radius: var(--radius-sm);
  cursor: default;
}

.tree-row:hover,
.tree-row:focus-visible {
  background: var(--bg-hover);
}

.tree-row--match .tree-name {
  font-weight: 600;
}

.tree-chevron {
  padding: 0;
  width: 1.5rem;
  justify-content: center;
}

.tree-chevron-spacer {
  display: inline-block;
  width: 1.5rem;
  flex-shrink: 0;
}

.tree-check {
  flex-shrink: 0;
  margin: 0;
}

.tree-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.tree-icon--dir {
  color: var(--accent);
}

.tree-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
  background: none;
  border: none;
  padding: 0;
  color: var(--text-primary);
  font: inherit;
  cursor: pointer;
}

.tree-highlight {
  background: var(--accent-subtle);
  color: inherit;
  border-radius: var(--radius-sm);
}

.tree-change {
  flex-shrink: 0;
}

.tree-meta {
  flex-shrink: 0;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.tree-size {
  min-width: 5rem;
  text-align: right;
}

@media (max-width: 640px) {
  .tree-mtime {
    display: none;
  }
}
</style>

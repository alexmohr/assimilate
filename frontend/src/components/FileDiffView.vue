<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { toSplitRows, type DiffLayout, type SplitRow } from '../utils/fileDiff'
import type { FileDiffHunk, FileDiffLine, FileDiffResponse } from '../types/generated'

/**
 * The changed stretches of a file comparison, inline or side by side: the
 * base's lines removed in red, the target's added in green, with the part of
 * a changed line that differs highlighted.
 */
const props = defineProps<{
  result: Extract<FileDiffResponse, { status: 'text' }>
  layout: DiffLayout
}>()

interface ShownHunk {
  hunk: FileDiffHunk
  rows: SplitRow[]
}

const shownHunks = computed<ShownHunk[]>(() =>
  props.result.hunks.map((hunk) => ({ hunk, rows: toSplitRows(hunk.lines) })),
)

function hunkLabel(hunk: FileDiffHunk): string {
  const last = hunk.target_start + Math.max(hunk.target_count, 1) - 1
  return hunk.target_count <= 1 ? `Line ${hunk.target_start}` : `Lines ${hunk.target_start}-${last}`
}

function sign(line: FileDiffLine): string {
  if (line.kind === 'added') return '+'
  if (line.kind === 'removed') return '-'
  return ''
}
</script>

<template>
  <div
    class="file-diff-body"
    :class="`file-diff-body--${layout}`"
    data-testid="file-diff-body"
  >
    <template
      v-for="(shown, h) in shownHunks"
      :key="h"
    >
      <div class="file-diff-hunk">{{ hunkLabel(shown.hunk) }}</div>
      <template v-if="layout === 'inline'">
        <div
          v-for="(line, i) in shown.hunk.lines"
          :key="i"
          class="file-diff-line"
          :class="`file-diff-line--${line.kind}`"
        >
          <span class="file-diff-num">{{ line.base_line ?? '' }}</span>
          <span class="file-diff-num">{{ line.target_line ?? '' }}</span>
          <span class="file-diff-sign">{{ sign(line) }}</span>
          <span class="file-diff-text"
            ><span
              v-for="(segment, s) in line.segments"
              :key="s"
              :class="{ 'file-diff-em': segment.emphasized }"
              >{{ segment.text }}</span
            ></span
          >
        </div>
      </template>
      <template v-else>
        <div
          v-for="(row, i) in shown.rows"
          :key="i"
          class="file-diff-row"
        >
          <template
            v-for="(cell, c) in [row.base, row.target]"
            :key="c"
          >
            <div
              v-if="cell === null"
              class="file-diff-line file-diff-line--empty"
            />
            <div
              v-else
              class="file-diff-line"
              :class="`file-diff-line--${cell.kind}`"
            >
              <span class="file-diff-num">{{
                (c === 0 ? cell.base_line : cell.target_line) ?? ''
              }}</span>
              <span class="file-diff-sign">{{ sign(cell) }}</span>
              <span class="file-diff-text"
                ><span
                  v-for="(segment, s) in cell.segments"
                  :key="s"
                  :class="{ 'file-diff-em': segment.emphasized }"
                  >{{ segment.text }}</span
                ></span
              >
            </div>
          </template>
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.file-diff-body {
  max-height: 60vh;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  line-height: 1.6;
}

.file-diff-hunk {
  position: sticky;
  left: 0;
  padding: var(--space-2) var(--space-5);
  background: var(--bg-card);
  border-block: 1px solid var(--border-subtle);
  color: var(--text-secondary);
}

.file-diff-hunk:first-child {
  border-top: 0;
}

.file-diff-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.file-diff-row > .file-diff-line:first-child {
  border-right: 1px solid var(--border-subtle);
}

.file-diff-line {
  display: grid;
  align-items: start;
}

.file-diff-body--inline .file-diff-line {
  grid-template-columns: 3.5em 3.5em 1.5em minmax(0, 1fr);
}

.file-diff-body--split .file-diff-line {
  grid-template-columns: 3.5em 1.5em minmax(0, 1fr);
}

.file-diff-line--removed {
  background: var(--danger-subtle);
}

.file-diff-line--added {
  background: var(--success-subtle);
}

.file-diff-line--empty {
  background: var(--bg-hover);
}

.file-diff-num {
  padding-right: var(--space-4);
  text-align: right;
  color: var(--text-muted);
  user-select: none;
}

.file-diff-sign {
  color: var(--text-secondary);
  user-select: none;
}

.file-diff-line--removed .file-diff-sign {
  color: var(--danger);
}

.file-diff-line--added .file-diff-sign {
  color: var(--success);
}

.file-diff-text {
  padding-right: var(--space-5);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.file-diff-line--removed .file-diff-em {
  background: color-mix(in srgb, var(--danger) 30%, transparent);
}

.file-diff-line--added .file-diff-em {
  background: color-mix(in srgb, var(--success) 30%, transparent);
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * One two-line row of the archive screens: a bold name over a muted line of
 * facts, the size (and a time) on the right. Archives in the list, files and
 * folders in an archive, search hits, the changes between two archives and
 * the database dumps an archive holds are all this one row, so the list and
 * the archive beside it read the same way.
 *
 * A row that only opens something is a button (`as="button"`); a row that
 * also carries a checkbox or a button of its own stays a `div`, since a button
 * may not hold another control, and names its own focusable parts.
 */
withDefaults(
  defineProps<{
    title?: string
    /** A machine name - an archive, a path - rather than words. */
    mono?: boolean
    size?: string | null
    time?: string | null
    /** The full timestamp behind `time`, as its tooltip. */
    timeTitle?: string | null
    selected?: boolean
    /** Something already on its way out, e.g. an archive being deleted. */
    dim?: boolean
    /** Whether a click on the row does something. */
    clickable?: boolean
    as?: 'div' | 'button'
  }>(),
  {
    title: '',
    mono: true,
    size: null,
    time: null,
    timeTitle: null,
    selected: false,
    dim: false,
    clickable: false,
    as: 'div',
  },
)

defineEmits<{ click: [event: MouseEvent] }>()

defineSlots<{
  /** A checkbox, an icon or a status dot in front of the text. */
  lead?: () => unknown
  /** Replaces the plain title, e.g. with a button or highlighted matches. */
  title?: () => unknown
  facts?: () => unknown
  /** A control at the row's far right, after the size. */
  end?: () => unknown
}>()
</script>

<template>
  <component
    :is="as"
    class="browser-row"
    :class="{
      'browser-row--selected': selected,
      'browser-row--clickable': clickable || as === 'button',
      'browser-row--dim': dim,
    }"
    v-bind="as === 'button' ? { type: 'button' } : {}"
    @click="$emit('click', $event)"
  >
    <slot name="lead" />
    <span class="browser-row-main">
      <span
        class="browser-row-title"
        :class="{ 'browser-row-title--mono': mono }"
      >
        <slot name="title">{{ title }}</slot>
      </span>
      <span
        v-if="$slots.facts"
        class="browser-row-facts"
      >
        <slot name="facts" />
      </span>
    </span>
    <span
      v-if="size !== null || time !== null"
      class="browser-row-side"
    >
      <span
        v-if="size !== null"
        class="browser-row-size"
        >{{ size }}</span
      >
      <time
        v-if="time !== null"
        class="browser-row-time"
        :title="timeTitle ?? undefined"
        >{{ time }}</time
      >
    </span>
    <slot name="end" />
  </component>
</template>

<style scoped>
.browser-row {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  width: 100%;
  min-width: 0;
  padding: var(--space-4) var(--space-7);
  border: 0;
  background: none;
  font: inherit;
  font-size: var(--fs-sm);
  color: var(--text-primary);
  text-align: left;
}

/* Rows are split by a hairline; a group's caption carries the line above
   the group instead (see `BrowserRowGroup`), so none runs under it. */
.browser-row + .browser-row {
  border-top: 1px solid var(--border-subtle);
}

.browser-row--clickable {
  cursor: pointer;
  transition: background var(--duration-fast);
}

.browser-row--clickable:hover {
  background: var(--bg-hover);
}

.browser-row--selected,
.browser-row--selected:hover {
  background: var(--accent-subtle);
}

.browser-row--dim .browser-row-main {
  opacity: 0.55;
}

.browser-row-main {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.browser-row-title {
  font-weight: 600;
  overflow-wrap: anywhere;
}

.browser-row-title--mono {
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.browser-row--selected .browser-row-title {
  color: var(--accent);
}

.browser-row-facts {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-1) var(--space-3);
  min-width: 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  overflow-wrap: anywhere;
}

.browser-row-side {
  display: flex;
  flex: none;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.browser-row-size {
  color: var(--text-secondary);
  white-space: nowrap;
}

.browser-row-time {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  white-space: nowrap;
}
</style>

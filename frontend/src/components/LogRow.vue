<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, useSlots } from 'vue'
import type { BadgeTone } from '../utils/badge'

/**
 * One entry of a log or history list - backup runs, system events, restores,
 * server and browser logs, the audit trail, notification deliveries. Every
 * one of them is the same row: a status dot, a bold title with a muted
 * secondary title beside it, a line of muted facts under that, and the time
 * (plus a badge, only when the entry is notable) on the right. Clicking the
 * row opens its detail underneath, inside the same card; the entry's actions
 * live there, never on the closed row. Rows go inside a `LogList`.
 */
const props = withDefaults(
  defineProps<{
    /** The dot's colour, in the badge tones. */
    tone: BadgeTone
    title: string
    subtitle?: string | null
    /** Short facts for the second line; empty ones are skipped. */
    facts?: readonly (string | null | undefined)[]
    /** The already formatted time shown on the right. */
    time?: string | null
    /** The full timestamp, as the time's tooltip. */
    timeTitle?: string | null
    /** Whether the detail is open. The list owns this, so it can keep one open at a time. */
    expanded?: boolean
    /** An entry already dealt with (acknowledged), shown but stepped back. */
    dimmed?: boolean
    /** A monospace title, for a log message or a path. */
    mono?: boolean
  }>(),
  {
    subtitle: null,
    facts: () => [],
    time: null,
    timeTitle: null,
    expanded: false,
    dimmed: false,
    mono: false,
  },
)

const emit = defineEmits<{ toggle: [] }>()

defineSlots<{
  /** Badges beside the time; only for an entry that needs noticing. */
  badges?: () => unknown
  /** Buttons acting on the entry, right-aligned at the top of the open detail. */
  actions?: () => unknown
  /** What the open row shows. Without it (and without actions) the row does not open. */
  detail?: () => unknown
}>()

const slots = useSlots()

const expandable = computed(
  (): boolean => slots.detail !== undefined || slots.actions !== undefined,
)

const open = computed((): boolean => props.expanded && expandable.value)

const dotClass = computed((): string =>
  props.tone === 'neutral' ? '' : `status-dot--${props.tone}`,
)

const shownFacts = computed((): string[] =>
  props.facts.filter((f): f is string => f !== null && f !== undefined && f !== ''),
)

function toggle(): void {
  if (expandable.value) emit('toggle')
}
</script>

<template>
  <div
    class="log-row"
    :class="{ 'log-row--open': open, 'log-row--dim': dimmed }"
  >
    <component
      :is="expandable ? 'button' : 'div'"
      class="log-row-summary"
      :class="{ 'log-row-summary--button': expandable }"
      v-bind="expandable ? { type: 'button', 'aria-expanded': open } : {}"
      @click="toggle"
    >
      <span
        class="status-dot log-row-dot"
        :class="dotClass"
      />
      <span class="log-row-text">
        <span class="log-row-head">
          <span
            class="log-row-title"
            :class="{ 'log-row-title--mono': mono }"
            :title="title"
            >{{ title }}</span
          >
          <span
            v-if="subtitle"
            class="log-row-subtitle"
            :title="subtitle"
            >{{ subtitle }}</span
          >
        </span>
        <span
          v-if="shownFacts.length > 0"
          class="log-row-facts"
        >
          <span
            v-for="(fact, i) in shownFacts"
            :key="i"
            class="log-row-fact"
            >{{ fact }}</span
          >
        </span>
      </span>
      <span class="log-row-end">
        <time
          v-if="time"
          class="log-row-time"
          :title="timeTitle ?? undefined"
          >{{ time }}</time
        >
        <span
          v-if="$slots.badges"
          class="log-row-badges"
        >
          <slot name="badges" />
        </span>
      </span>
    </component>

    <div
      v-if="open"
      class="log-row-detail"
    >
      <div
        v-if="$slots.actions"
        class="log-row-actions"
      >
        <slot name="actions" />
      </div>
      <slot name="detail" />
    </div>
  </div>
</template>

<style scoped>
.log-row + .log-row {
  border-top: 1px solid var(--border-subtle);
}

.log-row--open {
  background: var(--bg-hover);
}

/* Only reachable once a filter shows entries already dealt with. Stepped
   back so they read as reviewed next to the ones that still need attention. */
.log-row--dim {
  opacity: 0.6;
}

/* dot | two lines of text | time and badges */
.log-row-summary {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: start;
  gap: var(--space-2) var(--space-5);
  width: 100%;
  padding: var(--space-5) var(--space-6);
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  color: var(--text-primary);
  background: none;
  border: none;
  border-radius: 0;
}

.log-row-summary--button {
  cursor: pointer;
  transition: background var(--duration-fast);
}

.log-row-summary--button:hover {
  background: var(--bg-hover);
}

.log-row-dot {
  margin-top: 0.45em;
}

.log-row-text,
.log-row-end {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.log-row-end {
  align-items: flex-end;
}

.log-row-head {
  display: flex;
  align-items: baseline;
  gap: var(--space-4);
  min-width: 0;
}

.log-row-title,
.log-row-subtitle,
.log-row-facts {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* The title gives way last: the secondary title shrinks first. */
.log-row-title {
  flex: 0 1 auto;
  font-weight: 600;
}

.log-row-title--mono {
  font-family: var(--mono);
  font-weight: 500;
}

.log-row-subtitle {
  flex: 0 1000 auto;
  color: var(--text-secondary);
}

.log-row-facts {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.log-row-fact + .log-row-fact::before {
  content: '\00B7';
  margin: 0 var(--space-3);
}

.log-row-time {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  white-space: nowrap;
}

.log-row-badges {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  align-items: center;
  gap: var(--space-3);
}

.log-row-detail {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  min-width: 0;
  padding: var(--space-6) var(--space-6) var(--space-7);
  border-top: 1px solid var(--border-subtle);
}

.log-row-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-4);
}
</style>

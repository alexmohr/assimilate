<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="T extends string">
import { shallowRef, watch } from 'vue'
import { ChevronRight } from '@lucide/vue'

/**
 * The shell every settings tab uses (agent, schedule, repository, repository
 * host, dependency host): its sections as collapsible rows inside one
 * bordered card, the same card the Overview tab's repository rows sit in.
 * It replaced a side rail that collapsed into a second, wrapping row of
 * tabs on a phone, under the page's own tab bar. Several sections may be
 * open at once.
 *
 * `?section=` comes from the URL, so a section this viewer has no row for
 * must not render behind their back just because they typed its name - the
 * first section they do have opens instead.
 */
export interface SettingsSectionOption<V extends string> {
  id: V
  label: string
  /** Tones the rail item red, for a section that destroys something. */
  danger?: boolean
}

/**
 * A rail's sections, of which there is always at least one - a rail with no
 * sections has no first section to fall back to, and a settings tab showing
 * nothing at all is not a state any caller wants. The tuple says so to the
 * compiler, so callers whose list shrinks by role cannot shrink it to nothing
 * and no unreachable empty-state branch has to be written or tested here.
 */
export type SettingsSections<V extends string> = readonly [
  SettingsSectionOption<V>,
  ...SettingsSectionOption<V>[],
]

const props = defineProps<{
  sections: SettingsSections<T>
  /** The section named in the URL; it starts open. */
  section: T
  /** Names the group for assistive tech, e.g. "Agent settings sections". */
  label: string
}>()

const emit = defineEmits<{ 'update:section': [value: T] }>()

function initialOpen(): Set<T> {
  const requested = props.sections.some((s) => s.id === props.section)
    ? props.section
    : props.sections[0].id
  return new Set([requested])
}

const open = shallowRef<ReadonlySet<T>>(initialOpen())

watch(
  () => props.section,
  (s) => {
    if (props.sections.some((x) => x.id === s)) open.value = new Set([...open.value, s])
  },
)

function toggle(id: T): void {
  const next = new Set(open.value)
  if (next.has(id)) {
    next.delete(id)
  } else {
    next.add(id)
    emit('update:section', id)
  }
  open.value = next
}

function setAll(expand: boolean): void {
  open.value = expand ? new Set(props.sections.map((s) => s.id)) : new Set()
}
</script>

<template>
  <div
    role="group"
    :aria-label="label"
  >
    <div
      v-if="sections.length > 1"
      class="accordion-tools"
    >
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        @click="setAll(open.size < sections.length)"
      >
        {{ open.size < sections.length ? 'Expand all' : 'Collapse all' }}
      </button>
    </div>
    <div class="accordion-list">
      <section
        v-for="s in sections"
        :key="s.id"
        class="panel panel--sectioned"
      >
        <button
          :id="`acc-head-${s.id}`"
          type="button"
          class="accordion-head"
          :aria-expanded="open.has(s.id)"
          :aria-controls="`acc-body-${s.id}`"
          @click="toggle(s.id)"
        >
          <ChevronRight
            :size="16"
            class="accordion-chevron"
            :class="{ 'accordion-chevron--open': open.has(s.id) }"
          />
          <span
            class="accordion-title"
            :class="{ 'accordion-title--danger': s.danger }"
            >{{ s.label }}</span
          >
        </button>
        <div
          v-if="open.has(s.id)"
          :id="`acc-body-${s.id}`"
          class="accordion-body settings-pane"
          role="region"
          :aria-labelledby="`acc-head-${s.id}`"
        >
          <slot :section="s.id" />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.accordion-tools {
  display: flex;
  justify-content: flex-end;
  margin-bottom: var(--space-3);
}

/* Each section is a card of its own, a little apart from the next. */
.accordion-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.accordion-head {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  width: 100%;
  padding: var(--space-5) var(--space-6);
  border: none;
  background: none;
  font: inherit;
  font-weight: 600;
  color: var(--text-primary);
  text-align: left;
  cursor: pointer;
}

.accordion-head:hover {
  background: var(--bg-hover);
}

.accordion-chevron {
  flex: none;
  color: var(--text-muted);
  transition: transform var(--duration-base);
}

.accordion-chevron--open {
  transform: rotate(90deg);
}

.accordion-title {
  flex: 1;
  min-width: 0;
}

.accordion-title--danger {
  color: var(--danger);
}

.accordion-body {
  border-top: 1px solid var(--border-subtle);
  padding: var(--space-6);
}

/* The box already frames the section; the nest's left rule would be a
   third vertical line beside the box edge and the indent. */
.accordion-body :deep(.pane-nest) {
  border-left: none;
  padding-left: 0;
  margin-left: 0;
}

/* A list or table inside a section: the section's card is the frame already,
   so the list keeps only its hairlines between rows. */
.accordion-body :deep(.log-list),
.accordion-body :deep(.rows),
.accordion-body :deep(.list-group),
.accordion-body :deep(.table-wrap--framed) {
  border: none;
  border-radius: 0;
  background: none;
}

@media (prefers-reduced-motion: reduce) {
  .accordion-chevron {
    transition: none;
  }
}
</style>

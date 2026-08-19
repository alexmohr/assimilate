<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="T extends string">
import { ref, type Component } from 'vue'

export interface TabOption<V extends string> {
  id: V
  label: string
  icon?: Component
  /**
   * Optional tally rendered after the label, e.g. how many backups a tab
   * holds. Zero is rendered rather than hidden: on an imported host "0" is
   * the point, and a tab that silently loses its count reads as a different
   * tab depending on which host you opened.
   */
  count?: number
}

const props = defineProps<{
  tabs: readonly TabOption<T>[]
  modelValue: T
  /** Names the tab strip for assistive tech, e.g. "Repository sections". */
  label: string
}>()

const emit = defineEmits<{ 'update:modelValue': [value: T] }>()

const buttons = ref<HTMLButtonElement[]>([])

function select(id: T): void {
  emit('update:modelValue', id)
}

/**
 * Roving focus. A tab strip is one stop in the Tab order; Left/Right (and
 * Home/End) move between the tabs inside it.
 */
function onKeydown(e: KeyboardEvent, index: number): void {
  const last = props.tabs.length - 1
  let next: number | null = null
  if (e.key === 'ArrowRight') next = index === last ? 0 : index + 1
  else if (e.key === 'ArrowLeft') next = index === 0 ? last : index - 1
  else if (e.key === 'Home') next = 0
  else if (e.key === 'End') next = last
  if (next === null) return

  e.preventDefault()
  const target = props.tabs[next]
  if (!target) return
  select(target.id)
  buttons.value[next]?.focus()
}
</script>

<template>
  <!--
    The tablist is nested rather than being the row itself: a `tablist` may
    only contain `tab` children, and the trailing slot holds things that are
    not tabs (the schedule strip's "Logs" link, for one). Keeping them
    siblings of the tablist rather than inside it makes the markup honest
    while leaving them on the same row.
  -->
  <div class="tabs">
    <div
      class="tablist"
      role="tablist"
      :aria-label="label"
    >
      <button
        v-for="(tab, index) in tabs"
        :key="tab.id"
        ref="buttons"
        type="button"
        role="tab"
        class="tab"
        :class="{ active: tab.id === modelValue }"
        :aria-selected="tab.id === modelValue"
        :tabindex="tab.id === modelValue ? 0 : -1"
        @click="select(tab.id)"
        @keydown="onKeydown($event, index)"
      >
        <component
          :is="tab.icon"
          v-if="tab.icon"
          :size="14"
          aria-hidden="true"
        />
        {{ tab.label }}
        <span
          v-if="tab.count !== undefined"
          class="tab-count"
          >{{ tab.count }}</span
        >
      </button>
    </div>
    <slot name="trailing" />
  </div>
</template>

<style scoped>
.tabs {
  align-items: center;
}

/* Laid out as its own row so the tabs sit flush against `.tabs`' bottom
   rule exactly as they did when they were its direct children. */
.tablist {
  display: flex;
  align-items: center;
}

.tab {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
}

.tab-count {
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

.tab.active .tab-count {
  color: inherit;
}
</style>

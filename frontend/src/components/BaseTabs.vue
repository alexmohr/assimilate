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
  <div
    class="tabs"
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
    </button>
    <slot name="trailing" />
  </div>
</template>

<style scoped>
.tabs {
  align-items: center;
}

.tab {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
}
</style>

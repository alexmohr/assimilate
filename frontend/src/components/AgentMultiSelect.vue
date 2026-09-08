<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { ChevronDown } from '@lucide/vue'
import type { AgentRow } from '../types/agent'

/**
 * Picks the hosts a schedule runs on. Extracted from `ScheduleSettingsTab`
 * when the creation wizard needed the same control - a dropdown of checkboxes
 * rather than a native multi-select, which is unusable on touch and gives no
 * room for a display name beside a hostname.
 */
const props = defineProps<{
  agents: readonly AgentRow[]
  disabled?: boolean
}>()

const selected = defineModel<number[]>({ required: true })

const open = ref(false)
const wrapper = ref<HTMLElement | null>(null)

function agentName(id: number): string {
  const agent = props.agents.find((a) => a.id === id)
  return agent?.display_name ?? agent?.hostname ?? `Host #${id}`
}

/**
 * One host is named rather than counted: "1 agent selected" tells the operator
 * nothing they did not already know, and the single-host case is the common
 * one. Beyond that a count is all that fits.
 */
function label(): string {
  if (selected.value.length === 0) return 'Select agents...'
  if (selected.value.length === 1) return agentName(selected.value[0])
  return `${selected.value.length} agents selected`
}

function toggle(id: number): void {
  selected.value = selected.value.includes(id)
    ? selected.value.filter((x) => x !== id)
    : [...selected.value, id]
}

function handleClickOutside(event: MouseEvent): void {
  if (open.value && wrapper.value && !wrapper.value.contains(event.target as Node)) {
    open.value = false
  }
}

onMounted(() => document.addEventListener('click', handleClickOutside))
onBeforeUnmount(() => document.removeEventListener('click', handleClickOutside))
</script>

<template>
  <div
    ref="wrapper"
    class="multi-select-wrapper"
  >
    <button
      type="button"
      class="multi-select-trigger"
      :class="{ open }"
      :disabled="disabled"
      @click.stop="open = !open"
    >
      <span class="multi-select-label">{{ label() }}</span>
      <ChevronDown
        :size="14"
        class="disclosure-chevron"
        :class="{ 'disclosure-chevron--open': open }"
      />
    </button>
    <div
      v-if="open"
      class="multi-select-dropdown"
    >
      <label
        v-for="a in agents"
        :key="a.id"
        class="multi-select-item"
      >
        <input
          type="checkbox"
          :checked="selected.includes(a.id)"
          @change="toggle(a.id)"
        />
        <span class="multi-select-name">{{ a.display_name ?? a.hostname }}</span>
      </label>
    </div>
  </div>
</template>

<style scoped>
.multi-select-wrapper {
  position: relative;
}

.multi-select-trigger {
  width: 100%;
  padding: var(--space-4) var(--space-5);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--fs-base);
  outline: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  transition: border-color var(--duration-base);
  box-sizing: border-box;
  text-align: left;
}

.multi-select-trigger:hover:not(:disabled),
.multi-select-trigger.open {
  border-color: var(--accent);
}

.multi-select-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.multi-select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  padding: var(--space-3);
  z-index: 100;
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.multi-select-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--fs-base);
  color: var(--text-secondary);
  transition: background var(--duration-fast);
}

.multi-select-item:hover {
  background: var(--bg-hover);
}

.multi-select-item input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
  flex-shrink: 0;
}

.multi-select-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>

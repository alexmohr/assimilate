<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { ChannelScope } from '../types/generated'

/** One repository, host or schedule a notification channel can be limited to. */
export interface ScopeOption {
  id: number
  label: string
}

type ScopeKey = keyof ChannelScope

const props = defineProps<{
  /** The scope as it stands: the ids ticked in each section. */
  selected: ChannelScope
  repos: ScopeOption[]
  agents: ScopeOption[]
  schedules: ScopeOption[]
}>()

const emit = defineEmits<{ toggle: [type: ScopeKey, id: number] }>()

/**
 * The filter text. The caller owns it so the add wizard keeps it across its
 * steps, and so opening the scope dialog can clear it.
 */
const search = defineModel<string>('search', { required: true })

interface ScopeSection {
  type: ScopeKey
  title: string
  /** Prefixes the option id in the list key, so the three sections never collide. */
  keyPrefix: string
  options: ScopeOption[]
}

function sections(): ScopeSection[] {
  return [
    { type: 'repo_ids', title: 'Repositories', keyPrefix: 'r', options: props.repos },
    { type: 'agent_ids', title: 'Hosts', keyPrefix: 'c', options: props.agents },
    { type: 'schedule_ids', title: 'Schedules', keyPrefix: 's', options: props.schedules },
  ]
}

function filtered(options: ScopeOption[]): ScopeOption[] {
  const q = search.value.toLowerCase().trim()
  if (!q) return options
  return options.filter((o) => o.label.toLowerCase().includes(q))
}

function isSelected(type: ScopeKey, id: number): boolean {
  const arr = props.selected[type]
  return Array.isArray(arr) && arr.includes(id)
}
</script>

<template>
  <input
    v-model="search"
    class="input scope-search"
    type="text"
    placeholder="Search..."
  />
  <div class="scope-sections">
    <template
      v-for="section in sections()"
      :key="section.type"
    >
      <div
        v-if="section.options.length > 0"
        class="scope-section"
      >
        <span class="group-label group-label--lg scope-section-title">{{ section.title }}</span>
        <label
          v-for="opt in filtered(section.options)"
          :key="section.keyPrefix + opt.id"
          class="scope-item"
        >
          <input
            type="checkbox"
            :checked="isSelected(section.type, opt.id)"
            @change="emit('toggle', section.type, opt.id)"
          />
          <span>{{ opt.label }}</span>
        </label>
      </div>
    </template>
  </div>
</template>

<style scoped>
.scope-search {
  margin-bottom: var(--space-5);
}

.scope-sections {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  max-height: 320px;
  overflow-y: auto;
}

.scope-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

/* The shared group label plus the space this list wants under it. */
.scope-section-title {
  margin-bottom: var(--space-2);
}

.scope-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-2) var(--space-3);
  font-size: var(--fs-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.scope-item:hover {
  background: var(--bg-hover);
}

.scope-item input[type='checkbox'] {
  accent-color: var(--accent);
}
</style>

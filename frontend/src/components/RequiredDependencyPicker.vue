<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { RouteLocationRaw } from 'vue-router'
import { protocolLabel } from '../utils/dependencyHost'
import type { DependencyHost } from '../api/dependencyHosts'

/**
 * Picks dependencies from every one there is, as a checklist. One already
 * required from somewhere this form cannot change - an agent's backup
 * defaults, seen from a schedule - stays ticked and locked, with a link to
 * where it can be removed.
 */
export interface LockedDependency {
  /** What requires it, e.g. "web-01's backup defaults". */
  text: string
  /** Where that is set. */
  to: RouteLocationRaw
}

const props = defineProps<{
  options: readonly DependencyHost[]
  /** Accessible name for the group, e.g. "Dependencies for web-01". */
  label: string
  locked?: ReadonlyMap<number, LockedDependency>
}>()

const selected = defineModel<number[]>({ required: true })

function lockFor(id: number): LockedDependency | undefined {
  return props.locked?.get(id)
}

function toggle(id: number, checked: boolean): void {
  const rest = selected.value.filter((existing) => existing !== id)
  selected.value = checked ? [...rest, id] : rest
}
</script>

<template>
  <div
    class="dependency-picker"
    role="group"
    :aria-label="label"
  >
    <label
      v-for="option in options"
      :key="option.id"
      class="dependency-option"
      :class="{ 'dependency-option--locked': lockFor(option.id) }"
    >
      <input
        type="checkbox"
        :checked="lockFor(option.id) !== undefined || selected.includes(option.id)"
        :disabled="lockFor(option.id) !== undefined"
        @change="toggle(option.id, ($event.target as HTMLInputElement).checked)"
      />
      <span class="dependency-option-body">
        <span>
          <strong>{{ option.name }}</strong>
          <span class="muted"> · {{ protocolLabel(option.port) }} · {{ option.address }}</span>
        </span>
        <span
          v-if="lockFor(option.id)"
          class="field-hint"
        >
          Required by
          <RouterLink :to="lockFor(option.id)?.to ?? '/agents'">{{
            lockFor(option.id)?.text
          }}</RouterLink>
        </span>
      </span>
    </label>
  </div>
</template>

<style scoped>
.dependency-picker {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.dependency-option {
  display: flex;
  align-items: flex-start;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--fs-sm);
}

.dependency-option:hover {
  background: var(--bg-hover);
}

.dependency-option--locked {
  cursor: default;
}

.dependency-option input[type='checkbox'] {
  margin-top: var(--space-1);
  flex-shrink: 0;
}

.dependency-option-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}
</style>

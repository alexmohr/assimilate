<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import ToggleSwitch from './ToggleSwitch.vue'
import type { EventType } from '../types/generated'

/**
 * One switch per event type, shared by the add wizard's events step and the
 * events dialog of an existing channel. The caller decides what "on" means:
 * a draft list in the wizard, the saved rules in the dialog.
 */
withDefaults(
  defineProps<{
    eventTypes: readonly EventType[]
    labelFor: (et: EventType) => string
    isEnabled: (et: EventType) => boolean
    /** Locks a switch while its change is in flight. */
    isDisabled?: (et: EventType) => boolean
  }>(),
  { isDisabled: () => false },
)

const emit = defineEmits<{ toggle: [et: EventType] }>()
</script>

<template>
  <div class="events-list">
    <div
      v-for="et in eventTypes"
      :key="et"
      class="event-item"
    >
      <ToggleSwitch
        :model-value="isEnabled(et)"
        :disabled="isDisabled(et)"
        @update:model-value="emit('toggle', et)"
      />
      <span class="event-label">{{ labelFor(et) }}</span>
    </div>
  </div>
</template>

<style scoped>
.events-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.event-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-sm);
}

.event-item:hover {
  background: var(--bg-hover);
}

.event-label {
  font-size: var(--fs-base);
  color: var(--text-secondary);
}
</style>

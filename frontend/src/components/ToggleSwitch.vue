<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
defineProps<{
  modelValue: boolean
  disabled?: boolean
  label?: string
}>()

defineEmits<{
  'update:modelValue': [value: boolean]
}>()
</script>

<template>
  <label
    class="toggle-switch"
    :class="{ disabled }"
  >
    <button
      type="button"
      role="switch"
      :aria-checked="modelValue"
      :aria-label="label"
      :disabled="disabled"
      class="toggle-track"
      :class="{ active: modelValue }"
      @click="$emit('update:modelValue', !modelValue)"
    >
      <span class="toggle-thumb" />
    </button>
    <span
      v-if="$slots.default"
      class="toggle-text"
    >
      <slot />
    </span>
  </label>
</template>

<style scoped>
.toggle-switch {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.toggle-switch.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.toggle-track {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 10px;
  background: var(--border);
  border: none;
  cursor: pointer;
  padding: 0;
  transition: background 0.2s ease;
  flex-shrink: 0;
}

.toggle-track:disabled {
  cursor: not-allowed;
}

.toggle-track.active {
  background: var(--accent);
}

.toggle-thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  transition: transform 0.2s ease;
}

.toggle-track.active .toggle-thumb {
  transform: translateX(16px);
}

.toggle-text {
  font-size: 0.875rem;
  color: var(--text-secondary);
  user-select: none;
}
</style>

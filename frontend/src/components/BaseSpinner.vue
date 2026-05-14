<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
interface Props {
  size?: 'sm' | 'md' | 'lg'
  label?: string
}

withDefaults(defineProps<Props>(), {
  size: 'md',
  label: 'Loading',
})
</script>

<template>
  <div
    class="spinner-wrapper"
    role="status"
    :aria-label="label"
  >
    <svg
      class="spinner-icon"
      :class="`spinner-${size}`"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        stroke-width="3"
        opacity="0.2"
      />
      <path
        d="M12 2a10 10 0 0 1 10 10"
        stroke="currentColor"
        stroke-width="3"
        stroke-linecap="round"
      />
    </svg>
    <span
      v-if="$slots.default"
      class="spinner-text"
    >
      <slot />
    </span>
  </div>
</template>

<style scoped>
.spinner-wrapper {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.spinner-icon {
  animation: spin 0.75s linear infinite;
  color: var(--accent);
}

.spinner-sm {
  width: 1rem;
  height: 1rem;
}

.spinner-md {
  width: 1.5rem;
  height: 1.5rem;
}

.spinner-lg {
  width: 2.5rem;
  height: 2.5rem;
}

.spinner-text {
  font-size: 0.85rem;
  color: var(--text-muted);
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>

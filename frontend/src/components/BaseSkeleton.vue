<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
interface Props {
  variant?: 'text' | 'card' | 'row' | 'circle'
  lines?: number
  width?: string
  height?: string
}

withDefaults(defineProps<Props>(), {
  variant: 'text',
  lines: 1,
  width: undefined,
  height: undefined,
})
</script>

<template>
  <div
    class="skeleton-wrapper"
    aria-hidden="true"
  >
    <template v-if="variant === 'text'">
      <div
        v-for="i in lines"
        :key="i"
        class="skeleton-line"
        :style="{
          width: i === lines && lines > 1 ? '60%' : (width ?? '100%'),
          height: height ?? '0.85rem',
        }"
      />
    </template>
    <div
      v-else-if="variant === 'card'"
      class="skeleton-card"
      :style="{ height: height ?? '8rem' }"
    >
      <div
        class="skeleton-line"
        style="width: 40%; height: 1rem"
      />
      <div
        class="skeleton-line"
        style="width: 100%; height: 0.75rem"
      />
      <div
        class="skeleton-line"
        style="width: 70%; height: 0.75rem"
      />
    </div>
    <div
      v-else-if="variant === 'row'"
      class="skeleton-row"
      :style="{ height: height ?? '2.5rem' }"
    />
    <div
      v-else-if="variant === 'circle'"
      class="skeleton-circle"
      :style="{ width: width ?? '2.5rem', height: height ?? '2.5rem' }"
    />
  </div>
</template>

<style scoped>
.skeleton-wrapper {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.skeleton-line,
.skeleton-row,
.skeleton-circle,
.skeleton-card {
  background: var(--bg-hover);
  border-radius: var(--radius-sm);
  animation: shimmer 1.5s ease-in-out infinite;
}

.skeleton-circle {
  border-radius: 50%;
}

.skeleton-card {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  padding: 1.25rem;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius);
}

@keyframes shimmer {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.4;
  }
}
</style>

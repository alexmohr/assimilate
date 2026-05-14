<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { Component } from 'vue'

interface Props {
  icon?: Component
  title: string
  description?: string
  action?: string
}

defineProps<Props>()

defineEmits<{
  action: []
}>()
</script>

<template>
  <div class="empty-state">
    <div
      v-if="icon"
      class="empty-icon"
    >
      <component
        :is="icon"
        :size="40"
        aria-hidden="true"
      />
    </div>
    <h3 class="empty-title">{{ title }}</h3>
    <p
      v-if="description"
      class="empty-description"
    >
      {{ description }}
    </p>
    <button
      v-if="action"
      class="btn btn-primary empty-action"
      @click="$emit('action')"
    >
      {{ action }}
    </button>
    <slot />
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 3rem 1.5rem;
  text-align: center;
  gap: 0.75rem;
}

.empty-icon {
  color: var(--text-muted);
  opacity: 0.5;
  margin-bottom: 0.5rem;
}

.empty-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
}

.empty-description {
  font-size: 0.85rem;
  color: var(--text-muted);
  max-width: 320px;
  line-height: 1.5;
}

.empty-action {
  margin-top: 0.5rem;
}
</style>

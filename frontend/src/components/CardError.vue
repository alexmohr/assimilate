<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { AlertCircle, AlertTriangle } from '@lucide/vue'

const props = withDefaults(
  defineProps<{
    label: string
    message: string
    tone?: 'danger' | 'warning'
  }>(),
  { tone: 'danger' },
)

const expanded = ref(false)
</script>

<template>
  <div
    class="card-error"
    :class="`tone-${props.tone}`"
    @click.stop
  >
    <button
      class="error-toggle"
      @click="expanded = !expanded"
    >
      <AlertTriangle
        v-if="props.tone === 'warning'"
        :size="12"
      />
      <AlertCircle
        v-else
        :size="12"
      />
      {{ label }}
      <span class="toggle-arrow">{{ expanded ? '▴' : '▾' }}</span>
    </button>
    <pre
      v-if="expanded"
      :class="props.tone === 'warning' ? 'warning-pre' : 'error-pre'"
      >{{ message }}</pre
    >
  </div>
</template>

<style scoped>
.card-error {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.error-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  background: none;
  border: none;
  color: var(--danger);
  font-size: var(--fs-xs);
  font-weight: 500;
  cursor: pointer;
  padding: 0.2rem 0;
}

.error-toggle:hover {
  text-decoration: underline;
}

.toggle-arrow {
  font-size: var(--fs-2xs);
  margin-left: 0.1rem;
}

.card-error.tone-warning .error-toggle {
  color: var(--warning);
}
</style>

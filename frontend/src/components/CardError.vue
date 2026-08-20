<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { AlertCircle, AlertTriangle, ChevronDown } from '@lucide/vue'

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
      type="button"
      :aria-expanded="expanded"
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
      <ChevronDown
        :size="12"
        class="disclosure-chevron"
        :class="{ 'disclosure-chevron--open': expanded }"
      />
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
  gap: var(--space-3);
}

.error-toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  background: none;
  border: none;
  color: var(--danger);
  font-size: var(--fs-xs);
  font-weight: 500;
  cursor: pointer;
  padding: var(--space-2) 0;
}

.error-toggle:hover {
  text-decoration: underline;
}

.toggle-arrow {
  font-size: var(--fs-2xs);
  margin-left: var(--space-1);
}

.card-error.tone-warning .error-toggle {
  color: var(--warning);
}
</style>

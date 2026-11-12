<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { TOKEN_EXPIRY_CHOICES, tokenExpiresAt, type TokenExpiryChoice } from '../utils/apiToken'
import { formatDate } from '../utils/format'

/**
 * When a new API token stops working: never, after 30 days, 90 days or a
 * year, or at the end of a picked day. Shared by the Profile's and the admin
 * Tokens page's create dialogs.
 */
const choice = defineModel<TokenExpiryChoice>({ required: true })
const date = defineModel<string>('date', { required: true })

defineProps<{ idPrefix: string }>()

/** Tomorrow, the earliest day a token can be set to expire on. */
const minDate = computed(() => {
  const d = new Date(Date.now() + 86_400_000)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
})

const resolved = computed(() => tokenExpiresAt(choice.value, date.value))
</script>

<template>
  <div class="field">
    <label
      class="field-label"
      :for="`${idPrefix}-expiry`"
      >Expires</label
    >
    <div class="expiry-row">
      <select
        :id="`${idPrefix}-expiry`"
        v-model="choice"
        class="input"
      >
        <option
          v-for="option in TOKEN_EXPIRY_CHOICES"
          :key="option.value"
          :value="option.value"
        >
          {{ option.label }}
        </option>
      </select>
      <input
        v-if="choice === 'date'"
        :id="`${idPrefix}-expiry-date`"
        v-model="date"
        type="date"
        class="input"
        :min="minDate"
        aria-label="Expiry date"
      />
    </div>
    <span class="field-hint">
      {{
        resolved
          ? `Stops working on ${formatDate(resolved)}.`
          : choice === 'date'
            ? 'Pick the day it stops working.'
            : 'Works until it is deleted.'
      }}
    </span>
  </div>
</template>

<style scoped>
.expiry-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
}

.expiry-row .input {
  flex: 1 1 10rem;
  min-width: 0;
}
</style>

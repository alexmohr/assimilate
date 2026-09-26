<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { X } from '@lucide/vue'
import { keepsSavedValue, newHeaderRow, type WebhookHeaderRow } from '../utils/webhookHeaders'

/**
 * A webhook channel's custom HTTP headers, one name/value row each.
 *
 * Every value is treated as a secret - it is usually an `Authorization`
 * token - so the server stores it encrypted and never sends it back. A saved
 * header therefore shows its name with an empty value field, and leaving that
 * field blank keeps the saved value.
 */
defineProps<{
  /** Names of saved values the current URL requires typing again. */
  needsReentry?: string[]
}>()

const rows = defineModel<WebhookHeaderRow[]>({ required: true })

function addHeader(): void {
  rows.value = [...rows.value, newHeaderRow()]
}

function removeHeader(index: number): void {
  rows.value = rows.value.filter((_, i) => i !== index)
}
</script>

<template>
  <div class="field">
    <label class="field-label">Headers</label>
    <div class="header-list">
      <div
        v-for="(row, index) in rows"
        :key="row.id"
        class="header-row"
        data-testid="webhook-header-row"
      >
        <input
          v-model="row.name"
          class="input input-sm mono"
          placeholder="Authorization"
          :aria-label="`Header ${index + 1} name`"
          data-testid="webhook-header-name"
        />
        <input
          v-model="row.value"
          class="input input-sm"
          type="password"
          autocomplete="new-password"
          :placeholder="keepsSavedValue(row) ? 'Saved - leave blank to keep it' : 'Value'"
          :aria-label="`Header ${index + 1} value`"
          data-testid="webhook-header-value"
        />
        <button
          type="button"
          class="btn btn-sm btn-danger"
          :title="`Remove header ${index + 1}`"
          :aria-label="`Remove header ${index + 1}`"
          @click="removeHeader(index)"
        >
          <X :size="14" />
        </button>
      </div>
      <button
        type="button"
        class="btn btn-sm btn-ghost header-add"
        data-testid="webhook-add-header"
        @click="addHeader"
      >
        + Add header
      </button>
    </div>
    <span class="field-hint">
      Header values are stored encrypted and never shown again. Leave a saved value blank to keep
      it.
    </span>
    <span
      v-if="needsReentry && needsReentry.length > 0"
      class="form-error"
      data-testid="webhook-header-reentry"
    >
      The URL now points at a different host. Enter the saved value of
      {{ needsReentry.join(', ') }} again, or remove it.
    </span>
  </div>
</template>

<style scoped>
.header-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.header-row {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 3fr) auto;
  gap: var(--space-3);
  align-items: center;
}

.header-add {
  align-self: flex-start;
}
</style>

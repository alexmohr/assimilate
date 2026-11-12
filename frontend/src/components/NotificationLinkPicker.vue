<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { NOTIFICATION_LINK_OPTIONS, type NotificationLink } from '../utils/notificationLink'

/**
 * Where a channel's notifications link to: one choice from a list, plus the
 * address when it is your own. Used by the new-channel wizard and the
 * channel's Link section alike.
 */
const link = defineModel<NotificationLink>({ required: true })

defineProps<{
  /** Prefix for the inputs' ids, so two pickers on one page stay apart. */
  idPrefix: string
}>()
</script>

<template>
  <div
    class="link-picker"
    role="radiogroup"
    aria-label="Where notifications link to"
  >
    <label
      v-for="option in NOTIFICATION_LINK_OPTIONS"
      :key="option.value"
      class="link-option"
      :class="{ 'link-option--checked': link.target === option.value }"
    >
      <input
        :id="`${idPrefix}-${option.value}`"
        type="radio"
        :name="`${idPrefix}-target`"
        :value="option.value"
        :checked="link.target === option.value"
        @change="link = { target: option.value, url: link.url }"
      />
      <span class="link-option-text">
        <span class="link-option-label">{{ option.label }}</span>
        <span class="link-option-hint">{{ option.hint }}</span>
      </span>
    </label>
    <div
      v-if="link.target === 'custom'"
      class="field link-url"
    >
      <label
        class="field-label"
        :for="`${idPrefix}-url`"
        >Address</label
      >
      <input
        :id="`${idPrefix}-url`"
        class="input mono"
        :value="link.url ?? ''"
        placeholder="https://status.example.com"
        @input="link = { target: 'custom', url: ($event.target as HTMLInputElement).value }"
      />
    </div>
  </div>
</template>

<style scoped>
/* One list split by hairlines, like every other list inside a card. */
.link-picker {
  display: flex;
  flex-direction: column;
}

.link-option {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
  padding: var(--space-5) 0;
  border-top: 1px solid var(--border-subtle);
  cursor: pointer;
}

.link-option:first-child {
  border-top: none;
  padding-top: 0;
}

.link-option input {
  margin-top: 3px;
  accent-color: var(--accent);
}

.link-option-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.link-option-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.link-option--checked .link-option-label {
  color: var(--accent);
}

.link-option-hint {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.link-url {
  padding-left: calc(13px + var(--space-5));
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * A `?` button beside a label or a pane head, disclosing the prose that used
 * to sit permanently under it. Every settings pane on the agent, repository
 * and schedule detail pages carried its explanation in the page at all times,
 * read once and then re-read on every visit after.
 *
 * Click-toggled rather than hover-shown, so it behaves the same on a phone as
 * on a desk, and so the popover can hold a link or `<code>` without losing it
 * the moment the pointer leaves.
 */
import { ref } from 'vue'
import { CircleQuestionMark } from '@lucide/vue'
import { useHelpHint } from '../composables/useHelpHint'

withDefaults(
  defineProps<{
    /** Names the button for assistive tech, e.g. "Catch up missed runs". */
    label: string
    /**
     * Opens the popover toward the left instead of the right, for a button
     * that sits at the end of its row and would otherwise overflow.
     */
    align?: 'start' | 'end'
  }>(),
  { align: 'start' },
)

const root = ref<HTMLElement | null>(null)
const open = useHelpHint(root)
</script>

<template>
  <span
    ref="root"
    class="help-hint"
  >
    <button
      type="button"
      class="help-hint-btn"
      :aria-expanded="open"
      :aria-label="`Help: ${label}`"
      @click="open = !open"
    >
      <CircleQuestionMark :size="14" />
    </button>
    <span
      v-if="open"
      class="help-hint-pop"
      :class="{ 'help-hint-pop--end': align === 'end' }"
      role="note"
    >
      <slot />
    </span>
  </span>
</template>

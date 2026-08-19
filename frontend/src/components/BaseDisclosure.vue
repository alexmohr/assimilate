<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { ChevronDown } from '@lucide/vue'

/**
 * A labelled section that opens and closes.
 *
 * There were five of these, no two alike: one set `aria-expanded`, two rotated
 * a chevron without it, and one drew its caret as a literal character. This
 * is the one shape: the toggle state announced, the chevron rotating on the shared
 * duration token, and an optional badge saying what is inside without opening
 * it (Default / Customized, a count, a status).
 */
const props = withDefaults(
  defineProps<{
    title: string
    /** Short state shown on the closed head, e.g. "Default" or "3 warnings". */
    badge?: string | null
    /** Tone for that badge; neutral unless the state deserves attention. */
    badgeTone?: 'neutral' | 'warning' | 'danger' | 'info'
    /**
     * Starts open. For a section that is part of the task rather than a
     * detail of it - the deploy dialog's service unit on a first-time
     * install, where there is no established default yet.
     */
    defaultOpen?: boolean
  }>(),
  { badge: null, badgeTone: 'neutral', defaultOpen: false },
)

const open = ref(props.defaultOpen)
</script>

<template>
  <div>
    <button
      type="button"
      class="disclosure-head"
      :aria-expanded="open"
      @click="open = !open"
    >
      <ChevronDown
        :size="14"
        class="disclosure-chevron"
        :class="{ 'disclosure-chevron--open': open }"
      />
      <span class="disclosure-title">{{ title }}</span>
      <span
        v-if="badge"
        class="badge"
        :class="`badge--${badgeTone}`"
        >{{ badge }}</span
      >
    </button>
    <div
      v-show="open"
      class="disclosure-body"
    >
      <slot />
    </div>
  </div>
</template>

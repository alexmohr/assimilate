<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { useClipboard } from '../composables/useClipboard'

/** A shell snippet in the `.token-box` look, with a copy button. */
defineProps<{
  text: string
  /** Names the snippet above it, e.g. "Download and verify". */
  label?: string
  testId?: string
}>()

const { copied, copy } = useClipboard()
</script>

<template>
  <div class="field">
    <span
      v-if="label"
      class="field-label"
      >{{ label }}</span
    >
    <div class="token-box token-box--block">
      <code
        class="token-text token-text--plain"
        :data-testid="testId"
        >{{ text }}</code
      >
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        @click="copy(text)"
      >
        {{ copied ? 'Copied!' : 'Copy' }}
      </button>
    </div>
  </div>
</template>

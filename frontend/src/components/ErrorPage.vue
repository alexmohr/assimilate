<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { useRouter } from 'vue-router'

/**
 * The full-screen "this page went wrong" card, shared by `ErrorView` (a
 * runtime or backend failure) and `NotFoundView` (a bad route). Both carried
 * an identical copy of the markup and of all six style rules, differing only
 * in the tone of the numeral.
 */

withDefaults(
  defineProps<{
    /** The status numeral shown above the title. */
    code: string
    title: string
    message: string
    /** `danger` for a failure, `accent` for a route that simply does not exist. */
    tone?: 'danger' | 'accent'
  }>(),
  { tone: 'danger' },
)

const router = useRouter()

function goHome(): void {
  router.push('/')
}
</script>

<template>
  <div class="error-page">
    <div class="error-card">
      <div
        class="error-code"
        :class="`error-code--${tone}`"
      >
        {{ code }}
      </div>
      <h1 class="error-title">{{ title }}</h1>
      <!-- Between the title and the message: the error's source, if known. -->
      <slot name="source" />
      <p class="error-message">{{ message }}</p>
      <slot />
      <button
        class="btn btn-primary"
        @click="goHome"
      >
        Back to Dashboard
      </button>
    </div>
  </div>
</template>

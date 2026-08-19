<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import BaseSpinner from './BaseSpinner.vue'

/**
 * The four states anything that loads can be in, in one order: still loading,
 * failed, loaded but empty, loaded.
 *
 * Every list view assembled these by hand, and the assembly is where they
 * drifted: two views rendered no error branch at all, so a failed fetch left
 * an empty table with nothing to explain it, and page-level failures were
 * written as a centred `.state-msg` in some views and an `.error-banner` in
 * others. Here the order and the shapes are the component's, and a caller
 * cannot leave a state out by forgetting it.
 */
defineProps<{
  loading: boolean
  /** Why the load failed, if it did. */
  error?: string | null
  /** True once loaded with nothing to show, which renders the `empty` slot. */
  empty?: boolean
}>()

defineSlots<{
  default: () => unknown
  empty?: () => unknown
}>()
</script>

<template>
  <BaseSpinner
    v-if="loading"
    size="lg"
  />
  <div
    v-else-if="error"
    class="error-banner"
  >
    {{ error }}
  </div>
  <slot
    v-else-if="empty"
    name="empty"
  />
  <slot v-else />
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The one card a log or history list sits in: `LogRow`s split by hairlines,
 * with an optional header row (a title and its actions) above them.
 */
defineSlots<{
  header?: () => unknown
  default?: () => unknown
}>()
</script>

<template>
  <section class="log-list">
    <div
      v-if="$slots.header"
      class="list-group-header log-list-header"
    >
      <slot name="header" />
    </div>
    <slot />
  </section>
</template>

<style scoped>
.log-list {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-card);
  overflow: hidden;
}

.log-list-header {
  border-bottom: 1px solid var(--border);
}
</style>

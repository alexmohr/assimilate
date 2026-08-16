<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
// Shared two-pane layout (archive list + file browser) so every screen that
// browses borg archives collapses to a single column on narrow viewports
// instead of drifting out of sync with its own copy of the grid.
withDefaults(
  defineProps<{
    // Sizes the list pane to a fixed narrow column instead of splitting the
    // width with the browser. The repo view's list carries five detailed
    // columns and needs the room; the schedule view's carries four narrow
    // ones and would otherwise sit in a half-empty pane on a wide screen.
    narrowList?: boolean
  }>(),
  {
    narrowList: false,
  },
)
</script>

<template>
  <div
    class="archive-browser-layout"
    :class="{ 'layout-narrow-list': narrowList }"
  >
    <slot name="list" />
    <slot name="browser" />
  </div>
</template>

<style scoped>
.archive-browser-layout {
  display: grid;
  grid-template-columns: 1fr 1.2fr;
  gap: 1rem;
  align-items: start;
}

.archive-browser-layout.layout-narrow-list {
  grid-template-columns: 360px 1fr;
}

.archive-browser-layout > * {
  /* Grid items default to min-width: auto, so a wide table/DataTable inside
     either pane can force this track -- and the whole grid -- past the
     viewport instead of shrinking to fit. */
  min-width: 0;
}

@media (max-width: 1024px) {
  /* Both selectors listed so this still wins over the higher-specificity
     .layout-narrow-list rule above and the narrow variant collapses too. */
  .archive-browser-layout,
  .archive-browser-layout.layout-narrow-list {
    grid-template-columns: 1fr;
  }
}
</style>

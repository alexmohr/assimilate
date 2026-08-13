<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
// Shared two-pane layout (archive list + file browser) so every screen that
// browses borg archives collapses to a single column on narrow viewports
// instead of drifting out of sync with its own copy of the grid.
</script>

<template>
  <div class="archive-browser-layout">
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

.archive-browser-layout > * {
  /* Grid items default to min-width: auto, so a wide table/DataTable inside
     either pane can force this track -- and the whole grid -- past the
     viewport instead of shrinking to fit. */
  min-width: 0;
}

@media (max-width: 1100px) {
  .archive-browser-layout {
    grid-template-columns: 1fr;
  }
}
</style>

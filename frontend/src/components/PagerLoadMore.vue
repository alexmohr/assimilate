<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { REPORTS_PAGE_SIZE } from '../composables/useReportsPager'

/**
 * The "load more" footer shared by `RunLogTab` and `ScheduleBackupsTab`:
 * both page through the same `useReportsPager`-backed list and need the
 * same button (capped at a page, disabled and relabelled while a fetch is
 * in flight) plus a note explaining how much of the total is loaded.
 */
withDefaults(
  defineProps<{
    /** Reports loaded so far. */
    loaded: number
    /** The server's true count. */
    total: number
    loadingMore: boolean
    /** Text appended to the button's "Load N " prefix, e.g. "more" or "more runs". */
    loadLabel?: string
  }>(),
  { loadLabel: 'more' },
)

const emit = defineEmits<{ loadMore: [] }>()
</script>

<template>
  <div class="pager-load-more">
    <button
      v-if="loaded < total"
      class="btn btn-sm btn-ghost"
      type="button"
      :disabled="loadingMore"
      @click="emit('loadMore')"
    >
      {{
        loadingMore
          ? 'Loading...'
          : `Load ${Math.min(REPORTS_PAGE_SIZE, total - loaded)} ${loadLabel}`
      }}
    </button>
    <span class="pager-load-more-note"><slot /></span>
  </div>
</template>

<style scoped>
.pager-load-more {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.pager-load-more-note {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>

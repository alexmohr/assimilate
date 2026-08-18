<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onBeforeUnmount, watch } from 'vue'
import { MoreHorizontal } from '@lucide/vue'
import { useEscapeKey } from '../composables/useEscapeKey'
import type { ScheduleRow } from '../types/schedule'

/**
 * The schedule detail page's identity block, shown above the tab strip and so
 * present on every tab. Follows AgentHeader's grammar: one accented slot for
 * the thing that is actionable right now (Run Now/Cancel Backup), everything
 * else - Logs and the destructive Delete - behind the overflow menu. Before
 * this, Logs was a trailing tab-strip link and Delete sat in a full-width
 * Danger Zone card below the form.
 */
defineProps<{
  schedule: ScheduleRow
  /** "Backup" / "Integrity Check" / "Verify (extract dry-run)", already resolved by the parent. */
  typeLabel: string
  /** Cron expression rendered in words, e.g. "Daily at 02:00". */
  cronSummary: string
  backupRunning: boolean
  runNowLoading: boolean
  cancelLoading: boolean
  /** How many of this schedule's targets are currently overdue. */
  overdueCount: number
}>()

const emit = defineEmits<{
  runNow: []
  cancelBackup: []
  logs: []
  delete: []
}>()

const menuOpen = ref(false)
const menuRoot = ref<HTMLElement | null>(null)

useEscapeKey(menuOpen, () => {
  menuOpen.value = false
})

function onDocumentPointerDown(e: PointerEvent): void {
  if (!menuRoot.value?.contains(e.target as Node)) menuOpen.value = false
}

watch(menuOpen, (open) => {
  if (open) document.addEventListener('pointerdown', onDocumentPointerDown)
  else document.removeEventListener('pointerdown', onDocumentPointerDown)
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocumentPointerDown)
})

/** Runs a menu action and closes the menu, so no item has to remember to. */
function fromMenu(action: () => void): void {
  menuOpen.value = false
  action()
}
</script>

<template>
  <header class="schedule-header">
    <div class="schedule-identity">
      <div class="schedule-title-row">
        <h1 class="schedule-name">{{ schedule.name || typeLabel }}</h1>
        <span
          class="badge badge-dot"
          :class="schedule.enabled ? 'badge--success' : 'badge--neutral'"
        >
          {{ schedule.enabled ? 'Enabled' : 'Disabled' }}
        </span>
        <span
          v-if="backupRunning"
          class="badge badge-dot badge--accent"
        >
          Running
        </span>
        <span
          v-if="overdueCount > 0"
          class="badge badge--warning"
        >
          {{ overdueCount }} target{{ overdueCount === 1 ? '' : 's' }} overdue
        </span>
      </div>
      <p class="schedule-subtitle">{{ typeLabel }} &middot; {{ cronSummary }}</p>
    </div>

    <div
      ref="menuRoot"
      class="schedule-actions"
    >
      <button
        v-if="backupRunning"
        class="btn btn-sm btn-danger"
        :disabled="cancelLoading"
        @click="emit('cancelBackup')"
      >
        {{ cancelLoading ? 'Cancelling...' : 'Cancel Backup' }}
      </button>
      <button
        v-else
        class="btn btn-sm btn-primary"
        :disabled="runNowLoading"
        @click="emit('runNow')"
      >
        {{ runNowLoading ? 'Starting...' : 'Run Now' }}
      </button>

      <button
        class="btn btn-sm btn-ghost schedule-menu-toggle"
        type="button"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        aria-label="More schedule actions"
        @click="menuOpen = !menuOpen"
      >
        <MoreHorizontal :size="14" />
      </button>

      <div
        v-if="menuOpen"
        class="schedule-menu"
        role="menu"
      >
        <button
          class="schedule-menu-item"
          role="menuitem"
          type="button"
          @click="fromMenu(() => emit('logs'))"
        >
          Logs
        </button>
        <button
          class="schedule-menu-item schedule-menu-item--danger"
          role="menuitem"
          type="button"
          @click="fromMenu(() => emit('delete'))"
        >
          Delete schedule
        </button>
      </div>
    </div>
  </header>
</template>

<style scoped>
.schedule-header {
  display: flex;
  align-items: flex-start;
  gap: 1rem;
  flex-wrap: wrap;
  margin-bottom: 1rem;
}

.schedule-identity {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
}

.schedule-title-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  flex-wrap: wrap;
}

.schedule-name {
  font-size: var(--fs-xl);
  font-weight: 650;
  letter-spacing: -0.02em;
  margin: 0;
}

.schedule-subtitle {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  margin: 0;
}

.schedule-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  position: relative;
}

.schedule-menu-toggle {
  padding-inline: 0.45rem;
}

.schedule-menu {
  position: absolute;
  top: calc(100% + 0.35rem);
  right: 0;
  z-index: 20;
  min-width: 180px;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  padding: 0.3rem;
  display: flex;
  flex-direction: column;
}

.schedule-menu-item {
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  padding: 0.35rem 0.5rem;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.schedule-menu-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.schedule-menu-item--danger {
  color: var(--danger);
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-sm) var(--radius-sm);
  margin-top: 0.25rem;
  padding-top: 0.45rem;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import DetailHeader from './DetailHeader.vue'
import OverflowMenu from './OverflowMenu.vue'
import { scheduleDisabledLabel } from '../utils/scheduleStatus'
import type { ScheduleRow } from '../types/schedule'

/**
 * The schedule detail page's identity block, shown above the tab strip and so
 * present on every tab. Follows the same grammar as every other detail page:
 * one accented slot for the thing that is actionable right now (Run now /
 * Cancel backup), everything else - Logs and the destructive Delete - behind
 * the overflow menu. Before this, Logs was a trailing tab-strip link and
 * Delete sat in a full-width Danger Zone card below the form.
 */
defineProps<{
  schedule: ScheduleRow
  /** "Backup" / "Integrity check" / "Verify (extract dry-run)", already resolved by the parent. */
  typeLabel: string
  /** Cron expression rendered in words, e.g. "Daily at 02:00". */
  cronSummary: string
  backupRunning: boolean
  runNowLoading: boolean
  cancelLoading: boolean
  /** How many of this schedule's targets are currently overdue. */
  overdueCount: number
  /** How many of this schedule's runs currently show as failed. */
  failedReportCount: number
}>()

const emit = defineEmits<{
  runNow: []
  cancelBackup: []
  logs: []
  delete: []
  cleanFailedReports: []
}>()
</script>

<template>
  <DetailHeader
    :name="schedule.name || typeLabel"
    :subtitle="`${typeLabel} · ${cronSummary}`"
  >
    <template #badges>
      <span
        class="badge"
        :class="schedule.enabled ? 'badge--success' : 'badge--neutral'"
      >
        <span class="badge-dot" />
        {{ scheduleDisabledLabel(schedule) }}
      </span>
      <span
        v-if="backupRunning"
        class="badge badge--accent"
      >
        <span class="badge-dot" />
        Running
      </span>
      <span
        v-if="overdueCount > 0"
        class="badge badge--warning"
      >
        {{ overdueCount }} target{{ overdueCount === 1 ? '' : 's' }} overdue
      </span>
    </template>

    <template #actions>
      <button
        v-if="backupRunning"
        class="btn btn-sm btn-danger"
        :disabled="cancelLoading"
        @click="emit('cancelBackup')"
      >
        {{ cancelLoading ? 'Cancelling...' : 'Cancel backup' }}
      </button>
      <button
        v-else
        class="btn btn-sm btn-primary"
        :disabled="runNowLoading"
        @click="emit('runNow')"
      >
        {{ runNowLoading ? 'Starting...' : 'Run now' }}
      </button>

      <OverflowMenu
        v-slot="{ run }"
        label="More schedule actions"
      >
        <button
          class="overflow-menu-item"
          role="menuitem"
          type="button"
          @click="run(() => emit('logs'))"
        >
          Logs
        </button>
        <button
          v-if="failedReportCount > 0"
          class="overflow-menu-item overflow-menu-item--danger"
          role="menuitem"
          type="button"
          @click="run(() => emit('cleanFailedReports'))"
        >
          Clean up failed backups ({{ failedReportCount }})
        </button>
        <button
          class="overflow-menu-item overflow-menu-item--danger"
          role="menuitem"
          type="button"
          @click="run(() => emit('delete'))"
        >
          Delete schedule
        </button>
      </OverflowMenu>
    </template>
  </DetailHeader>
</template>

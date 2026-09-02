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
 * Cancel backup), everything else - the destructive Delete - behind the
 * overflow menu. Before this, Logs was a trailing tab-strip link, then an
 * overflow item pointing at the Activity page; it's a tab of its own now,
 * the same run history the agent detail page's Logs tab renders, so it no
 * longer needs a header affordance. Delete used to sit in a full-width
 * Danger Zone card below the form.
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

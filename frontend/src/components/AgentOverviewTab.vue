<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatDateShort, formatDuration, relativeTime } from '../utils/format'
import { normalizeBackupStatus, filterSettledReports } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import BackupProgressCard from './BackupProgressCard.vue'
import AgentRunStrip from './AgentRunStrip.vue'
import AgentScheduleRow from './AgentScheduleRow.vue'
import AgentBackupRow from './AgentBackupRow.vue'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'
import type { ScheduleRow } from '../types/schedule'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

/**
 * The landing tab for an agent, which is what you see when a backup failed at
 * 3am. It answers four questions in one screen - is it up, did the last
 * backup work, when does the next one run, is anything overdue - and nothing
 * else. Every setting that used to stack up here now lives under Settings.
 */

/** Elapsed seconds are computed by the parent, which owns the clock. */
export interface LiveBackup {
  targetName: string
  repoId: number | null
  archiveName: string | null
  elapsedSecs: number
  progress: { nfiles: number; originalSize: number; currentPath: string } | null
}

const SCHEDULE_PREVIEW_COUNT = 3
const BACKUP_PREVIEW_COUNT = 5

const props = defineProps<{
  agent: AgentRow
  repos: readonly { id: number; name: string }[]
  schedules: readonly ScheduleRow[]
  health: readonly ScheduleHealthEntry[]
  reports: readonly ReportRow[]
  liveBackups: readonly LiveBackup[]
  cancellingRepoIds: readonly number[]
  repoNameFor: (schedule: ScheduleRow) => string
}>()

const emit = defineEmits<{
  openSchedule: [schedule: ScheduleRow]
  openReport: [report: ReportRow]
  showTab: [tab: 'schedules' | 'backups']
  cancelBackup: [repoId: number]
}>()

function cancelLive(backup: LiveBackup): void {
  if (backup.repoId !== null) emit('cancelBackup', backup.repoId)
}

const settledReports = computed(() => filterSettledReports(props.reports))

const lastBackup = computed(
  () =>
    [...settledReports.value].sort(
      (a, b) => new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime(),
    )[0] ?? null,
)

/**
 * The soonest upcoming run across every schedule targeting this agent.
 * Disabled schedules are skipped: the server still reports a `next_run_at`
 * for them, but it will not fire.
 */
const nextRun = computed(() => {
  const upcoming = props.schedules
    .filter((s) => s.enabled && s.next_run_at !== null)
    .map((s) => ({ schedule: s, at: new Date(s.next_run_at as string).getTime() }))
    .sort((a, b) => a.at - b.at)
  return upcoming[0] ?? null
})

/**
 * Names rather than a total size: `/agents/{hostname}/repos` returns
 * `RepoResponse`, which carries no size - that lives on `RepoWithStats`, and
 * fetching one of those per repository to fill a tile subtitle is not worth a
 * request per repo.
 */
const repoNames = computed(() => props.repos.map((r) => r.name).join(', '))

interface Attention {
  key: string
  severity: 'warning' | 'danger'
  label: string
  message: string
  scheduleId: number | null
}

/**
 * Rendered only when there is something to say. An always-present "all good"
 * banner trains people to skip the place a real warning will appear.
 */
const attention = computed<Attention[]>(() => {
  const items: Attention[] = []
  if (!props.agent.is_imported && props.agent.is_connected === false) {
    items.push({
      key: 'offline',
      severity: 'warning',
      label: 'Offline',
      message: props.agent.last_seen_at
        ? `Agent last checked in ${relativeTime(props.agent.last_seen_at)}.`
        : 'Agent has never checked in.',
      scheduleId: null,
    })
  }
  for (const entry of props.health) {
    if (!entry.is_overdue) continue
    const schedule = props.schedules.find((s) => s.id === entry.schedule_id)
    const name = schedule?.name || entry.target_name
    items.push({
      key: `overdue-${entry.schedule_id}-${entry.repo_id}`,
      severity: 'warning',
      label: 'Overdue',
      message: entry.last_backup_at
        ? `${name} has not run since ${relativeTime(entry.last_backup_at)}.`
        : `${name} has never run.`,
      scheduleId: entry.schedule_id,
    })
  }
  const last = lastBackup.value
  if (last && normalizeBackupStatus(last.status) === 'failed') {
    items.push({
      key: `failed-${last.id}`,
      severity: 'danger',
      label: 'Failed',
      message: `The last backup to ${last.repo_name} failed ${relativeTime(last.finished_at)}.`,
      scheduleId: last.schedule_id,
    })
  }
  return items
})

const schedulePreview = computed(() => props.schedules.slice(0, SCHEDULE_PREVIEW_COUNT))
const backupPreview = computed(() =>
  [...settledReports.value]
    .sort((a, b) => new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime())
    .slice(0, BACKUP_PREVIEW_COUNT),
)

function healthFor(schedule: ScheduleRow): ScheduleHealthEntry[] {
  return props.health.filter((h) => h.schedule_id === schedule.id)
}
</script>

<template>
  <div class="overview-tab">
    <!-- A backup in flight is the most current thing on the page, so it leads. -->
    <BackupProgressCard
      v-for="b in liveBackups"
      :key="b.targetName"
      :badge="b.targetName"
      :repo-id="b.repoId"
      :archive-name="b.archiveName"
      :elapsed-secs="b.elapsedSecs"
      :estimated-remaining-secs="null"
      :progress="b.progress"
      :cancel-loading="b.repoId !== null && cancellingRepoIds.includes(b.repoId)"
      @cancel="cancelLive(b)"
    />

    <div
      v-if="attention.length > 0"
      class="attention"
    >
      <div
        v-for="item in attention"
        :key="item.key"
        class="attention-row"
      >
        <span
          class="badge"
          :class="item.severity === 'danger' ? 'badge--danger' : 'badge--warning'"
          >{{ item.label }}</span
        >
        <span class="attention-message">{{ item.message }}</span>
        <RouterLink
          v-if="item.scheduleId !== null"
          class="attention-link"
          :to="`/schedules/${item.scheduleId}`"
        >
          Open schedule
        </RouterLink>
      </div>
    </div>

    <div class="tiles">
      <div class="tile">
        <span class="stat-label">Last backup</span>
        <span
          v-if="lastBackup"
          class="stat-value stat-value--lg"
        >
          {{ relativeTime(lastBackup.finished_at) }}
          <span
            class="badge"
            :class="backupStatusBadgeClass(lastBackup.status)"
            >{{ normalizeBackupStatus(lastBackup.status) }}</span
          >
        </span>
        <span
          v-else
          class="stat-value stat-value--lg stat-value--empty"
          >Never</span
        >
        <span
          v-if="lastBackup"
          class="stat-sub"
        >
          {{ lastBackup.repo_name }} &middot; {{ formatDuration(lastBackup.duration_secs) }}
        </span>
      </div>

      <div class="tile">
        <span class="stat-label">Next run</span>
        <span
          v-if="nextRun"
          class="stat-value stat-value--lg"
          >{{ formatDateShort(nextRun.schedule.next_run_at) }}</span
        >
        <span
          v-else
          class="stat-value stat-value--lg stat-value--empty"
          >None scheduled</span
        >
        <span
          v-if="nextRun"
          class="stat-sub"
        >
          {{ nextRun.schedule.name || repoNameFor(nextRun.schedule) }} &middot;
          {{ nextRun.schedule.cron_expression }}
        </span>
      </div>

      <div class="tile">
        <span class="stat-label">Repositories</span>
        <span class="stat-value stat-value--lg">{{ repos.length }}</span>
        <span
          v-if="repoNames"
          class="stat-sub stat-sub--truncate"
          :title="repoNames"
          >{{ repoNames }}</span
        >
      </div>

      <div class="tile">
        <span class="stat-label">Recent runs</span>
        <AgentRunStrip :reports="reports" />
      </div>
    </div>

    <section v-if="schedulePreview.length > 0">
      <div class="section-head">
        <h2 class="section-title">Schedules</h2>
        <button
          v-if="schedules.length > schedulePreview.length"
          class="section-link"
          type="button"
          @click="emit('showTab', 'schedules')"
        >
          View all {{ schedules.length }}
        </button>
      </div>
      <div class="rows">
        <AgentScheduleRow
          v-for="s in schedulePreview"
          :key="s.id"
          :schedule="s"
          :repo-name="repoNameFor(s)"
          :health="healthFor(s)"
          @open="emit('openSchedule', s)"
        />
      </div>
    </section>

    <section v-if="backupPreview.length > 0">
      <div class="section-head">
        <h2 class="section-title">Recent backups</h2>
        <button
          v-if="settledReports.length > backupPreview.length"
          class="section-link"
          type="button"
          @click="emit('showTab', 'backups')"
        >
          View all {{ settledReports.length }}
        </button>
      </div>
      <div class="rows">
        <AgentBackupRow
          v-for="r in backupPreview"
          :key="r.id"
          :report="r"
          @open="emit('openReport', r)"
        />
      </div>
    </section>
  </div>
</template>

<style scoped>
/* Base .overview-tab / .attention / .tiles / .tile / .section-* shapes live
   in style.css, shared with ScheduleOverviewTab. Only the attention row's
   trailing link and the empty/truncate stat modifiers are this page's own. */
.attention-link {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--accent);
}

.stat-value--empty {
  color: var(--text-muted);
  font-weight: 500;
}

.stat-sub--truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>

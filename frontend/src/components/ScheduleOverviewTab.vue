<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import type { ScheduleRow } from '../types/schedule'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'
import type { ScheduleTargetResponse } from '../types/generated/ScheduleTargetResponse'
import { computed } from 'vue'
import { formatBytes, formatDateShort, formatDuration, relativeTime } from '../utils/format'
import {
  normalizeBackupStatus,
  filterSettledReports,
  reportMessageLabel,
} from '../utils/backupStatus'
import { scheduleRunStatus } from '../utils/scheduleHealth'
import { failingRepoCount, scheduleRepoRuns, type ScheduleRepoRuns } from '../utils/scheduleRepos'
import { backupStatusBadgeClass, badgeClass } from '../utils/badge'
import type { ScheduleRepoOption } from '../types/schedule'
import BackupProgressCard from './BackupProgressCard.vue'
import AgentRunStrip from './AgentRunStrip.vue'

interface ArchiveProgressData {
  hostname: string
  nfiles: number
  originalSize: number
  currentPath: string
}

/**
 * Replaces the old "Schedule Info" card, which was read-only status wedged
 * between a wall of editable settings cards. This is a dedicated status
 * screen instead: whether anything is overdue, the last/next run, every
 * target's health, and a preview of what the Backups tab holds in full.
 * Every editable field moved out to Settings.
 */
const props = defineProps<{
  schedule: ScheduleRow
  /** Targets of this schedule, for the catch-up markers they carry. */
  targets: readonly ScheduleTargetResponse[]
  repoName: string | null
  /** Every repository this schedule writes into, in write order. */
  repoOptions: readonly ScheduleRepoOption[]
  cronSummary: string
  agentIds: readonly number[]
  agentLabel: (id: number) => string
  healthForAgent: (id: number) => HealthSummaryResponse | null
  connectivityNote: (id: number) => string
  retryingAgentId: number | null
  reports: readonly ReportRow[]
  agents: ReadonlyMap<number, AgentRow>
  backupRunning: boolean
  backupHostname: string | null
  backupArchiveName: string | null
  backupElapsedSecs: number
  estimatedRemainingSecs: number | null
  archiveProgress: ArchiveProgressData | null
}>()

const emit = defineEmits<{
  retry: [agentId: number]
  openLogs: []
  openArchive: [report: ReportRow]
  openReportDetail: [report: ReportRow]
}>()

const BACKUP_PREVIEW_COUNT = 5
const MINUTES_PER_HOUR = 60

/**
 * This schedule's runs, split by the repository they wrote into.
 *
 * A schedule that copies into several repositories used to report one merged
 * outcome here: "2 failed" over a strip mixing both copies, and a preview row
 * naming only the host. That cannot distinguish the offsite copy failing
 * twice from both copies failing once, which is the first thing anyone asks
 * of a page that says a backup failed.
 */
const repoRuns = computed<ScheduleRepoRuns[]>(() =>
  scheduleRepoRuns(props.repoOptions, props.reports),
)

/** One repository is the page's own context; naming it on every row is noise. */
const multiRepo = computed(() => props.repoOptions.length > 1)

function repoStatusLabel(entry: ScheduleRepoRuns): string {
  return entry.status ?? 'never run'
}

function repoStatusBadgeClass(entry: ScheduleRepoRuns): string {
  return entry.last ? backupStatusBadgeClass(entry.last.status) : badgeClass('neutral')
}

/** When this repository last finished a run, and how much it took. */
function repoRunNote(entry: ScheduleRepoRuns): string {
  const last = entry.last
  if (!last) return 'no run yet'
  const when = relativeTime(last.finished_at)
  return last.original_size > 0 ? `${when} · ${formatBytes(last.original_size)}` : when
}

/** How many of this host's repositories ended their last run failed. */
function failingRepos(agentId: number): number {
  return failingRepoCount(repoRuns.value, agentId)
}

/**
 * The repository a run wrote into. `repo_name` comes with the report, but the
 * target list is what this page is about - so a run against a repository the
 * schedule no longer writes to still reports the name it was given.
 */
function repoLabel(report: ReportRow): string {
  const option = props.repoOptions.find((o) => o.id === report.repo_id)
  return option?.name ?? report.repo_name ?? `#${report.repo_id}`
}

/** The occurrence this target missed and will run when its host reconnects. */
function catchUpPendingFor(agentId: number): string | null {
  return props.targets.find((t) => t.agent_id === agentId)?.catch_up_pending_for ?? null
}

const pendingCatchUps = computed(() =>
  props.agentIds.filter((id) => catchUpPendingFor(id) !== null),
)

/** "90 minutes" reads worse than "1.5 hours" only past the hour mark. */
function leadTimeText(minutes: number): string {
  if (minutes < MINUTES_PER_HOUR) return `${minutes} minutes`
  const hours = minutes / MINUTES_PER_HOUR
  return `${Number(hours.toFixed(1))} hours`
}

const catchUpText = computed(() => {
  if (!props.schedule.catch_up_missed_runs) return 'Off'
  return `On, if the next run is at least ${leadTimeText(props.schedule.catch_up_min_lead_minutes)} away`
})

const overdueTargets = computed(() =>
  props.agentIds.filter((id) => props.healthForAgent(id)?.is_overdue),
)

function lastBackupText(id: number): string {
  const at = props.healthForAgent(id)?.last_backup_at
  return at ? relativeTime(at) : 'never'
}

/**
 * Goes through `scheduleRunStatus` rather than normalizing `last_status`
 * here: the health endpoint LEFT JOINs the latest report, so a target that
 * has never run comes back with a null `last_status`, and normalizing that
 * directly would fall through to 'failed' and paint a brand new target red
 * next to its own "last never" text. The helper reports null for "no run
 * yet", which is what the schedule cards' failed/warning chips already use.
 *
 * Both outcome-bearing statuses are mapped, not just 'failed': a target
 * whose last run finished with warnings is not the same as a clean one, and
 * the recent-backups preview below already gives that report its own colour.
 * Overdue shares the warning colour, so its position relative to the
 * warning check does not change what is rendered.
 */
function stripeFor(id: number): 'danger' | 'warning' | 'accent' | 'success' {
  const health = props.healthForAgent(id)
  const status = scheduleRunStatus(health)
  if (status === 'failed') return 'danger'
  if (status === 'warning' || health?.is_overdue) return 'warning'
  if (props.backupRunning && props.backupHostname === props.agentLabel(id)) return 'accent'
  return 'success'
}

function hostLabel(agentId: number | null): string {
  const agent = props.agents.get(agentId ?? 0)
  return agent?.display_name ?? agent?.hostname ?? `#${agentId ?? 0}`
}

const settledReports = computed(() => filterSettledReports(props.reports))

const backupPreview = computed(() =>
  [...settledReports.value]
    .sort((a, b) => new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime())
    .slice(0, BACKUP_PREVIEW_COUNT),
)

/**
 * The Backups tab lists archives, and builds them from exactly these runs -
 * so a preview row offers the jump only when the tab has somewhere to land.
 */
function hasArchive(r: ReportRow): boolean {
  const status = normalizeBackupStatus(r.status)
  return !!r.archive_name && (status === 'success' || status === 'warning')
}

/**
 * Same mapping AgentBackupRow uses, including the muted fallback: a settled
 * report that is neither a success, a warning nor a failure is a cancelled
 * one, and painting that green would contradict the neutral "cancelled"
 * badge sitting next to it in the same row.
 */
function reportStripe(r: ReportRow): 'danger' | 'warning' | 'success' | 'muted' {
  const status = normalizeBackupStatus(r.status)
  if (status === 'success') return 'success'
  if (status === 'warning') return 'warning'
  if (status === 'failed') return 'danger'
  return 'muted'
}
</script>

<template>
  <div class="overview-tab">
    <BackupProgressCard
      v-if="backupRunning"
      :badge="backupHostname"
      :archive-name="backupArchiveName"
      :elapsed-secs="backupElapsedSecs"
      :estimated-remaining-secs="estimatedRemainingSecs"
      :progress="archiveProgress"
    />

    <div
      v-if="overdueTargets.length > 0"
      class="attention"
    >
      <div
        v-for="id in overdueTargets"
        :key="id"
        class="attention-row"
      >
        <span class="badge badge--warning">Overdue</span>
        <span class="attention-message">
          {{ agentLabel(id) }} has not run since {{ lastBackupText(id) }}.
        </span>
        <span
          v-if="connectivityNote(id)"
          class="attention-note"
        >
          {{ connectivityNote(id) }}
        </span>
        <button
          class="btn btn-sm btn-ghost"
          :disabled="retryingAgentId === id"
          @click="emit('retry', id)"
        >
          {{ retryingAgentId === id ? '...' : 'Retry' }}
        </button>
      </div>
    </div>

    <div class="panel">
      <h2 class="panel-title">Schedule info</h2>
      <dl class="info-grid">
        <dt>{{ multiRepo ? 'Repositories' : 'Repository' }}</dt>
        <!--
          Each target with its own last outcome, rather than the comma-joined
          list of names this used to be: the names are already in Settings,
          and what a status screen owes the reader is which copy is healthy.
        -->
        <dd v-if="repoRuns.length > 0">
          <span class="repo-runs">
            <span
              v-for="entry in repoRuns"
              :key="entry.repo.id"
              class="repo-run"
            >
              <RouterLink
                class="repo-link"
                :to="`/repos/${entry.repo.id}`"
                >{{ entry.repo.name }}</RouterLink
              >
              <span
                v-if="!entry.repo.required"
                class="badge badge--neutral"
                title="A failure here is reported as a warning and never stops the run"
              >
                best effort
              </span>
              <span
                class="badge"
                :class="repoStatusBadgeClass(entry)"
              >
                <span class="badge-dot" />
                {{ repoStatusLabel(entry) }}
              </span>
              <span
                class="repo-run-note"
                :title="entry.last?.error_message ?? undefined"
                >{{ repoRunNote(entry) }}</span
              >
            </span>
          </span>
        </dd>
        <dd v-else>
          {{
            repoName ??
            (schedule.repo_id != null ? `#${schedule.repo_id}` : 'No repository assigned')
          }}
        </dd>
        <dt>On failure</dt>
        <dd>{{ schedule.on_failure === 'continue' ? 'Continue' : 'Stop' }}</dd>
        <dt>Next run</dt>
        <dd>{{ formatDateShort(schedule.next_run_at) }}</dd>
        <dt>Last run</dt>
        <dd>{{ formatDateShort(schedule.last_run_at, 'Never') }}</dd>
        <dt>Cron (human)</dt>
        <dd>{{ cronSummary }}</dd>
        <dt>Catch-up</dt>
        <dd>
          <span :class="{ muted: !schedule.catch_up_missed_runs }">{{ catchUpText }}</span>
          <span
            v-for="id in pendingCatchUps"
            :key="id"
            class="badge badge--info"
          >
            <span class="badge-dot" />
            Pending for {{ agentLabel(id) }}
          </span>
        </dd>
      </dl>
    </div>

    <div class="tiles">
      <div class="tile">
        <span class="stat-label">Targets</span>
        <span class="stat-value stat-value--lg">{{ agentIds.length }}</span>
        <span
          v-if="multiRepo"
          class="stat-sub"
        >
          into {{ repoOptions.length }} repositories
        </span>
        <span
          v-if="overdueTargets.length > 0"
          class="stat-sub stat-sub--bad"
        >
          {{ overdueTargets.length }} overdue
        </span>
      </div>
      <div class="tile">
        <span class="stat-label">Recent runs</span>
        <!--
          One strip per repository: a merged strip cannot say whether a run of
          failures is one repository having a bad week or every copy of the
          backup going down at once, and those call for different responses.
        -->
        <template v-if="multiRepo">
          <div
            v-for="entry in repoRuns"
            :key="entry.repo.id"
            class="repo-strip"
          >
            <span class="group-label">{{ entry.repo.name }}</span>
            <AgentRunStrip :reports="entry.reports" />
          </div>
        </template>
        <AgentRunStrip
          v-else
          :reports="reports"
        />
      </div>
    </div>

    <section v-if="agentIds.length > 0">
      <div class="section-head">
        <h2 class="section-title">Targets</h2>
      </div>
      <div class="rows">
        <div
          v-for="(id, idx) in agentIds"
          :key="id"
          class="agent-row"
        >
          <i
            class="agent-row-stripe"
            :class="`agent-row-stripe--${stripeFor(id)}`"
            aria-hidden="true"
          />
          <span class="agent-row-order">{{ idx + 1 }}</span>
          <span class="agent-row-name mono">{{ agentLabel(id) }}</span>
          <span
            v-if="healthForAgent(id)?.is_overdue"
            class="badge badge--warning"
          >
            Overdue
          </span>
          <span
            v-if="catchUpPendingFor(id)"
            class="badge badge--info"
            :title="`Missed the run due at ${formatDateShort(catchUpPendingFor(id))}`"
          >
            <span class="badge-dot" />
            Catch-up pending
          </span>
          <!--
            A target writes one copy per repository, so "last 3d ago" beside a
            red stripe is ambiguous on its own: this says how much of that
            host's fan-out is actually broken.
          -->
          <span
            v-if="multiRepo && failingRepos(id) > 0"
            class="badge badge--danger"
          >
            {{ failingRepos(id) }} of {{ repoOptions.length }} repos failing
          </span>
          <span class="agent-row-stats">
            <span>last {{ lastBackupText(id) }}</span>
          </span>
          <div class="agent-row-actions">
            <button
              v-if="healthForAgent(id)?.is_overdue"
              class="btn btn-sm btn-ghost"
              :disabled="retryingAgentId === id"
              @click="emit('retry', id)"
            >
              {{ retryingAgentId === id ? '...' : 'Retry' }}
            </button>
          </div>
        </div>
      </div>
    </section>

    <section v-if="backupPreview.length > 0">
      <div class="section-head">
        <h2 class="section-title">Recent backups</h2>
        <button
          class="section-link"
          type="button"
          @click="emit('openLogs')"
        >
          View all {{ settledReports.length }}
        </button>
      </div>
      <div class="rows">
        <div
          v-for="r in backupPreview"
          :key="r.id"
          class="agent-row"
        >
          <i
            class="agent-row-stripe"
            :class="`agent-row-stripe--${reportStripe(r)}`"
            aria-hidden="true"
          />
          <span class="agent-row-when">{{ relativeTime(r.finished_at) }}</span>
          <button
            v-if="hasArchive(r)"
            class="agent-row-name mono"
            type="button"
            title="Browse this archive"
            @click="emit('openArchive', r)"
          >
            {{ hostLabel(r.agent_id) }}
          </button>
          <span
            v-else
            class="agent-row-name mono"
            >{{ hostLabel(r.agent_id) }}</span
          >
          <!--
            Which copy this run wrote. Without it two rows a multi-repository
            schedule produces in the same minute - one green, one red, same
            host - are indistinguishable.
          -->
          <span
            v-if="multiRepo"
            class="meta-pill"
            >{{ repoLabel(r) }}</span
          >
          <span
            v-if="normalizeBackupStatus(r.status) !== 'success'"
            class="badge"
            :class="backupStatusBadgeClass(r.status)"
          >
            {{ normalizeBackupStatus(r.status) }}
          </span>
          <span class="agent-row-stats">
            <span>{{ formatBytes(r.original_size) }}</span>
            <span>{{ formatDuration(r.duration_secs) }}</span>
          </span>
          <!--
            A run that broke is the reason someone opens this page, and the
            row itself has no room for the output - so it points at the host
            row that renders it in full. Same verdict, from the same shared
            helper, as the host Overview's own rows reach for that run.
          -->
          <button
            v-if="reportMessageLabel(r)"
            class="btn btn-sm btn-ghost"
            type="button"
            title="Open this run on the host's Backups tab"
            @click="emit('openReportDetail', r)"
          >
            {{ reportMessageLabel(r) }}
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* Base .overview-tab / .attention / .tiles / .tile / .section-* shapes live
   in style.css, shared with AgentOverviewTab. Only the attention row's Retry
   button and note, the warning stat modifier, and the target-order badge are
   this page's own. */
.attention-note {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.attention-row .btn {
  margin-left: auto;
}

.stat-sub--bad {
  color: var(--warning);
}

/* Each repository's status on its own line: the badges wrap into a second
   row on a phone rather than pushing the name out of the value column. */
.repo-runs {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.repo-run {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.repo-run-note {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.repo-strip {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.agent-row-order {
  font-family: var(--mono);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  background: var(--bg-hover);
  border-radius: var(--radius-pill);
  width: 1.35rem;
  height: 1.35rem;
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
</style>

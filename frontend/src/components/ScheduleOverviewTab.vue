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
import type { DependencyWaitResponse, ScheduleDependencyResponse } from '../types/generated'
import { computed } from 'vue'
import { formatBytes, formatDateShort, relativeTime } from '../utils/format'
import { scheduleRepoRuns, type ScheduleRepoRuns } from '../utils/scheduleRepos'
import { backupStatusBadgeClass, badgeClass } from '../utils/badge'
import { humanizeMinutes } from '../utils/duration'
import { reachabilityBadge } from '../utils/dependencyHost'
import { dependencyWaitNote, summarizeScheduleDependencies } from '../utils/scheduleDependencies'
import type { ScheduleRepoOption } from '../types/schedule'
import BackupProgressCard from './BackupProgressCard.vue'
import ScheduleRunsPanel from './ScheduleRunsPanel.vue'

interface ArchiveProgressData {
  hostname: string
  nfiles: number
  originalSize: number
  currentPath: string
}

/**
 * Replaces the old "Schedule Info" card, which was read-only status wedged
 * between a wall of editable settings cards. This is a dedicated status
 * screen instead: whether anything is overdue or waiting, the last/next run,
 * and the recent runs - each one's hosts, results and output a click away.
 * Every editable field moved out to Settings.
 */
const props = defineProps<{
  schedule: ScheduleRow
  /** Targets of this schedule, for the catch-up markers they carry. */
  targets: readonly ScheduleTargetResponse[]
  repoName: string | null
  /** Every repository this schedule writes into, in write order. */
  repoOptions: readonly ScheduleRepoOption[]
  /** Repositories waiting to be caught up once their host answers again. */
  pendingRepoCatchUps?: readonly number[]
  cronSummary: string
  agentIds: readonly number[]
  agentLabel: (id: number) => string
  healthForAgent: (id: number) => HealthSummaryResponse | null
  connectivityNote: (id: number) => string
  retryingAgentId: number | null
  reports: readonly ReportRow[]
  agents: ReadonlyMap<number, AgentRow>
  backupRunning: boolean
  /** The run is held for a host that is offline, not underway. */
  backupQueued: boolean
  backupHostname: string | null
  backupArchiveName: string | null
  backupElapsedSecs: number
  estimatedRemainingSecs: number | null
  archiveProgress: ArchiveProgressData | null
  /** Every dependency this schedule's agents need, per agent. */
  dependencies?: readonly ScheduleDependencyResponse[]
  /** Runs skipped because a dependency did not answer, waiting for it to come back. */
  dependencyWaits?: readonly DependencyWaitResponse[]
  /** Whether the viewer may ask a dependency whether it is back. */
  canCheckDependencies?: boolean
  /** The dependency a Check now is in flight for. */
  checkingDependencyId?: number | null
}>()

const emit = defineEmits<{
  retry: [agentId: number]
  openLogs: []
  openReportDetail: [report: ReportRow]
  checkDependency: [dependencyHostId: number]
}>()

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

/**
 * When this repository last finished a run, and how much it took.
 *
 * Empty when it never has: the badge beside it already says "never run", and
 * a note repeating that in different words read as a second, separate claim.
 * "never run" is also what every other screen calls this state (see
 * `AgentScheduleRow`), so the badge is the one that should say it.
 */
function repoRunNote(entry: ScheduleRepoRuns): string {
  const last = entry.last
  if (!last) return ''
  const when = relativeTime(last.finished_at)
  return last.original_size > 0 ? `${when} · ${formatBytes(last.original_size)}` : when
}

/** The occurrence this target missed and will run when its host reconnects. */
function catchUpPendingFor(agentId: number): string | null {
  return props.targets.find((t) => t.agent_id === agentId)?.catch_up_pending_for ?? null
}

const pendingCatchUps = computed(() =>
  props.agentIds.filter((id) => catchUpPendingFor(id) !== null),
)

/** The same, for a repository whose host was the one away - named, not numbered. */
const pendingRepoLabels = computed(() =>
  (props.pendingRepoCatchUps ?? []).map(
    (id) => props.repoOptions.find((o) => o.id === id)?.name ?? `#${id}`,
  ),
)

/**
 * The floor is the schedule's only catch-up setting: whether a host is waited
 * for at all is set on that host, so there is no schedule-wide "off" to show.
 */
const catchUpText = computed(
  () =>
    `Only if the next run is at least ${humanizeMinutes(props.schedule.catch_up_min_lead_minutes)} away`,
)

const overdueTargets = computed(() =>
  props.agentIds.filter((id) => props.healthForAgent(id)?.is_overdue),
)

function lastBackupText(id: number): string {
  const at = props.healthForAgent(id)?.last_backup_at
  return at ? relativeTime(at) : 'never'
}

const waits = computed(() =>
  (props.dependencyWaits ?? []).filter((w) => w.schedule_id === props.schedule.id),
)

/** Every host of this schedule waiting on one dependency, as one attention row. */
interface DependencyWaitGroup {
  dependencyId: number
  dependencyName: string
  hosts: string
  verb: 'was' | 'were'
  /** The earliest skipped occurrence among the hosts. */
  skippedAt: string
  /** The wait whose clocks the row's note reads - they share one dependency's. */
  first: DependencyWaitResponse
  catchingUp: boolean
}

const waitGroups = computed((): DependencyWaitGroup[] => {
  const groups = new Map<number, DependencyWaitResponse[]>()
  for (const wait of waits.value) {
    const group = groups.get(wait.dependency_host_id)
    if (group) group.push(wait)
    else groups.set(wait.dependency_host_id, [wait])
  }
  return [...groups.values()].map((group) => {
    const sorted = [...group].sort((a, b) => a.pending_for.localeCompare(b.pending_for))
    const first = sorted[0]
    const labels = sorted.map((w) => props.agentLabel(w.agent_id))
    const hosts =
      labels.length <= 1
        ? (labels[0] ?? first.hostname)
        : `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`
    return {
      dependencyId: first.dependency_host_id,
      dependencyName: first.dependency_name,
      hosts,
      verb: labels.length > 1 ? 'were' : 'was',
      skippedAt: first.pending_for,
      first,
      catchingUp: group.every((w) => w.catching_up),
    }
  })
})

const dependencySummaries = computed(() =>
  summarizeScheduleDependencies(props.dependencies ?? [], props.agentLabel),
)
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
      :waiting-for="backupQueued ? backupHostname : null"
    />

    <div
      v-if="overdueTargets.length > 0 || waits.length > 0"
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
      <!--
        A run skipped for a dependency is not overdue - it is waiting, and
        will run by itself once the machine answers. What the reader needs is
        which machine, and when it is next asked.
      -->
      <div
        v-for="group in waitGroups"
        :key="group.dependencyId"
        class="attention-row"
      >
        <span class="badge badge--warning">Waiting</span>
        <span class="attention-message">
          {{ group.hosts }} {{ group.verb }} skipped at {{ formatDateShort(group.skippedAt) }}:
          dependency
          <RouterLink
            class="repo-link"
            :to="`/dependency-hosts/${group.dependencyId}`"
            >{{ group.dependencyName }}</RouterLink
          >
          did not answer.
        </span>
        <span class="attention-note">{{ dependencyWaitNote(group.first) }}</span>
        <button
          v-if="canCheckDependencies && !group.catchingUp"
          class="btn btn-sm btn-ghost"
          type="button"
          :disabled="checkingDependencyId === group.dependencyId"
          @click="emit('checkDependency', group.dependencyId)"
        >
          {{ checkingDependencyId === group.dependencyId ? 'Checking...' : 'Check now' }}
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

          `multiRepo`, not `repoRuns.length > 0`: the latter is one entry per
          target and so true of every schedule with a repository at all, which
          put the status treatment on the single-repo page this change is
          supposed to leave alone. A single target's outcome is the schedule's
          own, already on the strip and the rows below.
        -->
        <dd v-if="multiRepo">
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
                v-if="entry.last"
                class="repo-run-note"
                :title="entry.last.error_message ?? undefined"
                >{{ repoRunNote(entry) }}</span
              >
            </span>
          </span>
        </dd>
        <!-- The single-target page, unchanged: the name, as it always was. -->
        <dd v-else>
          {{
            repoOptions[0]?.name ??
            repoName ??
            (schedule.repo_id != null ? `#${schedule.repo_id}` : 'No repository assigned')
          }}
        </dd>
        <!--
          Which machines this schedule runs on, in order. The Targets rows that
          used to list them are gone; what a host's last run did is in the run
          detail under Recent runs.
        -->
        <dt>Hosts</dt>
        <dd v-if="agentIds.length > 0">{{ agentIds.map((id) => agentLabel(id)).join(', ') }}</dd>
        <dd
          v-else
          class="muted"
        >
          None
        </dd>
        <dt>On failure</dt>
        <dd>{{ schedule.on_failure === 'continue' ? 'Continue' : 'Stop' }}</dd>
        <dt>Next run</dt>
        <dd>{{ formatDateShort(schedule.next_run_at) }}</dd>
        <dt>Last run</dt>
        <dd>{{ formatDateShort(schedule.last_run_at, 'Never') }}</dd>
        <dt>Cron (human)</dt>
        <dd>{{ cronSummary }}</dd>
        <template v-if="dependencySummaries.length > 0">
          <dt>Dependencies</dt>
          <dd>
            <span class="dependency-list">
              <span
                v-for="dep in dependencySummaries"
                :key="dep.id"
                class="dependency-item"
              >
                <RouterLink
                  class="repo-link"
                  :to="`/dependency-hosts/${dep.id}`"
                  >{{ dep.name }}</RouterLink
                >
                <span
                  class="badge"
                  :class="badgeClass(reachabilityBadge(dep.lastCheckReachable).tone)"
                >
                  <span class="badge-dot" />
                  {{ reachabilityBadge(dep.lastCheckReachable).label }}
                </span>
                <span class="dependency-note">{{ dep.appliesTo }}</span>
              </span>
            </span>
          </dd>
        </template>
        <dt>Catch-up</dt>
        <dd>
          <span>{{ catchUpText }}</span>
          <span
            v-for="id in pendingCatchUps"
            :key="id"
            class="badge badge--info"
          >
            <span class="badge-dot" />
            Pending for {{ agentLabel(id) }}
          </span>
          <span
            v-for="name in pendingRepoLabels"
            :key="`repo-${name}`"
            class="badge badge--info"
          >
            <span class="badge-dot" />
            Pending for {{ name }}
          </span>
        </dd>
      </dl>
    </div>

    <ScheduleRunsPanel
      :reports="reports"
      :repo-options="repoOptions"
      :agents="agents"
      @open-report-detail="emit('openReportDetail', $event)"
      @open-logs="emit('openLogs')"
    />
  </div>
</template>

<style scoped>
/* Base .overview-tab / .attention shapes live in style.css, shared with
   AgentOverviewTab. Only the attention row's trailing button and note are
   this page's own; the runs strip is ScheduleRunsPanel's. */
.attention-note {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.attention-row .btn {
  margin-left: auto;
}

/* Each repository's status on its own line: the badges wrap into a second
   row on a phone rather than pushing the name out of the value column. */
.repo-runs,
.dependency-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.repo-run,
.dependency-item {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.repo-run-note,
.dependency-note {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}
</style>

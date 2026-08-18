<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, formatDateShort, formatDuration, relativeTime } from '../utils/format'
import { normalizeBackupStatus, filterSettledReports } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import BackupProgressCard from './BackupProgressCard.vue'
import AgentRunStrip from './AgentRunStrip.vue'
import type { ScheduleRow } from '../types/schedule'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'
import type { HealthSummaryResponse } from '../types/generated/HealthSummaryResponse'

interface ArchiveProgressData {
  hostname: string
  nfiles: number
  originalSize: number
  currentPath: string
}

/**
 * The landing tab for a schedule, which is what you see when a target missed
 * its run. It answers the questions the old "Schedule Info" card buried under
 * a wall of settings cards - is anything overdue, when did it last run, which
 * targets are healthy - and previews the backups the Backups tab holds in
 * full. Everything editable now lives under Settings.
 */
const props = defineProps<{
  schedule: ScheduleRow
  repoName: string | null
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
  openBackups: []
}>()

const BACKUP_PREVIEW_COUNT = 5

const overdueTargets = computed(() =>
  props.agentIds.filter((id) => props.healthForAgent(id)?.is_overdue),
)

function lastBackupText(id: number): string {
  const at = props.healthForAgent(id)?.last_backup_at
  return at ? relativeTime(at) : 'never'
}

function stripeFor(id: number): 'danger' | 'warning' | 'accent' | 'success' {
  const health = props.healthForAgent(id)
  if (health && normalizeBackupStatus(health.last_status ?? '') === 'failed') return 'danger'
  if (health?.is_overdue) return 'warning'
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

function reportStripe(r: ReportRow): 'danger' | 'warning' | 'success' {
  const status = normalizeBackupStatus(r.status)
  if (status === 'failed') return 'danger'
  if (status === 'warning') return 'warning'
  return 'success'
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

    <div class="info-card">
      <h3 class="info-title">Schedule Info</h3>
      <dl class="info-grid">
        <dt>Repository</dt>
        <dd>
          {{
            repoName ??
            (schedule.repo_id != null ? `#${schedule.repo_id}` : 'No repository assigned')
          }}
        </dd>
        <dt>On Failure</dt>
        <dd>{{ schedule.on_failure === 'continue' ? 'Continue' : 'Stop' }}</dd>
        <dt>Next Run</dt>
        <dd>{{ formatDateShort(schedule.next_run_at) }}</dd>
        <dt>Last Run</dt>
        <dd>{{ formatDateShort(schedule.last_run_at, 'Never') }}</dd>
        <dt>Cron (human)</dt>
        <dd>{{ cronSummary }}</dd>
      </dl>
    </div>

    <div class="tiles">
      <div class="tile">
        <span class="stat-label">Targets</span>
        <span class="stat-value stat-value--lg">{{ agentIds.length }}</span>
        <span
          v-if="overdueTargets.length > 0"
          class="stat-sub stat-sub--bad"
        >
          {{ overdueTargets.length }} overdue
        </span>
      </div>
      <div class="tile">
        <span class="stat-label">Recent runs</span>
        <AgentRunStrip :reports="reports" />
      </div>
    </div>

    <section v-if="agentIds.length > 0">
      <h2 class="section-title">Targets</h2>
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
          @click="emit('openBackups')"
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
          <span class="agent-row-name mono">{{ hostLabel(r.agent_id) }}</span>
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
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.overview-tab {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.attention {
  border: 1px solid var(--warning);
  background: var(--warning-subtle);
  border-radius: var(--radius);
  padding: 0.6rem 0.8rem;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.attention-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  font-size: var(--fs-sm);
  flex-wrap: wrap;
}

.attention-message {
  color: var(--text-primary);
}

.attention-note {
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.attention-row .btn {
  margin-left: auto;
}

.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(180px, 100%), 1fr));
  gap: 0.6rem;
}

.tile {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 0.7rem 0.8rem;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.stat-sub--bad {
  color: var(--warning);
}

.section-title {
  font-size: var(--fs-2xs);
  font-weight: 700;
  letter-spacing: 0.09em;
  text-transform: uppercase;
  color: var(--text-muted);
  font-family: var(--mono);
  margin: 0 0 0.5rem;
}

.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  margin-bottom: 0.5rem;
}

.section-head .section-title {
  margin: 0;
}

.section-link {
  font: inherit;
  font-size: var(--fs-xs);
  background: none;
  border: none;
  padding: 0;
  color: var(--accent);
  cursor: pointer;
}

.section-link:hover {
  text-decoration: underline;
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

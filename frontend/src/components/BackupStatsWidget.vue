<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { CheckCheck } from '@lucide/vue'
import { formatDuration } from '../utils/format'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { logger } from '../utils/logger'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import {
  acknowledgeAllActivity,
  getOutstandingAcknowledgements,
  type AcknowledgeScope,
} from '../api/stats'
import type { Repo } from '../types/repo'
import { type SegmentedOption } from './BaseSegmented.vue'
import BaseModal from './BaseModal.vue'
import ChartRangeControls from './ChartRangeControls.vue'
import { useRangeFilteredFetch } from '../composables/useRangeFilteredFetch'

const rangeOptions: SegmentedOption<number>[] = [
  { value: 7, label: '7d' },
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
  { value: 90, label: '90d' },
]

interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
  acknowledged: boolean
}

const props = defineProps<{ repos: Repo[] }>()
const router = useRouter()
const { error: toastError, success: toastSuccess } = useToast()

const selectedDays = ref<number>(30)
const selectedRepoId = ref<number | undefined>(undefined)
const { entries, loading, refetch } = useRangeFilteredFetch<ActivityEntry>(
  '/stats/activity',
  selectedDays,
  selectedRepoId,
)

const totalCount = computed((): number => entries.value.length)
const successCount = computed(
  (): number => entries.value.filter((e) => normalizeBackupStatus(e.status) === 'success').length,
)
/** A run somebody can review: exactly what the reset clears. */
function isReviewable(entry: ActivityEntry): boolean {
  const status = normalizeBackupStatus(entry.status)
  return status === 'failed' || status === 'warning'
}

const failedEntries = computed((): ActivityEntry[] =>
  entries.value.filter((e) => normalizeBackupStatus(e.status) === 'failed'),
)
// Only the failures nobody has reviewed yet. A reviewed failure stays in the
// history the success rate is computed from - it just stops being something
// this tile is still asking the operator to look at.
const failedCount = computed(
  (): number => failedEntries.value.filter((e) => !e.acknowledged).length,
)
// Warned runs are acknowledgeable too, so a reset clears them - and without
// this the button could appear over a tile reading zero, with nothing on the
// panel accounting for what it was about to mark reviewed.
const warnedCount = computed(
  (): number =>
    entries.value.filter((e) => normalizeBackupStatus(e.status) === 'warning' && !e.acknowledged)
      .length,
)
// Everything the reset has already retired, warnings included - counting only
// reviewed failures would let a reviewed warning leave the "warned" side
// without arriving on the "reviewed" one, reading as a run that vanished
// rather than one that was looked at.
const reviewedCount = computed(
  (): number => entries.value.filter((e) => e.acknowledged && isReviewable(e)).length,
)

/** What the sub-line under the Failed tile has to say, if anything. */
const failedSubLine = computed((): string => {
  const parts: string[] = []
  if (warnedCount.value > 0) parts.push(`${warnedCount.value} warned`)
  if (reviewedCount.value > 0) parts.push(`${reviewedCount.value} reviewed`)
  return parts.join(' \u00b7 ')
})
const successRate = computed((): number => {
  if (totalCount.value === 0) return 0
  return Math.round((successCount.value / totalCount.value) * 100)
})
const avgDurationSecs = computed((): number => {
  if (entries.value.length === 0) return 0
  const total = entries.value.reduce((sum, e) => sum + e.duration_secs, 0)
  return Math.round(total / entries.value.length)
})

/** The repo and range the panel is showing, as the API spells them. */
const scope = computed(
  (): AcknowledgeScope => ({ days: selectedDays.value, repo_id: selectedRepoId.value }),
)

// What a reset would actually clear, counted server-side over the same repo
// and window. The feed this panel renders is only what the caller may see and
// stops at the window's edge, so counting its rows would offer the button to
// someone whose permissions leave nothing to acknowledge - and would promise a
// number the write then does not match.
const outstanding = ref(0)
const resetting = ref(false)
const showResetDialog = ref(false)

async function fetchOutstanding(): Promise<void> {
  const counts = await getOutstandingAcknowledgements(scope.value)
  outstanding.value = counts.backup_reports
}

const canReset = computed((): boolean => outstanding.value > 0)

async function confirmReset(): Promise<void> {
  resetting.value = true
  try {
    const result = await acknowledgeAllActivity(scope.value)
    toastSuccess(
      result.backup_reports === 1
        ? 'Marked 1 run as reviewed'
        : `Marked ${result.backup_reports} runs as reviewed`,
    )
    showResetDialog.value = false
  } catch (e: unknown) {
    toastError(extractError(e))
    return
  } finally {
    resetting.value = false
  }

  // Deliberately outside the try: by here the runs *are* acknowledged and the
  // operator has been told so. A failed re-read leaves the panel stale, not the
  // reset undone, and reporting it as an error right after the success toast
  // would say the opposite of what happened.
  await Promise.all([refetch(), fetchOutstanding()]).catch((e: unknown) =>
    logger.error('refresh after acknowledge failed', e),
  )
}

onMounted(() => {
  fetchOutstanding().catch((e: unknown) => logger.error('fetchOutstanding failed', e))
})

watch([selectedDays, selectedRepoId], () => {
  fetchOutstanding().catch((e: unknown) => logger.error('fetchOutstanding failed', e))
})

function navigateToActivity(status?: string): void {
  const query: Record<string, string> = { days: String(selectedDays.value) }
  if (status) {
    query.status = status
  }
  router.push({ name: 'activity', query })
}
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Backup stats</h2>
      <ChartRangeControls
        v-model:repo-id="selectedRepoId"
        v-model:days="selectedDays"
        :repos="props.repos"
        :options="rangeOptions"
        label="Backup statistics range"
      />
    </div>
    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading...
    </div>
    <div
      v-else
      class="stats-grid"
    >
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity()"
      >
        <span class="stat-value stat-value--lg">{{ totalCount }}</span>
        <span class="stat-label">Total</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('success')"
      >
        <span
          class="stat-value stat-value--lg"
          :class="{
            'color-success': successRate >= 90,
            'color-warning': successRate >= 70 && successRate < 90,
            'color-danger': successRate < 70,
          }"
        >
          {{ successRate }}%
        </span>
        <span class="stat-label">Success</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('failed')"
      >
        <span
          class="stat-value stat-value--lg"
          :class="{ 'color-danger': failedCount > 0 }"
        >
          {{ failedCount }}
        </span>
        <span class="stat-label">Failed</span>
        <span
          v-if="failedSubLine"
          class="stat-sub"
        >
          {{ failedSubLine }}
        </span>
      </div>
      <div class="mini-stat">
        <span class="stat-value stat-value--lg">{{ formatDuration(avgDurationSecs) }}</span>
        <span class="stat-label">Avg duration</span>
      </div>
    </div>
    <!-- Below the tiles rather than in the header: the repo and range controls
         already fill that row, and a third control there wraps the heading off
         its own line. -->
    <div
      v-if="!loading && canReset"
      class="stats-actions"
    >
      <button
        class="btn btn-sm btn-ghost"
        title="Mark the failed and warned runs in this range as reviewed"
        @click="showResetDialog = true"
      >
        <CheckCheck :size="14" />
        Mark reviewed
      </button>
    </div>

    <BaseModal
      :open="showResetDialog"
      title="Mark runs as reviewed"
      @close="showResetDialog = false"
    >
      <p>
        Mark the <strong>{{ outstanding }}</strong>
        {{ outstanding === 1 ? 'failed or warned run' : 'failed and warned runs' }} in this range as
        reviewed? The failed count drops to zero and the runs stay in the Activity Log — nothing is
        deleted, and any one of them can be un-reviewed there.
      </p>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showResetDialog = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="resetting"
          @click="confirmReset"
        >
          {{ resetting ? 'Marking...' : 'Mark reviewed' }}
        </button>
      </template>
    </BaseModal>
  </section>
</template>

<style scoped>
.stats-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--space-5);
}

.stats-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-5);
}

.mini-stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-4);
  background: var(--bg-base);
  border-radius: var(--radius-sm);
}

.mini-stat-link {
  cursor: pointer;
  transition:
    background var(--duration-base),
    border-color var(--duration-base);
}

.mini-stat-link:hover {
  background: var(--bg-hover);
}

.color-success {
  color: var(--success);
}

.color-warning {
  color: var(--warning);
}

.color-danger {
  color: var(--danger);
}
</style>

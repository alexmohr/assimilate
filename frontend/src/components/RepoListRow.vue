<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { ChevronRight } from '@lucide/vue'
import { formatBytes, relativeTime } from '../utils/format'
import type { BadgeTone } from '../utils/badge'
import { repoQuotaUse, repoRowStatus, type RepoRowStatus } from '../utils/repoStatus'
import type { RepoWithStats } from '../types/repo'

/**
 * One repository on the Repositories overview: a link to its page. A status
 * dot, the name over a line of facts, and its size (against its own quota,
 * when it has one) on the right. The dot is the status; a status other than
 * healthy is also named first in the facts, in its tone when it is a risk
 * and muted when it is not. On a phone the size drops under the facts.
 */
const props = withDefaults(
  defineProps<{
    repo: RepoWithStats
    /** Steps the row back, for a host-group entry the current filter hides. */
    dim?: boolean
  }>(),
  { dim: false },
)

const status = computed<RepoRowStatus>(() => repoRowStatus(props.repo))
const quotaUse = computed(() => repoQuotaUse(props.repo))

/** Unmatched archives are resolved on the Archives tab, so that is where the row leads. */
const target = computed<string>(() =>
  status.value.kind === 'unmatched'
    ? `/repos/${props.repo.id}?tab=archives`
    : `/repos/${props.repo.id}`,
)

const lastBackup = computed<string>(() =>
  props.repo.last_backup_at
    ? `last backup ${relativeTime(props.repo.last_backup_at)}`
    : 'no backup yet',
)

const sizeText = computed<string>(() => {
  const used = formatBytes(props.repo.total_deduplicated_size)
  return quotaUse.value ? `${used} of ${formatBytes(quotaUse.value.ceilingBytes)}` : used
})

const pendingDeletionsText = computed<string>(() => {
  const count = props.repo.append_only?.pending_deletion_count ?? 0
  return `${count} pending deletion${count === 1 ? '' : 's'}`
})

const archivesText = computed<string>(
  () => `${props.repo.archive_count} archive${props.repo.archive_count === 1 ? '' : 's'}`,
)

const DOT_CLASS: Record<BadgeTone, string | null> = {
  success: 'status-dot--success',
  warning: 'status-dot--warning',
  danger: 'status-dot--danger',
  info: 'status-dot--info',
  accent: 'status-dot--accent',
  neutral: null,
  violet: null,
}

/** The status named in the facts takes a colour only when it is a risk. */
const STATUS_CLASS: Record<BadgeTone, string | null> = {
  success: null,
  warning: 'repo-row-status--warning',
  danger: 'repo-row-status--danger',
  info: null,
  accent: null,
  neutral: null,
  violet: null,
}

const BAR_CLASS: Record<BadgeTone, string | null> = {
  success: null,
  warning: 'progress-bar--warning',
  danger: 'progress-bar--danger',
  info: null,
  accent: null,
  neutral: 'progress-bar--muted',
  violet: null,
}

/** The bar takes the quota's tone, not the row's: an unmatched archive says nothing about space. */
const barTone = computed<BadgeTone>(() => {
  switch (status.value.kind) {
    case 'over_quota':
      return 'danger'
    case 'near_quota':
      return 'warning'
    default:
      return props.repo.enabled ? 'success' : 'neutral'
  }
})
</script>

<template>
  <RouterLink
    :to="target"
    class="list-row repo-row"
    :class="{ 'repo-row--dim': dim }"
  >
    <span class="list-row-main">
      <span
        class="status-dot"
        :class="DOT_CLASS[status.tone]"
      ></span>
      <span class="list-row-text">
        <span class="list-row-name">{{ repo.name }}</span>
        <span class="list-row-sub repo-row-facts">
          <span
            v-if="status.kind !== 'healthy'"
            :class="STATUS_CLASS[status.tone]"
            :title="status.detail ?? undefined"
            >{{ status.label }}</span
          >
          <span
            v-if="repo.append_only"
            title="Agents can add archives but not delete them"
            >append-only</span
          >
          <span
            v-if="repo.append_only?.maintenance_stopped"
            class="repo-row-status--warning"
            title="Archives vanished that Assimilate did not delete"
            >maintenance stopped</span
          >
          <span
            v-if="(repo.append_only?.pending_deletion_count ?? 0) > 0"
            title="Archives queued for deletion at the next maintenance run"
            >{{ pendingDeletionsText }}</span
          >
          <span>{{ archivesText }}</span>
          <span>{{ lastBackup }}</span>
        </span>
      </span>
    </span>
    <div class="repo-row-quota">
      <div
        v-if="quotaUse"
        class="progress-track"
      >
        <div
          class="progress-bar"
          :class="BAR_CLASS[barTone]"
          :style="{ width: `${quotaUse.fillPercent}%` }"
        ></div>
      </div>
      <span class="repo-row-size">{{ sizeText }}</span>
    </div>
    <ChevronRight
      :size="16"
      class="list-row-chevron"
    />
  </RouterLink>
</template>

<style scoped>
.repo-row--dim {
  opacity: 0.45;
}

.repo-row-facts {
  display: flex;
  flex-wrap: wrap;
  column-gap: var(--space-2);
  white-space: normal;
}

.repo-row-facts > span + span::before {
  content: '\00B7';
  margin-right: var(--space-2);
  color: var(--text-muted);
}

.repo-row-status--warning {
  color: var(--warning);
}

.repo-row-status--danger {
  color: var(--danger);
}

/* The bar over its figure, both at the right; a grid rather than a column
   flex, because the shared track grows along a flex axis and would collapse
   to nothing. */
.repo-row-quota {
  display: grid;
  gap: var(--space-2);
  justify-items: end;
  min-width: 0;
}

.repo-row-quota .progress-track {
  width: 100%;
}

.repo-row-size {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
}

@media (max-width: 640px) {
  /* Under the facts, at the start of the line like them. */
  .repo-row-quota {
    flex: 1 1 100%;
    justify-items: start;
  }

  .repo-row-quota .progress-track {
    max-width: 14rem;
  }
}
</style>

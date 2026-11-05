<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { ChevronRight } from '@lucide/vue'
import { formatBytes, relativeTime } from '../utils/format'
import { badgeClass, type BadgeTone } from '../utils/badge'
import RepoAppendOnlyBadges from './RepoAppendOnlyBadges.vue'
import { repoQuotaUse, repoRowStatus, type RepoRowStatus } from '../utils/repoStatus'
import type { RepoWithStats } from '../types/repo'

/**
 * One repository on the Repositories overview: a link to its page, with the
 * columns the list's head names (status, archives, last backup, size of
 * quota). On a phone the cells stack into a name line, a counts line and a
 * coloured status line.
 */
const props = withDefaults(
  defineProps<{
    repo: RepoWithStats
    /** Steps the row back, for a host-group entry the current filter hides. */
    dim?: boolean
    /** The row sits under its host's header, which already says whether the host sleeps. */
    pooled?: boolean
  }>(),
  { dim: false, pooled: false },
)

const status = computed<RepoRowStatus>(() => repoRowStatus(props.repo))
const quotaUse = computed(() => repoQuotaUse(props.repo))

/** Unmatched archives are resolved on the Archives tab, so that is where the row leads. */
const target = computed<string>(() =>
  status.value.kind === 'unmatched'
    ? `/repos/${props.repo.id}?tab=archives`
    : `/repos/${props.repo.id}`,
)

/** The borg address and settings, plus a note on a sleeping host the row's header does not carry. */
const subline = computed<string>(() => {
  const { repo } = props
  const parts = [
    `${repo.ssh_user}@${repo.ssh_host}:${repo.repo_path}`,
    repo.encryption,
    repo.compression,
  ]
  if (!props.pooled && repo.repo_host.intermittent) parts.push('host sleeps')
  return parts.join(' · ')
})

const lastBackup = computed<string>(() => relativeTime(props.repo.last_backup_at ?? ''))

const sizeText = computed<string>(() => {
  const used = formatBytes(props.repo.total_deduplicated_size)
  return quotaUse.value ? `${used} of ${formatBytes(quotaUse.value.ceilingBytes)}` : used
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
        <span class="list-row-sub">{{ subline }}</span>
      </span>
      <span class="list-narrow-only repo-row-when">{{ lastBackup }}</span>
    </span>
    <span class="list-row-chips list-wide-only">
      <RepoAppendOnlyBadges :append-only="repo.append_only" />
      <span
        v-if="(repo.append_only?.pending_deletion_count ?? 0) > 0"
        class="badge badge--violet"
        title="Archives queued for deletion at the next maintenance run"
      >
        {{ repo.append_only?.pending_deletion_count }} pending deletion
      </span>
      <span
        class="badge"
        :class="[badgeClass(status.tone), { 'badge--pulse': status.kind === 'importing' }]"
        :title="status.detail ?? undefined"
      >
        {{ status.label }}
      </span>
    </span>
    <span class="list-row-value list-wide-only">{{ repo.archive_count }}</span>
    <span class="list-row-value list-wide-only">{{ lastBackup }}</span>
    <div class="list-wide-only repo-row-quota">
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
    <span class="list-narrow-only repo-row-counts">{{ archivesText }} · {{ sizeText }}</span>
    <span
      class="list-narrow-only repo-row-status"
      :class="`repo-row-status--${status.tone}`"
    >
      {{ status.label }}
    </span>
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

/* The bar over its figure; a grid rather than a column flex, because the
   shared track grows along a flex axis and would collapse to nothing. */
.repo-row-quota {
  display: grid;
  gap: var(--space-2);
  min-width: 0;
}

.repo-row-size {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.repo-row-when,
.repo-row-counts {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.repo-row-status {
  flex-basis: 100%;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
}

.repo-row-status--success {
  color: var(--success);
}

.repo-row-status--warning {
  color: var(--warning);
}

.repo-row-status--danger {
  color: var(--danger);
}

@media (max-width: 640px) {
  /* Last backup sits at the right of the name line, clear of the chevron. */
  .repo-row-when {
    margin-left: auto;
    margin-right: calc(16px + var(--space-3));
    white-space: nowrap;
  }
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, relativeTime } from '../utils/format'
import EntityStatusBadges, { type EntityIssue } from './EntityStatusBadges.vue'
import RepoQuotaMeter from './RepoQuotaMeter.vue'
import type { RepoWithStats } from '../types/repo'

/** A tag as the repositories list resolves it for one card. */
export interface RepoCardTag {
  name: string
  color: string
}

const props = withDefaults(
  defineProps<{
    repo: RepoWithStats
    issues: EntityIssue[]
    tags?: RepoCardTag[]
    /**
     * The compact card inside a host pool, where the pool header already
     * carries the host and the usage: it drops the import progress, the
     * host-sleeps and tag pills, and the deduplicated-size stat.
     */
    pooled?: boolean
    /** Steps the card back, for a pool entry the current filter hides. */
    dim?: boolean
  }>(),
  { tags: () => [], pooled: false, dim: false },
)

defineEmits<{ select: [] }>()

const importPhaseVerb = computed<string>(() =>
  (props.repo.import_status_message ?? '').startsWith('Indexing') ? 'Indexing' : 'Importing',
)

const importPercent = computed<number>(() =>
  Math.round((props.repo.import_progress / props.repo.import_total) * 100),
)
</script>

<template>
  <div
    class="entity-card"
    :class="{
      'entity-card--notable': !repo.enabled,
      'entity-card--dim': dim,
    }"
    @click="$emit('select')"
  >
    <div class="card-top">
      <div class="card-info">
        <span class="card-name">{{ repo.name }}</span>
        <span class="card-ssh">{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</span>
      </div>
      <div class="card-badges">
        <span
          v-if="repo.import_error || repo.importing"
          class="badge"
          :class="repo.import_error ? 'badge--danger' : 'badge--warning badge--pulse'"
          :title="repo.import_error ?? undefined"
        >
          {{
            repo.import_error
              ? 'Import Failed'
              : repo.import_total > 0
                ? `${importPhaseVerb} ${repo.import_progress}/${repo.import_total}`
                : `${importPhaseVerb}\u2026`
          }}
        </span>
      </div>
    </div>
    <template v-if="!pooled">
      <div
        v-if="repo.importing && repo.import_total > 0"
        class="progress-row"
      >
        <div class="progress-track">
          <div
            class="progress-bar"
            :style="{ width: `${importPercent}%` }"
          ></div>
        </div>
        <span class="progress-label">{{ importPercent }}%</span>
      </div>
      <p
        v-if="repo.importing && repo.import_status_message"
        class="import-status-inline"
      >
        {{ repo.import_status_message }}
      </p>
    </template>
    <EntityStatusBadges
      :notable="!repo.enabled"
      notable-label="Disabled"
      :issues="issues"
    />
    <div class="card-meta">
      <span class="meta-pill">{{ repo.encryption }}</span>
      <span class="meta-pill">{{ repo.compression }}</span>
      <template v-if="!pooled">
        <span
          v-if="repo.repo_host.intermittent"
          class="meta-pill"
          title="Its repository host is not always online"
        >
          host sleeps
        </span>
        <span
          v-for="tag in tags"
          :key="tag.name"
          class="tag-pill"
          :style="{
            background: tag.color + '22',
            color: tag.color,
            borderColor: tag.color + '44',
          }"
        >
          {{ tag.name }}
        </span>
      </template>
    </div>
    <div class="card-stats">
      <div class="stat">
        <span class="stat-value">{{ repo.archive_count }}</span>
        <span class="stat-label">Archives</span>
      </div>
      <div
        v-if="!pooled"
        class="stat"
      >
        <span class="stat-value">{{ formatBytes(repo.total_deduplicated_size) }}</span>
        <span class="stat-label">Deduplicated</span>
      </div>
      <div class="stat">
        <span class="stat-value">{{ relativeTime(repo.last_backup_at ?? '') }}</span>
        <span class="stat-label">Last backup</span>
      </div>
    </div>
    <!-- A host pool swaps the meter for its slice of the shared box. -->
    <slot name="quota">
      <RepoQuotaMeter
        :quota="repo.quota"
        :usage-bytes="repo.total_deduplicated_size"
      />
    </slot>
  </div>
</template>

<style scoped>
.import-status-inline {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-badges {
  display: flex;
  gap: var(--space-3);
  align-items: center;
  flex-shrink: 0;
}

.card-ssh {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  font-family: var(--mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import EntityCard, { type EntityCardStat } from './EntityCard.vue'
import EntityStatusBadges, { type EntityIssue } from './EntityStatusBadges.vue'
import { formatBytes, relativeTime } from '../utils/format'
import type { RepoWithStats } from '../types/repo'

const props = defineProps<{
  repo: RepoWithStats
  tags: { name: string; color: string }[]
  issues: EntityIssue[]
}>()

defineEmits<{ click: [] }>()

const importPhaseVerb = computed<string>(() =>
  (props.repo.import_status_message ?? '').startsWith('Indexing') ? 'Indexing' : 'Importing',
)

const importProgressPercent = computed<number>(() => {
  if (props.repo.import_total <= 0) return 0
  return Math.round((props.repo.import_progress / props.repo.import_total) * 100)
})

const stats = computed<EntityCardStat[]>(() => [
  { value: props.repo.archive_count, label: 'Archives' },
  { value: formatBytes(props.repo.total_deduplicated_size), label: 'Deduplicated' },
  { value: relativeTime(props.repo.last_backup_at ?? ''), label: 'Last backup' },
])
</script>

<template>
  <EntityCard
    class="repo-card"
    :class="{ 'repo-card-notable': !repo.enabled }"
    :title="repo.name"
    :subtitle="`${repo.ssh_user}@${repo.ssh_host}:${repo.ssh_port}`"
    :stats="stats"
    @click="$emit('click')"
  >
    <template
      v-if="repo.import_error || repo.importing"
      #top-badges
    >
      <span
        class="status-badge"
        :class="repo.import_error ? 'status-error' : 'status-importing'"
        :title="repo.import_error ?? undefined"
      >
        {{
          repo.import_error
            ? 'Import Failed'
            : repo.import_total > 0
              ? `${importPhaseVerb} ${repo.import_progress}/${repo.import_total}`
              : `${importPhaseVerb}…`
        }}
      </span>
    </template>

    <template
      v-if="repo.importing"
      #extra
    >
      <div
        v-if="repo.import_total > 0"
        class="import-progress"
      >
        <div class="import-progress-track">
          <div
            class="import-progress-bar"
            :style="{ width: `${importProgressPercent}%` }"
          ></div>
        </div>
        <span class="import-progress-label">{{ importProgressPercent }}%</span>
      </div>
      <p
        v-if="repo.import_status_message"
        class="import-status-inline"
      >
        {{ repo.import_status_message }}
      </p>
    </template>

    <template #status>
      <EntityStatusBadges
        :notable="!repo.enabled"
        notable-label="Disabled"
        :issues="issues"
      />
    </template>

    <template #meta>
      <span class="meta-pill">{{ repo.encryption }}</span>
      <span class="meta-pill">{{ repo.compression }}</span>
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
  </EntityCard>
</template>

<style scoped>
.repo-card-notable {
  background: var(--bg-hover);
}

.import-progress {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.import-progress-track {
  flex: 1;
  height: 6px;
  background: var(--border);
  border-radius: 3px;
  overflow: hidden;
}

.import-progress-bar {
  height: 100%;
  background: var(--accent);
  border-radius: 3px;
  transition: width 0.4s ease;
}

.import-progress-label {
  font-size: 0.75rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.import-status-inline {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta-pill {
  display: inline-block;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 500;
  background: var(--bg-card);
  color: var(--text-muted);
  text-transform: lowercase;
}
</style>

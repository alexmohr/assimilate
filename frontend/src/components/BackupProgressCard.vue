<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { formatDuration, formatBytes } from '../utils/format'

interface ArchiveProgressData {
  nfiles: number
  originalSize: number
  currentPath: string
}

defineProps<{
  badge: string | null
  archiveName: string | null
  elapsedSecs: number
  estimatedRemainingSecs: number | null
  progress: ArchiveProgressData | null
}>()
</script>

<template>
  <div class="live-log-card">
    <div class="live-log-header">
      <span class="pulse-dot pulse-dot--success" />
      <span class="live-log-title">Backup in progress</span>
      <span
        v-if="badge"
        class="live-log-host-badge"
        >{{ badge }}</span
      >
    </div>
    <div class="progress-body">
      <div
        v-if="!progress"
        class="live-log-empty"
      >
        Waiting for progress...
      </div>
      <template v-else>
        <div class="live-stat-row">
          <span class="live-stat-label">Elapsed</span>
          <span class="live-stat-value">{{ formatDuration(elapsedSecs) }}</span>
        </div>
        <div
          v-if="estimatedRemainingSecs !== null"
          class="live-stat-row"
        >
          <span class="live-stat-label">Est. remaining</span>
          <span class="live-stat-value">{{ formatDuration(estimatedRemainingSecs) }}</span>
        </div>
        <div class="live-stat-row">
          <span class="live-stat-label">Files</span>
          <span class="live-stat-value">{{ progress.nfiles.toLocaleString() }}</span>
        </div>
        <div class="live-stat-row">
          <span class="live-stat-label">Data</span>
          <span class="live-stat-value">{{ formatBytes(progress.originalSize) }}</span>
        </div>
        <div
          v-if="archiveName"
          class="live-stat-row"
        >
          <span class="live-stat-label">Archive</span>
          <span class="live-stat-value progress-mono">{{ archiveName }}</span>
        </div>
        <div
          v-if="progress.currentPath"
          class="live-stat-row live-stat-row-wrap"
        >
          <span class="live-stat-label">Current file</span>
          <span class="live-stat-value progress-path">{{ progress.currentPath }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.live-log-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  margin-bottom: 1rem;
  overflow: hidden;
}

.live-log-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.live-log-title {
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.live-log-host-badge {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--accent);
  font-family: var(--mono);
}

.live-log-empty {
  padding: 0.75rem 1rem;
  color: var(--text-muted);
  font-style: italic;
}

.progress-body {
  padding: 0.5rem 0;
}

.live-stat-row {
  display: flex;
  gap: 1rem;
  padding: 0.2rem 1rem;
  font-size: var(--fs-base);
}

.live-stat-label {
  color: var(--text-muted);
  min-width: 9rem;
  flex-shrink: 0;
}

.live-stat-value {
  color: var(--text-primary);
}

.live-stat-row-wrap {
  align-items: flex-start;
}

.progress-path {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  word-break: break-all;
  overflow-wrap: break-word;
  white-space: pre-wrap;
  min-width: 0;
}

.progress-mono {
  font-family: var(--mono);
  font-size: var(--fs-xs);
}
</style>

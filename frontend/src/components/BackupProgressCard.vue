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

withDefaults(
  defineProps<{
    badge: string | null
    archiveName: string | null
    elapsedSecs: number
    estimatedRemainingSecs: number | null
    progress: ArchiveProgressData | null
    logLines?: string[]
  }>(),
  { logLines: () => [] },
)
</script>

<template>
  <div class="live-log-card">
    <div class="live-log-header">
      <span class="live-log-pulse" />
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
        Waiting for progress&hellip;
      </div>
      <template v-else>
        <div class="progress-row">
          <span class="progress-label">Elapsed</span>
          <span class="progress-value">{{ formatDuration(elapsedSecs) }}</span>
        </div>
        <div
          v-if="estimatedRemainingSecs !== null"
          class="progress-row"
        >
          <span class="progress-label">Est. remaining</span>
          <span class="progress-value">{{ formatDuration(estimatedRemainingSecs) }}</span>
        </div>
        <div class="progress-row">
          <span class="progress-label">Files</span>
          <span class="progress-value">{{ progress.nfiles.toLocaleString() }}</span>
        </div>
        <div class="progress-row">
          <span class="progress-label">Data</span>
          <span class="progress-value">{{ formatBytes(progress.originalSize) }}</span>
        </div>
        <div
          v-if="archiveName"
          class="progress-row"
        >
          <span class="progress-label">Archive</span>
          <span class="progress-value progress-mono">{{ archiveName }}</span>
        </div>
        <div
          v-if="progress.currentPath"
          class="progress-row progress-row-wrap"
        >
          <span class="progress-label">Current file</span>
          <span class="progress-value progress-path">{{ progress.currentPath }}</span>
        </div>
      </template>
    </div>
    <div
      v-if="logLines.length > 0"
      class="live-log-output"
    >
      <div
        v-for="(line, i) in logLines"
        :key="i"
        class="live-log-line"
      >
        {{ line }}
      </div>
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
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.live-log-host-badge {
  margin-left: auto;
  font-size: 0.72rem;
  color: var(--accent);
  font-family: var(--mono);
}

.live-log-pulse {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--success);
  animation: pulse 1.5s ease-in-out infinite;
  flex-shrink: 0;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.live-log-empty {
  padding: 0.75rem 1rem;
  color: var(--text-muted);
  font-style: italic;
}

.progress-body {
  padding: 0.5rem 0;
}

.progress-row {
  display: flex;
  gap: 1rem;
  padding: 0.2rem 1rem;
  font-size: 0.85rem;
}

.progress-label {
  color: var(--text-muted);
  min-width: 9rem;
  flex-shrink: 0;
}

.progress-value {
  color: var(--text-primary);
}

.progress-row-wrap {
  align-items: flex-start;
}

.progress-path {
  font-family: var(--mono);
  font-size: 0.78rem;
  word-break: break-all;
  overflow-wrap: break-word;
  white-space: pre-wrap;
  min-width: 0;
}

.progress-mono {
  font-family: var(--mono);
  font-size: 0.78rem;
}

.live-log-output {
  border-top: 1px solid var(--border);
  background: var(--bg-base);
  max-height: 200px;
  overflow-y: auto;
  padding: 0.5rem 1rem;
  font-family: var(--mono);
  font-size: 0.72rem;
  color: var(--text-secondary);
}

.live-log-line {
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 1.5;
  padding: 0.05rem 0;
}
</style>

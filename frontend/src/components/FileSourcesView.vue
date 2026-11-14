<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import ShellScript from './ShellScript.vue'
import { parseFileChangePatterns } from '../utils/fileChangePatterns'
import type { HookCommand } from '../types/generated'

/**
 * The read-only face of `FileSourcesEditor`: a file set's paths, excludes,
 * file change patterns and commands, as a host or a schedule has saved them.
 */
const props = withDefaults(
  defineProps<{
    paths: readonly string[]
    excludes: readonly string[]
    includes?: readonly string[]
    fileChangePatternsRaw: string
    preCommands: readonly HookCommand[]
    postCommands: readonly HookCommand[]
    /** What a command without its own timeout runs under, in words. */
    inheritedTimeout?: string
  }>(),
  { includes: undefined, inheritedTimeout: "the schedule's hook command timeout" },
)

const fileChangePatterns = computed(() => parseFileChangePatterns(props.fileChangePatternsRaw))

function timeoutLabel(cmd: HookCommand): string {
  return cmd.timeout_seconds === null
    ? `Timeout: ${props.inheritedTimeout}`
    : `Timeout: ${cmd.timeout_seconds}s`
}
</script>

<template>
  <div class="file-sources-view">
    <div class="file-sources-group">
      <span class="group-label group-label--lg">Backup paths</span>
      <slot name="paths">
        <div
          v-if="paths.length > 0"
          class="paths-list"
        >
          <code
            v-for="(p, idx) in paths"
            :key="idx"
            class="path-item mono"
            >{{ p }}</code
          >
        </div>
        <span
          v-else
          class="muted"
          >No paths configured.</span
        >
      </slot>
    </div>

    <div class="file-sources-group">
      <span class="group-label group-label--lg">Exclude patterns</span>
      <div
        v-if="excludes.length > 0"
        class="paths-list"
      >
        <code
          v-for="(p, idx) in excludes"
          :key="idx"
          class="path-item mono"
          >{{ p }}</code
        >
      </div>
      <span
        v-else
        class="muted"
        >No excludes configured.</span
      >
    </div>

    <div
      v-if="includes !== undefined"
      class="file-sources-group"
    >
      <span class="group-label group-label--lg">Include patterns</span>
      <div
        v-if="includes.length > 0"
        class="paths-list"
      >
        <code
          v-for="(p, idx) in includes"
          :key="idx"
          class="path-item mono"
          >{{ p }}</code
        >
      </div>
      <span
        v-else
        class="muted"
        >None.</span
      >
    </div>

    <div class="file-sources-group">
      <span class="group-label group-label--lg">File change patterns</span>
      <div
        v-if="fileChangePatterns.length > 0"
        class="paths-list"
      >
        <code
          v-for="(p, idx) in fileChangePatterns"
          :key="idx"
          class="path-item mono"
        >
          {{ p.path }} <span class="fcp-action-badge">{{ p.action }}</span>
        </code>
      </div>
      <span
        v-else
        class="muted"
        >No file change patterns configured.</span
      >
    </div>

    <div class="file-sources-group">
      <span class="group-label group-label--lg">Pre-backup commands</span>
      <div
        v-if="preCommands.length > 0"
        class="cmd-view-list"
      >
        <div
          v-for="(cmd, idx) in preCommands"
          :key="idx"
          class="cmd-view-entry"
        >
          <ShellScript :source="cmd.command" />
          <span class="field-hint">{{ timeoutLabel(cmd) }}</span>
        </div>
      </div>
      <span
        v-else
        class="muted"
        >None configured.</span
      >
    </div>

    <div class="file-sources-group">
      <span class="group-label group-label--lg">Post-backup commands</span>
      <div
        v-if="postCommands.length > 0"
        class="cmd-view-list"
      >
        <div
          v-for="(cmd, idx) in postCommands"
          :key="idx"
          class="cmd-view-entry"
        >
          <ShellScript :source="cmd.command" />
          <span class="field-hint">{{ timeoutLabel(cmd) }}</span>
        </div>
      </div>
      <span
        v-else
        class="muted"
        >None configured.</span
      >
    </div>
  </div>
</template>

<style scoped>
.file-sources-view {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  margin-bottom: var(--space-4);
}

.file-sources-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.cmd-view-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.cmd-view-entry {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.fcp-action-badge {
  font-family: var(--font-sans);
  font-size: var(--fs-2xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import ToggleSwitch from './ToggleSwitch.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import CommandListEditor from './CommandListEditor.vue'
import PerAgentFields from './PerAgentFields.vue'
import BorgPatternReference from './BorgPatternReference.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

/**
 * The backup schedule's Advanced tab: borg options, exclude patterns, file
 * change patterns and hook commands, each of which can be overridden per agent
 * on a multi-host schedule.
 */
defineProps<{
  agentIds: number[]
  agentLabel: (id: number) => string
}>()

const form = defineModel<ScheduleFormState>('form', { required: true })
const overrides = defineModel<ScheduleAgentOverrides>('overrides', { required: true })

const refOpen = ref(false)
</script>

<template>
  <div class="form-stack">
    <p class="pane-lede">
      Settings most schedules leave alone: bandwidth and verification, the patterns that decide what
      is skipped, and commands to run around each backup.
    </p>
    <section class="pane-section">
      <span class="group-label group-label--lg">Options</span>
      <div class="field field-inline">
        <label class="field-label">Canary verification</label>
        <ToggleSwitch v-model="form.canary_enabled" />
      </div>
      <div class="field field-inline">
        <label class="field-label">Ignore global excludes</label>
        <ToggleSwitch v-model="form.ignore_global_excludes" />
      </div>
      <div class="field field-inline">
        <label class="field-label">Compact after backup</label>
        <ToggleSwitch v-model="form.compact_enabled" />
      </div>
      <div class="field">
        <label class="field-label">Remote rate limit (kB/s)</label>
        <input
          v-model.number="form.rate_limit_kbps"
          type="number"
          min="0"
          class="input"
        />
        <span class="field-hint">Caps borg's upload bandwidth. Set to 0 for unlimited.</span>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Exclude patterns</span>
      <div
        v-if="agentIds.length > 1"
        class="field field-inline"
      >
        <label class="field-label">Configure per agent</label>
        <ToggleSwitch v-model="overrides.usePerHostExcludes" />
      </div>
      <div class="field">
        <div class="field-label-row">
          <label class="field-label">Patterns</label>
          <button
            type="button"
            class="ref-toggle"
            @click="refOpen = !refOpen"
          >
            {{ refOpen ? 'Close Reference' : 'Pattern Reference' }}
          </button>
        </div>
        <textarea
          v-if="!overrides.usePerHostExcludes"
          v-model="form.exclude_patterns"
          class="input area-input"
          placeholder="One pattern per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
          spellcheck="false"
        />
        <PerAgentFields
          v-else
          :agent-ids="agentIds"
          :agent-label="agentLabel"
        >
          <template #default="{ agentId }">
            <textarea
              :value="overrides.perHostExcludes[agentId] ?? ''"
              class="input area-input area-input-sm"
              placeholder="Exclude patterns, one per line"
              spellcheck="false"
              @input="
                ($event) =>
                  (overrides.perHostExcludes[agentId] = (
                    $event.target as HTMLTextAreaElement
                  ).value)
              "
            />
          </template>
          <template #hint>
            Leave an agent empty to use only global and agent-level default excludes.
          </template>
        </PerAgentFields>
        <span
          v-if="!overrides.usePerHostExcludes"
          class="field-hint"
        >
          Leave empty to use only global and agent-level default excludes. Lines starting with
          <code>#</code> are treated as comments.
        </span>
        <BorgPatternReference v-if="refOpen" />
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">File change patterns</span>
      <div
        v-if="agentIds.length > 1"
        class="field field-inline"
      >
        <label class="field-label">Configure per agent</label>
        <ToggleSwitch v-model="overrides.usePerHostFileChangePatterns" />
      </div>
      <div class="field">
        <label class="field-label">Patterns</label>
        <FileChangePatternsEditor
          v-if="!overrides.usePerHostFileChangePatterns"
          v-model="form.file_change_patterns"
        />
        <PerAgentFields
          v-else
          :agent-ids="agentIds"
          :agent-label="agentLabel"
        >
          <template #default="{ agentId }">
            <textarea
              :value="overrides.perHostFileChangePatterns[agentId] ?? ''"
              class="input area-input area-input-sm"
              placeholder="File change patterns, one per line"
              spellcheck="false"
              @input="
                ($event) =>
                  (overrides.perHostFileChangePatterns[agentId] = (
                    $event.target as HTMLTextAreaElement
                  ).value)
              "
            />
          </template>
          <template #hint>
            Leave an agent empty to use schedule-level file change patterns.
          </template>
        </PerAgentFields>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Commands</span>
      <div
        v-if="agentIds.length > 1"
        class="field field-inline"
      >
        <label class="field-label">Configure per agent</label>
        <ToggleSwitch v-model="overrides.usePerAgentCmds" />
      </div>
      <div class="field">
        <label class="field-label">Hook command timeout (seconds)</label>
        <input
          v-model.number="form.hook_timeout_seconds"
          type="number"
          min="1"
          max="3600"
          class="input"
        />
        <span class="field-hint">
          Applied to each pre- and post-backup command. A command still running past this is killed
          and the backup fails.
        </span>
      </div>
      <template v-if="!overrides.usePerAgentCmds">
        <div class="field">
          <label class="field-label">Pre-backup commands</label>
          <CommandListEditor
            v-model="form.pre_backup_commands"
            placeholder="e.g. docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
            aria-label="Pre-backup commands"
          />
        </div>
        <div class="field">
          <label class="field-label">Post-backup commands</label>
          <CommandListEditor
            v-model="form.post_backup_commands"
            placeholder="e.g. rm /tmp/dump.sql (optional)"
            aria-label="Post-backup commands"
          />
        </div>
      </template>
      <PerAgentFields
        v-else
        :agent-ids="agentIds"
        :agent-label="agentLabel"
      >
        <template #default="{ agentId }">
          <label class="form-sublabel">Pre-backup</label>
          <CommandListEditor
            :model-value="overrides.perAgentPreCmds[agentId] ?? []"
            placeholder="e.g. docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
            aria-label="Pre-backup commands"
            @update:model-value="(v) => (overrides.perAgentPreCmds[agentId] = v)"
          />
          <label class="form-sublabel">Post-backup</label>
          <CommandListEditor
            :model-value="overrides.perAgentPostCmds[agentId] ?? []"
            placeholder="e.g. rm /tmp/dump.sql (optional)"
            aria-label="Post-backup commands"
            @update:model-value="(v) => (overrides.perAgentPostCmds[agentId] = v)"
          />
        </template>
        <template #hint>Leave an agent empty to run no schedule-level commands.</template>
      </PerAgentFields>
    </section>
  </div>
</template>

<style scoped>
.ref-toggle {
  padding: var(--space-1) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  font-weight: 500;
  cursor: pointer;
  transition:
    color var(--duration-base),
    background var(--duration-base);
}

.ref-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.form-sublabel {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-top: var(--space-4);
  display: block;
}
</style>

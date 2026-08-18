<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import ToggleSwitch from './ToggleSwitch.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
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
    <div class="panel">
      <h3 class="info-title">Options</h3>
      <div class="field field-inline">
        <label class="field-label">Canary Verification</label>
        <ToggleSwitch v-model="form.canary_enabled" />
      </div>
      <div class="field field-inline">
        <label class="field-label">Ignore Global Excludes</label>
        <ToggleSwitch v-model="form.ignore_global_excludes" />
      </div>
      <div class="field field-inline">
        <label class="field-label">Compact after backup</label>
        <ToggleSwitch v-model="form.compact_enabled" />
      </div>
      <div class="field">
        <label class="field-label">Remote Rate Limit (kB/s)</label>
        <input
          v-model.number="form.rate_limit_kbps"
          type="number"
          min="0"
          class="input"
        />
        <span class="field-hint">Caps borg's upload bandwidth. Set to 0 for unlimited.</span>
      </div>
    </div>

    <div class="panel">
      <h3 class="info-title">Exclude Patterns</h3>
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
    </div>

    <div class="panel">
      <h3 class="info-title">File Change Patterns</h3>
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
    </div>

    <div class="panel">
      <h3 class="info-title">Commands</h3>
      <div
        v-if="agentIds.length > 1"
        class="field field-inline"
      >
        <label class="field-label">Configure per agent</label>
        <ToggleSwitch v-model="overrides.usePerAgentCmds" />
      </div>
      <template v-if="!overrides.usePerAgentCmds">
        <div class="field">
          <label class="field-label">Pre-backup Commands</label>
          <textarea
            v-model="form.pre_backup_commands"
            class="input cmd-area"
            placeholder="One command per line, e.g.&#10;docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
            spellcheck="false"
          />
        </div>
        <div class="field">
          <label class="field-label">Post-backup Commands</label>
          <textarea
            v-model="form.post_backup_commands"
            class="input cmd-area"
            placeholder="One command per line (optional)"
            spellcheck="false"
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
          <textarea
            :value="overrides.perAgentPreCmds[agentId] ?? ''"
            class="input cmd-area"
            placeholder="One command per line"
            spellcheck="false"
            @input="
              ($event) =>
                (overrides.perAgentPreCmds[agentId] = ($event.target as HTMLTextAreaElement).value)
            "
          />
          <label class="form-sublabel">Post-backup</label>
          <textarea
            :value="overrides.perAgentPostCmds[agentId] ?? ''"
            class="input cmd-area"
            placeholder="One command per line (optional)"
            spellcheck="false"
            @input="
              ($event) =>
                (overrides.perAgentPostCmds[agentId] = ($event.target as HTMLTextAreaElement).value)
            "
          />
        </template>
        <template #hint>Leave an agent empty to run no schedule-level commands.</template>
      </PerAgentFields>
    </div>
  </div>
</template>

<style scoped>
.cmd-area {
  min-height: 60px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.ref-toggle {
  padding: 0.15rem 0.5rem;
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
  margin-top: 0.5rem;
  display: block;
}
</style>

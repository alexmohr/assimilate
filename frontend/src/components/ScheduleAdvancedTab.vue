<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import ToggleSwitch from './ToggleSwitch.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import CommandListEditor from './CommandListEditor.vue'
import HelpHint from './HelpHint.vue'
import PerAgentFields from './PerAgentFields.vue'
import BorgPatternReference from './BorgPatternReference.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

/**
 * The backup schedule's Advanced tab: borg options, exclude patterns, file
 * change patterns and hook commands, each of which can be overridden per agent
 * on a multi-host schedule.
 *
 * Every setting is a `.pane-row`: what it is on the left, the control on
 * the right. The pane used to be `.field-inline` rows, which put each switch
 * at the far edge of the pane with its label a screen away, and let every hint
 * run the full width of the window - six paragraphs at the same visual weight
 * as the settings they explain.
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
    <div class="pane-head pane-head--end">
      <HelpHint
        label="bandwidth, patterns and commands"
        align="end"
      >
        Settings most schedules leave alone: bandwidth and verification, the patterns that decide
        what is skipped, and commands to run around each backup.
      </HelpHint>
    </div>
    <section class="pane-section">
      <span class="group-label group-label--lg">Options</span>
      <div class="pane-rows">
        <div class="pane-row">
          <div class="field-body">
            <p class="field-title">
              Canary verification
              <HelpHint label="catching a silent failure">
                Writes a canary file before the backup and verifies it afterwards, so a silent
                failure does not pass as a success.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="form.canary_enabled"
              label="Canary verification"
            />
          </div>
        </div>
        <div class="pane-row">
          <div class="field-body">
            <p class="field-title">
              Ignore global excludes
              <HelpHint label="skipping the server-wide list">
                Back up using only this schedule's patterns, not the server-wide exclude list.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="form.ignore_global_excludes"
              label="Ignore global excludes"
            />
          </div>
        </div>
        <div class="pane-row">
          <div class="field-body">
            <p class="field-title">
              Compact after backup
              <HelpHint label="reclaiming freed space">
                Runs borg compact once pruning is done, to reclaim the space it freed.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="form.compact_enabled"
              label="Compact after backup"
            />
          </div>
        </div>
        <div class="pane-row">
          <div class="field-body">
            <p class="field-title">
              Back up virtual machines
              <HelpHint label="VM snapshots">
                Snapshots the libvirt domains of every host this schedule targets before the backup
                starts, using each host's own staging settings. Hosts that do not allow it are
                skipped.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="form.vm_snapshot_enabled"
              label="Back up virtual machines"
            />
          </div>
        </div>
        <div class="pane-row">
          <div class="field-body">
            <div class="field-label-row field-label-row--tight">
              <label
                class="field-title"
                for="schedule-rate-limit"
              >
                Remote rate limit (kB/s)
              </label>
              <HelpHint
                label="capping upload bandwidth"
                align="end"
              >
                Caps borg's upload bandwidth.
              </HelpHint>
            </div>
            <p class="field-hint">Set to 0 for unlimited.</p>
          </div>
          <div class="pane-row-control">
            <input
              id="schedule-rate-limit"
              v-model.number="form.rate_limit_kbps"
              type="number"
              min="0"
              class="input"
            />
          </div>
        </div>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Exclude patterns</span>
      <div class="pane-rows">
        <div
          v-if="agentIds.length > 1"
          class="pane-row"
        >
          <div class="field-body">
            <p class="field-title">
              Configure per agent
              <HelpHint label="splitting the list by host">
                Give each host its own patterns instead of one list for the schedule.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="overrides.usePerHostExcludes"
              label="Configure per agent (exclude patterns)"
            />
          </div>
        </div>
        <div class="pane-row pane-row--stack">
          <div class="field-body">
            <div class="field-label-row">
              <p class="field-title">
                Patterns
                <HelpHint
                  v-if="!overrides.usePerHostExcludes"
                  label="pattern syntax"
                >
                  Leave empty to use only global and agent-level default excludes. Lines starting
                  with <code>#</code> are treated as comments.
                </HelpHint>
              </p>
              <button
                type="button"
                class="ref-toggle"
                @click="refOpen = !refOpen"
              >
                {{ refOpen ? 'Close Reference' : 'Pattern Reference' }}
              </button>
            </div>
          </div>
          <div class="pane-row-control">
            <textarea
              v-if="!overrides.usePerHostExcludes"
              v-model="form.exclude_patterns"
              class="input area-input"
              aria-label="Exclude patterns"
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
            <BorgPatternReference v-if="refOpen" />
          </div>
        </div>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">File change patterns</span>
      <div class="pane-rows">
        <div
          v-if="agentIds.length > 1"
          class="pane-row"
        >
          <div class="field-body">
            <p class="field-title">
              Configure per agent
              <HelpHint label="configure file change patterns per agent">
                Give each host its own patterns instead of one list for the schedule.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="overrides.usePerHostFileChangePatterns"
              label="Configure per agent (file change patterns)"
            />
          </div>
        </div>
        <div class="pane-row pane-row--stack">
          <div class="field-body">
            <p class="field-title">Patterns</p>
          </div>
          <div class="pane-row-control">
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
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Commands</span>
      <div class="pane-rows">
        <div
          v-if="agentIds.length > 1"
          class="pane-row"
        >
          <div class="field-body">
            <p class="field-title">
              Configure per agent
              <HelpHint label="configure commands per agent">
                Give each host its own commands instead of one set for the schedule.
              </HelpHint>
            </p>
          </div>
          <div class="pane-row-control">
            <ToggleSwitch
              v-model="overrides.usePerAgentCmds"
              label="Configure per agent (commands)"
            />
          </div>
        </div>
        <div class="pane-row">
          <div class="field-body">
            <div class="field-label-row field-label-row--tight">
              <label
                class="field-title"
                for="schedule-hook-timeout"
              >
                Hook command timeout (seconds)
              </label>
              <HelpHint label="the default per-command budget">
                The default for every pre- and post-backup command that does not set its own. A
                command still running past its timeout is killed and the backup fails.
              </HelpHint>
            </div>
          </div>
          <div class="pane-row-control">
            <input
              id="schedule-hook-timeout"
              v-model.number="form.hook_timeout_seconds"
              type="number"
              min="1"
              max="3600"
              class="input"
            />
          </div>
        </div>
        <template v-if="!overrides.usePerAgentCmds">
          <div class="pane-row pane-row--stack">
            <div class="field-body">
              <p class="field-title">Pre-backup commands</p>
            </div>
            <div class="pane-row-control">
              <CommandListEditor
                v-model="form.pre_backup_commands"
                placeholder="e.g. docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
                aria-label="Pre-backup commands"
                :default-timeout-seconds="form.hook_timeout_seconds"
              />
            </div>
          </div>
          <div class="pane-row pane-row--stack">
            <div class="field-body">
              <p class="field-title">Post-backup commands</p>
            </div>
            <div class="pane-row-control">
              <CommandListEditor
                v-model="form.post_backup_commands"
                placeholder="e.g. rm /tmp/dump.sql (optional)"
                aria-label="Post-backup commands"
                :default-timeout-seconds="form.hook_timeout_seconds"
              />
            </div>
          </div>
        </template>
        <div
          v-else
          class="pane-row pane-row--stack"
        >
          <div class="pane-row-control">
            <PerAgentFields
              :agent-ids="agentIds"
              :agent-label="agentLabel"
            >
              <template #default="{ agentId }">
                <label class="form-sublabel">Pre-backup</label>
                <CommandListEditor
                  :model-value="overrides.perAgentPreCmds[agentId] ?? []"
                  placeholder="e.g. docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
                  aria-label="Pre-backup commands"
                  :default-timeout-seconds="form.hook_timeout_seconds"
                  @update:model-value="(v) => (overrides.perAgentPreCmds[agentId] = v)"
                />
                <label class="form-sublabel">Post-backup</label>
                <CommandListEditor
                  :model-value="overrides.perAgentPostCmds[agentId] ?? []"
                  placeholder="e.g. rm /tmp/dump.sql (optional)"
                  aria-label="Post-backup commands"
                  :default-timeout-seconds="form.hook_timeout_seconds"
                  @update:model-value="(v) => (overrides.perAgentPostCmds[agentId] = v)"
                />
              </template>
              <template #hint>Leave an agent empty to run no schedule-level commands.</template>
            </PerAgentFields>
          </div>
        </div>
      </div>
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

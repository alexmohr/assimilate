<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { CornerDownRight } from '@lucide/vue'
import { computed, ref } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import CommandListEditor from './CommandListEditor.vue'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
import PerAgentFields from './PerAgentFields.vue'
import BorgPatternReference from './BorgPatternReference.vue'
import type { AgentRow } from '../types/agent'
import { isFsSnapshotOn } from '../utils/fsSnapshot'
import type { FsSnapshotMode } from '../types/generated'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'

/**
 * The backup schedule's Advanced tab: borg options, filesystem snapshots,
 * exclude patterns, file change patterns and hook commands, each of which can be overridden per agent
 * on a multi-host schedule.
 *
 * Every setting is a `.pane-row`: what it is on the left, the control on
 * the right. The pane used to be `.field-inline` rows, which put each switch
 * at the far edge of the pane with its label a screen away, and let every hint
 * run the full width of the window - six paragraphs at the same visual weight
 * as the settings they explain.
 */
const props = withDefaults(
  defineProps<{
    agentIds: number[]
    agentLabel: (id: number) => string
    /** The schedule's agents, for the snapshot tools each one reported. */
    agents?: readonly AgentRow[]
  }>(),
  { agents: () => [] },
)

const form = defineModel<ScheduleFormState>('form', { required: true })
const overrides = defineModel<ScheduleAgentOverrides>('overrides', { required: true })

const refOpen = ref(false)

const FS_SNAPSHOT_OPTIONS: readonly SegmentedOption<FsSnapshotMode>[] = [
  { value: 'off', label: 'Off' },
  { value: 'required', label: 'Required' },
  { value: 'best_effort', label: 'If possible' },
]

const snapshotsOn = computed(() => isFsSnapshotOn(form.value.fs_snapshot))

/**
 * The connected hosts of this schedule that reported no snapshot tool when
 * they connected: either none is installed, or the agent predates the report.
 * A host that is offline has reported nothing yet, so it is left out rather
 * than reported as lacking them.
 */
const hostsWithoutSnapshotTools = computed(() =>
  props.agents
    .filter((agent) => props.agentIds.includes(agent.id))
    .filter((agent) => agent.is_connected && agent.snapshot_tools.length === 0)
    .map((agent) => props.agentLabel(agent.id)),
)
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
        <PaneRow
          title="Canary verification"
          help="catching a silent failure"
        >
          <template #help>
            Writes a canary file before the backup and verifies it afterwards, so a silent failure
            does not pass as a success.
          </template>
          <ToggleSwitch
            v-model="form.canary_enabled"
            label="Canary verification"
          />
        </PaneRow>
        <PaneRow
          title="Ignore global excludes"
          help="skipping the server-wide list"
        >
          <template #help>
            Back up using only this schedule's patterns, not the server-wide exclude list.
          </template>
          <ToggleSwitch
            v-model="form.ignore_global_excludes"
            label="Ignore global excludes"
          />
        </PaneRow>
        <PaneRow
          title="Compact after backup"
          help="reclaiming freed space"
        >
          <template #help>
            Runs borg compact once pruning is done, to reclaim the space it freed.
          </template>
          <ToggleSwitch
            v-model="form.compact_enabled"
            label="Compact after backup"
          />
        </PaneRow>
        <PaneRow
          title="Back up virtual machines"
          help="VM snapshots"
        >
          <template #help>
            Snapshots the libvirt domains of every host this schedule targets before the backup
            starts, using each host's own staging settings. Hosts that do not allow it are skipped.
          </template>
          <ToggleSwitch
            v-model="form.vm_snapshot_enabled"
            label="Back up virtual machines"
          />
        </PaneRow>
        <PaneRow
          title="Dump databases"
          help="database dumps"
        >
          <template #help>
            Streams a dump of the Kubernetes databases of every host this schedule targets into the
            same archive, using each host's own database settings. Hosts that do not allow it are
            skipped.
          </template>
          <ToggleSwitch
            v-model="form.database_backup_enabled"
            label="Dump databases"
          />
        </PaneRow>
        <PaneRow
          title="Filesystem snapshots"
          help="reading a frozen copy of the sources"
          :hint="
            snapshotsOn && hostsWithoutSnapshotTools.length > 0
              ? `No zfs, btrfs or LVM tools reported by ${hostsWithoutSnapshotTools.join(', ')}. An agent older than this server reports none: update it if the tools are installed.`
              : undefined
          "
        >
          <template #help>
            Right before the backup, snapshots the ZFS dataset, btrfs subvolume or LVM volume under
            each source, and backs up the snapshot under the original paths, so every file is read
            as it was at one moment. <strong>Required</strong> fails the run when a source cannot be
            snapshotted; <strong>If possible</strong> backs such a source up live and ends the run
            with a warning.
          </template>
          <BaseSegmented
            v-model="form.fs_snapshot"
            :options="FS_SNAPSHOT_OPTIONS"
            label="Filesystem snapshots"
          />
        </PaneRow>
        <div
          v-if="snapshotsOn"
          class="pane-nest"
        >
          <div class="pane-rows">
            <PaneRow
              title="LVM snapshot size (%)"
              label-for="schedule-lvm-snapshot-percent"
              hint="Of each volume. Thin volumes need none."
              help="room for changes while borg reads"
            >
              <template #help>
                A classic LVM snapshot holds the blocks that change while the backup reads it. One
                that fills up is dropped by LVM and fails the backup, so size it for the writes a
                backup's length sees.
              </template>
              <input
                id="schedule-lvm-snapshot-percent"
                v-model.number="form.lvm_snapshot_percent"
                type="number"
                min="1"
                max="100"
                class="input"
              />
            </PaneRow>
          </div>
        </div>
        <!-- No `align="end"` here, though `main`'s markup had it: that was for
             the old mobile layout, where every row stacked, the label column
             ran the full width and pushed this icon far enough right that the
             popover spilled. The narrow mobile track moves the icon back left,
             so at 390px the default now fits (67..287 of 390) and `end` would
             throw the popover 135px off the left edge instead. -->
        <PaneRow
          title="Remote rate limit (kB/s)"
          label-for="schedule-rate-limit"
          hint="Set to 0 for unlimited."
          help="capping upload bandwidth"
        >
          <template #help> Caps borg's upload bandwidth. </template>
          <input
            id="schedule-rate-limit"
            v-model.number="form.rate_limit_kbps"
            type="number"
            min="0"
            class="input"
          />
        </PaneRow>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Exclude patterns</span>
      <div class="pane-rows">
        <PaneRow
          v-if="agentIds.length > 1"
          title="Configure per agent"
          help="splitting the list by host"
        >
          <template #help>
            Give each host its own patterns instead of one list for the schedule.
          </template>
          <ToggleSwitch
            v-model="overrides.usePerHostExcludes"
            label="Configure per agent (exclude patterns)"
          />
        </PaneRow>
        <PaneRow
          title="Patterns"
          :help="overrides.usePerHostExcludes ? undefined : 'pattern syntax'"
          stack
        >
          <template #help>
            Leave empty to use only global and agent-level default excludes. Lines starting with
            <code>#</code> are treated as comments.
          </template>
          <template #titleAside>
            <button
              type="button"
              class="ref-toggle"
              @click="refOpen = !refOpen"
            >
              {{ refOpen ? 'Close Reference' : 'Pattern Reference' }}
            </button>
          </template>
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
        </PaneRow>
        <p class="pane-exception">
          <CornerDownRight :size="14" />
          Exceptions to the excludes above
        </p>
        <div class="pane-nest">
          <div class="pane-rows">
            <PaneRow
              v-if="agentIds.length > 1"
              title="Configure per agent"
              help="splitting the includes by host"
            >
              <template #help>
                Give each host its own patterns instead of one list for the schedule.
              </template>
              <ToggleSwitch
                v-model="overrides.usePerHostIncludes"
                label="Configure per agent (include patterns)"
              />
            </PaneRow>
            <PaneRow
              title="Patterns"
              :help="overrides.usePerHostIncludes ? undefined : 'rescuing paths from the excludes'"
              stack
            >
              <template #help>
                Rescues paths from the exclude patterns above - checked first, so a path matching
                one of these is backed up even if a broader exclude would otherwise skip it. Leave
                empty to exclude everything the exclude patterns cover.
              </template>
              <textarea
                v-if="!overrides.usePerHostIncludes"
                v-model="form.include_patterns"
                class="input area-input"
                aria-label="Include patterns"
                placeholder="One pattern per line&#10;# Lines starting with # are comments&#10;e.g. /home/keep&#10;pp:/var/keep"
                spellcheck="false"
              />
              <PerAgentFields
                v-else
                :agent-ids="agentIds"
                :agent-label="agentLabel"
              >
                <template #default="{ agentId }">
                  <textarea
                    :value="overrides.perHostIncludes[agentId] ?? ''"
                    class="input area-input area-input-sm"
                    placeholder="Include patterns, one per line"
                    spellcheck="false"
                    @input="
                      ($event) =>
                        (overrides.perHostIncludes[agentId] = (
                          $event.target as HTMLTextAreaElement
                        ).value)
                    "
                  />
                </template>
                <template #hint>
                  Leave an agent empty to exclude everything its exclude patterns cover.
                </template>
              </PerAgentFields>
            </PaneRow>
          </div>
        </div>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">File change patterns</span>
      <div class="pane-rows">
        <PaneRow
          v-if="agentIds.length > 1"
          title="Configure per agent"
          help="configure file change patterns per agent"
        >
          <template #help>
            Give each host its own patterns instead of one list for the schedule.
          </template>
          <ToggleSwitch
            v-model="overrides.usePerHostFileChangePatterns"
            label="Configure per agent (file change patterns)"
          />
        </PaneRow>
        <PaneRow
          title="Patterns"
          stack
        >
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
        </PaneRow>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Commands</span>
      <div class="pane-rows">
        <PaneRow
          v-if="agentIds.length > 1"
          title="Configure per agent"
          help="configure commands per agent"
        >
          <template #help>
            Give each host its own commands instead of one set for the schedule.
          </template>
          <ToggleSwitch
            v-model="overrides.usePerAgentCmds"
            label="Configure per agent (commands)"
          />
        </PaneRow>
        <PaneRow
          title="Hook command timeout (seconds)"
          label-for="schedule-hook-timeout"
          help="the default per-command budget"
        >
          <template #help>
            The default for every pre- and post-backup command that does not set its own. A command
            still running past its timeout is killed and the backup fails.
          </template>
          <input
            id="schedule-hook-timeout"
            v-model.number="form.hook_timeout_seconds"
            type="number"
            min="1"
            max="3600"
            class="input"
          />
        </PaneRow>
        <template v-if="!overrides.usePerAgentCmds">
          <PaneRow
            title="Pre-backup commands"
            stack
          >
            <CommandListEditor
              v-model="form.pre_backup_commands"
              placeholder="e.g. docker exec mydb pg_dump -U postgres mydb > /tmp/dump.sql"
              aria-label="Pre-backup commands"
              :default-timeout-seconds="form.hook_timeout_seconds"
            />
          </PaneRow>
          <PaneRow
            title="Post-backup commands"
            stack
          >
            <CommandListEditor
              v-model="form.post_backup_commands"
              placeholder="e.g. rm /tmp/dump.sql (optional)"
              aria-label="Post-backup commands"
              :default-timeout-seconds="form.hook_timeout_seconds"
            />
          </PaneRow>
        </template>
        <PaneRow
          v-else
          stack
        >
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
        </PaneRow>
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

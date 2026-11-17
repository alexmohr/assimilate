<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import PaneRow from './PaneRow.vue'
import type { AgentRow } from '../types/agent'
import { isFsSnapshotOn } from '../utils/fsSnapshot'
import type { FsSnapshotMode } from '../types/generated'
import type { ScheduleFormState } from '../types/scheduleForm'

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
    <div class="pane-head">
      <p class="pane-lede">
        Settings most schedules leave alone: bandwidth, verification, snapshots and command
        timeouts. What a schedule backs up, with its excludes and commands, is under Sources.
      </p>
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
      </div>
    </section>
  </div>
</template>

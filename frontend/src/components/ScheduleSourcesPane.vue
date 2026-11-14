<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import DatabaseDumpMismatch from './DatabaseDumpMismatch.vue'
import PaneRow from './PaneRow.vue'
import PerAgentFields from './PerAgentFields.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import { useHostDumpSettings } from '../composables/useHostDumpSettings'
import { parseLines } from '../utils/validation'
import type { AgentRow } from '../types/agent'
import type { ScheduleFilesMode, ScheduleFormState } from '../types/scheduleForm'

/**
 * What a backup schedule takes from each of its hosts.
 *
 * Every host lists what it has under its own Sources tab: files, virtual
 * machines and databases. A schedule usually takes all of it; this pane is
 * where one picks something narrower, such as only /home, or only the
 * databases. It replaces the backup paths that sat under Targets and the VM
 * and database switches that sat under Advanced.
 */
const props = defineProps<{
  agentIds: number[]
  agentLabel: (id: number) => string
  agents: readonly AgentRow[]
}>()

const form = defineModel<ScheduleFormState>('form', { required: true })
const usePerHostPaths = defineModel<boolean>('usePerHostPaths', { required: true })
const perHostSources = defineModel<Record<number, string>>('perHostSources', { required: true })

const FILES_OPTIONS: readonly SegmentedOption<ScheduleFilesMode>[] = [
  { value: 'host', label: "Each host's paths" },
  { value: 'custom', label: 'Only these paths' },
  { value: 'off', label: 'Off' },
]

const targetAgents = computed(() =>
  props.agentIds
    .map((id) => props.agents.find((agent) => agent.id === id))
    .filter((agent): agent is AgentRow => agent !== undefined),
)

const { hostsWithDumpsOff } = useHostDumpSettings(() => targetAgents.value)

interface HostPreview {
  id: number
  label: string
  link: string
  files: string[]
  filesNote: string | null
  commands: string
  commandsMuted: boolean
  vms: string
  databases: string
  databasesOn: boolean
}

function filesFor(agent: AgentRow): string[] {
  switch (form.value.files_mode) {
    case 'off':
      return []
    case 'custom':
      return usePerHostPaths.value
        ? parseLines(perHostSources.value[agent.id] ?? '')
        : parseLines(form.value.backup_sources)
    case 'host':
      return agent.default_backup_paths
  }
}

function commandsFor(agent: AgentRow): { text: string; muted: boolean } {
  if (form.value.files_mode === 'off' || !form.value.run_host_commands)
    return { text: 'Skipped', muted: true }
  const pre = agent.default_pre_backup_commands.length
  const post = agent.default_post_backup_commands.length
  return pre + post === 0
    ? { text: 'None', muted: true }
    : { text: `${pre} pre, ${post} post`, muted: false }
}

/** Mirrors what the server assembles for each host, so nobody has to guess. */
const preview = computed<HostPreview[]>(() =>
  targetAgents.value.map((agent) => {
    const files = filesFor(agent)
    const commands = commandsFor(agent)
    const databasesOn = form.value.database_backup_enabled && agent.database_backup_enabled
    let filesNote: string | null = null
    if (form.value.files_mode === 'off') filesNote = 'Off'
    else if (files.length === 0)
      filesNote =
        form.value.files_mode === 'host' ? 'No paths set on this host' : 'No paths given for it'
    else if (form.value.files_mode === 'custom')
      filesNote = `replaces the host's ${agent.default_backup_paths.length}`
    return {
      id: agent.id,
      label: props.agentLabel(agent.id),
      link: `/agents/${encodeURIComponent(agent.hostname)}?tab=sources`,
      files,
      filesNote,
      commands: commands.text,
      commandsMuted: commands.muted,
      vms: form.value.vm_snapshot_enabled ? 'Those it allows' : 'Off',
      databases: databasesOn
        ? 'Those it allows'
        : form.value.database_backup_enabled
          ? 'Off on this host'
          : 'Off',
      databasesOn,
    }
  }),
)
</script>

<template>
  <div class="form-stack">
    <div class="pane-head">
      <p class="pane-lede">
        Each host lists its own files, virtual machines and databases under its Sources tab. Pick
        which of them this schedule takes.
      </p>
    </div>

    <section class="pane-section">
      <span class="group-label group-label--lg">Files</span>
      <div class="pane-rows">
        <PaneRow
          title="Paths"
          help="where the paths come from"
        >
          <template #help>
            <strong>Each host's paths</strong> backs up what every host lists under Sources › Files.
            <strong>Only these paths</strong> replaces that list for this schedule, for example
            only <code>/home</code>. <strong>Off</strong> reads no files, for a schedule that only
            dumps databases or snapshots VMs. Each host's excludes apply whichever you pick.
          </template>
          <BaseSegmented
            v-model="form.files_mode"
            :options="FILES_OPTIONS"
            label="Paths"
          />
        </PaneRow>

        <div
          v-if="form.files_mode === 'custom'"
          class="pane-nest"
        >
          <div class="pane-rows">
            <PaneRow
              v-if="agentIds.length > 1"
              title="Different paths per host"
              help="splitting the paths by host"
            >
              <template #help>
                Give each host its own list instead of one list for every host.
              </template>
              <ToggleSwitch
                v-model="usePerHostPaths"
                label="Different paths per host"
              />
            </PaneRow>
            <PaneRow
              title="Only these paths"
              stack
            >
              <textarea
                v-if="!usePerHostPaths"
                v-model="form.backup_sources"
                class="input area-input"
                aria-label="Only these paths"
                placeholder="/home"
                spellcheck="false"
              />
              <PerAgentFields
                v-else
                :agent-ids="agentIds"
                :agent-label="agentLabel"
              >
                <template #default="{ agentId }">
                  <textarea
                    :value="perHostSources[agentId] ?? ''"
                    class="input area-input area-input-sm"
                    placeholder="Directories to back up, one per line"
                    spellcheck="false"
                    @input="
                      ($event) =>
                        (perHostSources[agentId] = ($event.target as HTMLTextAreaElement).value)
                    "
                  />
                </template>
              </PerAgentFields>
            </PaneRow>
          </div>
        </div>

        <PaneRow
          v-if="form.files_mode !== 'off'"
          title="Run each host's backup commands"
          help="the hosts' pre- and post-backup commands"
        >
          <template #help>
            The pre- and post-backup commands each host lists under Sources › Files, such as
            stopping a service so its files are consistent. Turn them off for a quick backup that
            does not need them, like an hourly /home.
          </template>
          <ToggleSwitch
            v-model="form.run_host_commands"
            label="Run each host's backup commands"
          />
        </PaneRow>
      </div>
    </section>

    <section class="pane-section">
      <span class="group-label group-label--lg">Virtual machines and databases</span>
      <div class="pane-rows">
        <PaneRow
          title="Virtual machines"
          help="VM snapshots"
        >
          <template #help>
            Snapshots the libvirt domains each host allows under Sources › Virtual machines, before
            the backup starts.
          </template>
          <ToggleSwitch
            v-model="form.vm_snapshot_enabled"
            label="Virtual machines"
          />
        </PaneRow>
        <PaneRow
          title="Databases"
          help="database dumps"
        >
          <template #help>
            Streams a dump of the databases each host allows under Sources › Databases into the same
            archive.
          </template>
          <ToggleSwitch
            v-model="form.database_backup_enabled"
            label="Databases"
          />
        </PaneRow>
        <DatabaseDumpMismatch
          v-if="form.database_backup_enabled && hostsWithDumpsOff.length > 0"
          :hosts="hostsWithDumpsOff"
        />
      </div>
    </section>

    <section
      v-if="preview.length > 0"
      class="pane-section"
    >
      <span class="group-label group-label--lg">What each host will back up</span>
      <div class="table-wrap">
        <table class="data-table data-table--compact">
          <thead>
            <tr>
              <th scope="col">Host</th>
              <th scope="col">Files</th>
              <th scope="col">Commands</th>
              <th scope="col">VMs</th>
              <th scope="col">Databases</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in preview"
              :key="row.id"
            >
              <td>
                <RouterLink :to="row.link">{{ row.label }}</RouterLink>
              </td>
              <td>
                <code
                  v-if="row.files.length > 0"
                  class="mono"
                  >{{ row.files.join(' ') }}</code
                >
                <span
                  v-if="row.filesNote"
                  class="muted"
                  >{{ row.files.length > 0 ? ` (${row.filesNote})` : row.filesNote }}</span
                >
              </td>
              <td :class="{ muted: row.commandsMuted }">
                {{ row.commands }}
              </td>
              <td :class="{ muted: !form.vm_snapshot_enabled }">{{ row.vms }}</td>
              <td :class="{ muted: !row.databasesOn }">{{ row.databases }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <span class="field-hint">
        Excludes and file change patterns from Advanced are added to each host's own.
      </span>
    </section>
  </div>
</template>

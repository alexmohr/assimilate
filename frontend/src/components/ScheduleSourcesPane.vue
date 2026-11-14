<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import FileSourcesEditor from './FileSourcesEditor.vue'
import SourceNotSetUp from './SourceNotSetUp.vue'
import PaneRow from './PaneRow.vue'
import PerAgentFields from './PerAgentFields.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import { useHostSources } from '../composables/useHostSources'
import { parseLines } from '../utils/validation'
import type { AgentRow } from '../types/agent'
import type { ScheduleFilesMode, ScheduleFormState } from '../types/scheduleForm'

/**
 * What a backup schedule takes from each of its hosts.
 *
 * Every host lists what it has under its own Sources tab: files, virtual
 * machines and databases. A schedule usually takes them as they are; this
 * pane is where one sets its own paths instead (fewer, like only /home, or
 * more), or leaves files out to back up only the databases. It replaces the backup paths that sat under Targets and the VM
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
  { value: 'host', label: "Each host's files" },
  { value: 'custom', label: 'Own files' },
  { value: 'off', label: 'Off' },
]

const targetAgents = computed(() =>
  props.agentIds
    .map((id) => props.agents.find((agent) => agent.id === id))
    .filter((agent): agent is AgentRow => agent !== undefined),
)

const { counts, hostsWithout } = useHostSources(() => targetAgents.value)
const hostsWithoutVms = computed(() => (form.value.vm_snapshot_enabled ? hostsWithout('vms') : []))
const hostsWithoutDatabases = computed(() =>
  form.value.database_backup_enabled ? hostsWithout('databases') : [],
)

/** "2 VMs", "None set up", or "Off" when this schedule leaves them out. */
interface SourceCell {
  text: string
  muted: boolean
  /** The schedule takes them, but the host has none set up. */
  missing: boolean
}

function sourceCell(on: boolean, count: number | undefined, unit: string): SourceCell {
  if (!on) return { text: 'Off', muted: true, missing: false }
  if (count === undefined) return { text: '…', muted: true, missing: false }
  if (count === 0) return { text: 'None set up', muted: false, missing: true }
  return { text: `${count} ${unit}`, muted: false, missing: false }
}

interface HostPreview {
  id: number
  label: string
  link: string
  files: string[]
  filesNote: string | null
  commands: string
  commandsMuted: boolean
  vms: SourceCell
  databases: SourceCell
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
  if (form.value.files_mode === 'off') return { text: 'None', muted: true }
  const own = form.value.files_mode === 'custom'
  if (!own && !form.value.run_host_commands) return { text: 'Skipped', muted: true }
  const pre = own ? form.value.pre_backup_commands.length : agent.default_pre_backup_commands.length
  const post = own
    ? form.value.post_backup_commands.length
    : agent.default_post_backup_commands.length
  return pre + post === 0
    ? { text: 'None', muted: true }
    : { text: `${pre} pre, ${post} post`, muted: false }
}

/** Mirrors what the server assembles for each host, so nobody has to guess. */
const preview = computed<HostPreview[]>(() =>
  targetAgents.value.map((agent) => {
    const files = filesFor(agent)
    const commands = commandsFor(agent)
    const hostCounts = counts.value.get(agent.id)
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
      vms: sourceCell(form.value.vm_snapshot_enabled, hostCounts?.vms, 'VMs'),
      databases: sourceCell(
        form.value.database_backup_enabled,
        hostCounts?.databases,
        'database sources',
      ),
    }
  }),
)
</script>

<template>
  <div class="form-stack">
    <div class="pane-head">
      <p class="pane-lede">
        Each host lists its own files, virtual machines and databases under its Sources tab. This
        schedule takes them as they are, or sets its own files.
      </p>
    </div>

    <section class="pane-section">
      <div class="pane-rows">
        <PaneRow
          title="Files"
          help="where the files come from"
        >
          <template #help>
            <strong>Each host's files</strong> backs up what every host sets under Sources › Files:
            its paths, excludes, file change patterns and commands. <strong>Own files</strong> sets
            all of those for this schedule instead, with the same fields. The paths can be
            narrower, such as only <code>/home</code>, or wider. <strong>Off</strong> reads no
            files, for a schedule that only dumps databases or snapshots VMs.
          </template>
          <BaseSegmented
            v-model="form.files_mode"
            :options="FILES_OPTIONS"
            label="Files"
          />
        </PaneRow>

        <PaneRow
          v-if="form.files_mode === 'host'"
          title="Run each host's backup commands"
          help="the hosts' pre- and post-backup commands"
        >
          <template #help>
            The pre- and post-backup commands each host lists under Sources › Files, such as
            stopping a service so its files are consistent. Turn them off for a backup that does
            not need them.
          </template>
          <ToggleSwitch
            v-model="form.run_host_commands"
            label="Run each host's backup commands"
          />
        </PaneRow>

        <PaneRow
          v-if="form.files_mode === 'custom' && agentIds.length > 1"
          title="Different paths per host"
          help="splitting the paths by host"
        >
          <template #help>
            Give each host its own paths. Excludes, patterns and commands stay shared.
          </template>
          <ToggleSwitch
            v-model="usePerHostPaths"
            label="Different paths per host"
          />
        </PaneRow>
      </div>

      <div
        v-if="form.files_mode === 'custom'"
        class="pane-nest own-files"
      >
        <FileSourcesEditor
          v-model:paths="form.backup_sources"
          v-model:excludes="form.exclude_patterns"
          v-model:includes="form.include_patterns"
          v-model:file-change-patterns="form.file_change_patterns"
          v-model:pre-commands="form.pre_backup_commands"
          v-model:post-commands="form.post_backup_commands"
          id-prefix="schedule-files"
          with-includes
          :default-timeout-seconds="form.hook_timeout_seconds"
        >
          <template
            v-if="usePerHostPaths"
            #paths
          >
            <PerAgentFields
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
          </template>
          <template #pathsHint>Used instead of each host's own files.</template>
        </FileSourcesEditor>
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
            Snapshots the libvirt domains each host includes under Sources › Virtual machines,
            before the backup starts. Turn it off to leave them out of this schedule.
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
            Streams a dump of the database sources each host includes under Sources › Databases
            into the same archive. Turn it off to leave them out of this schedule.
          </template>
          <ToggleSwitch
            v-model="form.database_backup_enabled"
            label="Databases"
          />
        </PaneRow>
        <SourceNotSetUp
          v-if="hostsWithoutVms.length > 0"
          kind="vms"
          :hosts="hostsWithoutVms"
        />
        <SourceNotSetUp
          v-if="hostsWithoutDatabases.length > 0"
          kind="databases"
          :hosts="hostsWithoutDatabases"
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
              <td :class="{ muted: row.vms.muted, 'source-missing': row.vms.missing }">
                {{ row.vms.text }}
              </td>
              <td
                :class="{
                  muted: row.databases.muted,
                  'source-missing': row.databases.missing,
                }"
              >
                {{ row.databases.text }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <span
        v-if="form.files_mode !== 'off'"
        class="field-hint"
      >
        {{
          form.files_mode === 'custom'
            ? "This schedule's own excludes, patterns and commands replace each host's."
            : "Each host's own excludes, patterns and commands apply, as set on its Sources tab."
        }}
        Global excludes apply unless Advanced turns them off.
      </span>
    </section>
  </div>
</template>

<style scoped>
.own-files {
  margin-top: var(--space-5);
}

.source-missing {
  color: var(--warning);
  font-weight: 600;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { getScheduleBackupSources } from '../api/schedules'
import { logger } from '../utils/logger'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

/**
 * Which schedules take what from this host. The host lists everything it
 * has; each schedule picks all of it or a slice (only /home, only the
 * databases), and this is the one place that shows those picks side by side.
 */
const props = defineProps<{
  agent: AgentRow
  schedules: readonly ScheduleRow[]
}>()

interface UsedByRow {
  id: number
  name: string
  files: string
  filesCustom: boolean
  filesOff: boolean
  vms: boolean
  databases: boolean
}

const rows = ref<UsedByRow[]>([])

async function load(): Promise<void> {
  const backups = props.schedules.filter((s) => s.schedule_type === 'backup')
  rows.value = await Promise.all(
    backups.map(async (s): Promise<UsedByRow> => {
      let custom: string[] = []
      try {
        const sources = await getScheduleBackupSources(s.id)
        const perHost = sources.backup_sources_per_agent.find((e) => e.agent_id === props.agent.id)
        custom = perHost?.paths ?? sources.backup_sources
      } catch (e: unknown) {
        logger.error('loading schedule sources failed', e)
      }
      const hostCount = props.agent.default_backup_paths.length
      const filesOff = s.files_mode === 'off'
      const filesCustom = !filesOff && custom.length > 0
      let files = `All ${hostCount} paths`
      if (filesOff) files = 'Off'
      else if (filesCustom) files = custom.join(' ')
      return {
        id: s.id,
        name: s.name || `Schedule ${s.id}`,
        files,
        filesCustom,
        filesOff,
        vms: s.vm_snapshot_enabled,
        databases: s.database_backup_enabled,
      }
    }),
  )
}

watch(() => [props.schedules, props.agent.id], load, { immediate: true })
</script>

<template>
  <section class="pane-section">
    <span class="group-label group-label--lg">Used by</span>
    <p
      v-if="rows.length === 0"
      class="muted"
    >
      No backup schedule targets this host yet.
    </p>
    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table data-table--compact">
        <thead>
          <tr>
            <th scope="col">Schedule</th>
            <th scope="col">Files</th>
            <th scope="col">VMs</th>
            <th scope="col">Databases</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in rows"
            :key="row.id"
          >
            <td>
              <RouterLink :to="`/schedules/${row.id}?tab=settings&section=sources`">{{
                row.name
              }}</RouterLink>
            </td>
            <td>
              <code
                v-if="row.filesCustom"
                class="mono"
                >{{ row.files }}</code
              >
              <span
                v-else
                :class="{ muted: row.filesOff }"
                >{{ row.files }}</span
              >
            </td>
            <td :class="{ muted: !row.vms }">{{ row.vms ? 'Yes' : 'Off' }}</td>
            <td :class="{ muted: !row.databases }">{{ row.databases ? 'Yes' : 'Off' }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

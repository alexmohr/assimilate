<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { getScheduleBackupSources } from '../api/schedules'
import { logger } from '../utils/logger'
import { useAsyncAction } from '../composables/useAsyncAction'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'

/**
 * Which schedules take what from this host. The host lists everything it
 * has; each schedule takes its files as they are, sets its own paths (fewer
 * or more), or leaves files out, and this shows those choices side by side.
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

/** "All 3 paths", "1 path", or that the host lists none. */
function hostPathsLabel(): string {
  const count = (props.agent.default_backup_paths ?? []).length
  if (count === 0) return 'No paths set on this host'
  return count === 1 ? "The host's 1 path" : `All ${count} paths`
}

/** The paths a schedule with its own files reads here: this host's own split, else the shared list. */
async function ownPaths(scheduleId: number): Promise<string[] | null> {
  try {
    const sources = await getScheduleBackupSources(scheduleId)
    const perHost = sources.backup_sources_per_agent.find((e) => e.agent_id === props.agent.id)
    return perHost?.paths ?? sources.backup_sources
  } catch (e: unknown) {
    logger.error('loading schedule sources failed', e)
    return null
  }
}

async function rowFor(s: ScheduleRow): Promise<UsedByRow> {
  const base = {
    id: s.id,
    name: s.name || `Schedule ${s.id}`,
    vms: s.vm_snapshot_enabled,
    databases: s.database_backup_enabled,
  }
  if (s.files_mode === 'off') {
    return { ...base, files: 'Off', filesCustom: false, filesOff: true }
  }
  if (s.files_mode === 'host') {
    return { ...base, files: hostPathsLabel(), filesCustom: false, filesOff: false }
  }
  const paths = await ownPaths(s.id)
  if (paths === null) {
    return {
      ...base,
      files: 'Own paths (could not be loaded)',
      filesCustom: false,
      filesOff: false,
    }
  }
  return paths.length > 0
    ? { ...base, files: paths.join(' '), filesCustom: true, filesOff: false }
    : { ...base, files: 'Own files, no paths', filesCustom: false, filesOff: false }
}

const { runLatest } = useAsyncAction('loading the schedules using this host')

/** A later load (another host, or new schedules) supersedes one in flight. */
async function load(): Promise<void> {
  await runLatest(async (isCurrent) => {
    const backups = props.schedules.filter((s) => s.schedule_type === 'backup')
    const next = await Promise.all(backups.map(rowFor))
    if (isCurrent()) rows.value = next
  })
}

watch(() => [props.schedules, props.agent.id], load, { immediate: true })
</script>

<template>
  <section class="pane-section">
    <div class="pane-head">
      <p class="pane-lede">
        The backup schedules that target this host, and what each takes from it.
      </p>
    </div>
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

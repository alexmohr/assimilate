<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { listRepoStats } from '../api/repos'
import { logger } from '../utils/logger'
import type { RepoWithStats } from '../types/repo'

/**
 * Where a schedule's retention actually runs, per target repository. The
 * policy is edited once on the schedule, but an append-only repository's
 * agents cannot prune: the server does it during that repository's
 * maintenance, so a schedule writing to a mix needs this spelled out.
 */
const props = defineProps<{
  repoIds: readonly number[]
}>()

const stats = ref<RepoWithStats[]>([])

const targets = computed<RepoWithStats[]>(() =>
  props.repoIds.flatMap((id) => stats.value.filter((r) => r.id === id)),
)

async function load(): Promise<void> {
  try {
    stats.value = await listRepoStats()
  } catch (e: unknown) {
    // A hint, not part of the form: without it the table is simply absent.
    logger.error(e)
  }
}

onMounted(load)
// A repository added to the schedule that the first load did not know of.
watch(
  () => props.repoIds.filter((id) => !stats.value.some((r) => r.id === id)).length,
  (missing) => {
    if (missing > 0) void load()
  },
)
</script>

<template>
  <section
    v-if="targets.length > 0"
    class="pane-section"
  >
    <span class="group-label">Where retention runs</span>
    <div class="table-wrap">
      <table class="data-table">
        <thead>
          <tr>
            <th>Repository</th>
            <th>Prune and compact</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="target in targets"
            :key="target.id"
          >
            <td class="cell-mono">
              {{ target.name }}
              <span
                v-if="target.append_only"
                class="badge badge--violet"
              >
                Append-only
              </span>
            </td>
            <td>
              {{
                target.append_only
                  ? 'At repository maintenance, by the server'
                  : 'After each backup, by the agent'
              }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

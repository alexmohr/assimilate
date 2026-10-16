<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Database, RotateCcw } from '@lucide/vue'
import { listArchiveDatabases } from '../api/archives'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ArchiveDatabaseDump } from '../types/generated'
import { resolveArchiveDomain, resolveArchiveHost } from '../utils/archiveHost'
import { dumpPlace } from '../utils/databaseRestore'
import { KIND_LABELS, RUNTIME_LABELS, sourceName } from '../utils/databaseRuntimes'
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'
import DatabaseRestoreDialog from './DatabaseRestoreDialog.vue'
import EmptyState from './EmptyState.vue'

/**
 * The database dumps an archive holds, read from its manifests when this
 * view opens, each with a way to restore it.
 */
const props = withDefaults(
  defineProps<{
    repoId: number
    archive: ArchiveEntry
    /** Gates the restore action, as it gates file restores. */
    canRestore?: boolean
  }>(),
  { canRestore: false },
)

const dumps = ref<ArchiveDatabaseDump[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const restoring = ref<ArchiveDatabaseDump | null>(null)

const archiveHost = computed<string>(() => resolveArchiveHost(props.archive))
const archiveDomain = computed<string | null>(() => resolveArchiveDomain(props.archive))

// Archives can be switched while a listing is in flight; only the latest lands.
let loadSeq = 0

async function load(): Promise<void> {
  const seq = ++loadSeq
  loading.value = true
  error.value = null
  try {
    const listed = await listArchiveDatabases(props.repoId, props.archive.name)
    if (seq === loadSeq) dumps.value = listed
  } catch (e: unknown) {
    if (seq === loadSeq) error.value = extractError(e)
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

watch(
  () => [props.repoId, props.archive.name],
  () => {
    dumps.value = []
    void load()
  },
  { immediate: true },
)
</script>

<template>
  <div class="archive-db-dumps">
    <div
      v-if="loading"
      class="loading-row"
    >
      <BaseSpinner
        size="md"
        label="Reading the archive's database dumps"
      />
    </div>
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="dumps.length === 0"
      :icon="Database"
      title="No database dumps"
      description="This archive holds no database dumps."
    />
    <div
      v-else
      class="table-wrap"
    >
      <table
        class="data-table"
        data-testid="archive-database-dumps"
      >
        <thead>
          <tr>
            <th>Database</th>
            <th>Source</th>
            <th v-if="canRestore" />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="dump in dumps"
            :key="dump.archive_path"
          >
            <td class="cell-mono">{{ dump.database }}</td>
            <td>
              <div class="cell-mono nowrap">{{ sourceName(dump) }}</div>
              <div class="cell-muted">
                {{
                  [RUNTIME_LABELS[dump.runtime], KIND_LABELS[dump.kind], dumpPlace(dump)]
                    .filter(Boolean)
                    .join(' · ')
                }}
              </div>
            </td>
            <td v-if="canRestore">
              <button
                type="button"
                class="btn btn-sm btn-ghost nowrap"
                :title="`Restore ${dump.database}`"
                @click="restoring = dump"
              >
                <RotateCcw :size="14" />
                Restore
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <DatabaseRestoreDialog
      :open="restoring !== null"
      :repo-id="repoId"
      :archive-name="archive.name"
      :dump="restoring"
      :default-host="archiveHost"
      :default-domain="archiveDomain"
      @close="restoring = null"
    />
  </div>
</template>

<style scoped>
.archive-db-dumps {
  padding: var(--space-5) var(--space-6);
}

.nowrap {
  white-space: nowrap;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Database } from '@lucide/vue'
import { listArchiveDatabases } from '../api/archives'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ArchiveDatabaseDump } from '../types/generated'
import { resolveArchiveDomain, resolveArchiveHost } from '../utils/archiveHost'
import { dumpPlace } from '../utils/databaseRestore'
import { KIND_LABELS, RUNTIME_LABELS, sourceName } from '../utils/databaseRuntimes'
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'
import BrowserRow from './BrowserRow.vue'
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

/** Where the dump came from, after its source's name: runtime, kind and place. */
function dumpFacts(dump: ArchiveDatabaseDump): string[] {
  return [RUNTIME_LABELS[dump.runtime], KIND_LABELS[dump.kind], dumpPlace(dump)].filter(
    (fact): fact is string => typeof fact === 'string' && fact.length > 0,
  )
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
  <div>
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
    <template v-else>
      <div data-testid="archive-database-dumps">
        <BrowserRow
          v-for="dump in dumps"
          :key="dump.archive_path"
          :title="dump.database"
          data-testid="database-dump"
        >
          <template #lead>
            <Database
              :size="16"
              class="db-dump-icon"
            />
          </template>
          <template #facts>
            <span>{{ sourceName(dump) }}</span>
            <template
              v-for="fact in dumpFacts(dump)"
              :key="fact"
            >
              <span aria-hidden="true">&middot;</span>
              <span>{{ fact }}</span>
            </template>
          </template>
          <template
            v-if="canRestore"
            #end
          >
            <button
              type="button"
              class="btn btn-sm btn-ghost db-dump-restore"
              :title="`Restore ${dump.database}`"
              @click="restoring = dump"
            >
              Restore...
            </button>
          </template>
        </BrowserRow>
      </div>
      <p class="db-dump-note">
        The agent dumps each database just before the backup starts and stores the dump inside this
        archive.
      </p>
    </template>

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
.db-dump-icon {
  flex: none;
  color: var(--text-muted);
}

.db-dump-restore {
  flex: none;
}

.db-dump-note {
  margin: 0;
  padding: var(--space-4) var(--space-7);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { getDatabaseStorage } from '../api/system'
import type { DatabaseStorageResponse } from '../api/system'
import { extractError } from '../utils/error'
import { formatBytes } from '../utils/format'
import BaseSpinner from '../components/BaseSpinner.vue'

/**
 * Where the PostgreSQL disk allocation goes, table by table. It used to sit
 * between the settings form and config export on the System page, where a
 * long table of storage figures pushed the settings out of view; it is a
 * diagnostic to visit, not a setting, so it has a page of its own.
 */
const databaseStorage = ref<DatabaseStorageResponse | null>(null)
const databaseStorageLoading = ref(true)
const databaseStorageError = ref('')

async function loadDatabaseStorage(): Promise<void> {
  databaseStorageLoading.value = true
  databaseStorageError.value = ''
  try {
    databaseStorage.value = await getDatabaseStorage()
  } catch (e: unknown) {
    databaseStorageError.value = extractError(e, 'Failed to load database storage')
  } finally {
    databaseStorageLoading.value = false
  }
}

function storagePercent(bytes: number): number {
  const total = databaseStorage.value?.database_bytes ?? 0
  return total > 0 ? (bytes / total) * 100 : 0
}

onMounted(loadDatabaseStorage)
</script>

<template>
  <div>
    <div class="page-header">
      <h1 class="page-title">Database Storage</h1>
      <div class="header-actions">
        <button
          class="btn btn-sm btn-ghost"
          :disabled="databaseStorageLoading"
          @click="loadDatabaseStorage"
        >
          {{ databaseStorageLoading ? 'Loading...' : 'Refresh' }}
        </button>
      </div>
    </div>

    <p class="page-description">
      PostgreSQL allocation by application table, including table data, indexes, and TOAST data.
    </p>

    <div class="panel">
      <BaseSpinner
        v-if="databaseStorageLoading"
        size="lg"
      />
      <div
        v-else-if="databaseStorageError"
        class="state-msg state-msg--inline state-error"
      >
        {{ databaseStorageError }}
      </div>
      <template v-else-if="databaseStorage">
        <div class="database-total">
          <span>Total database size</span>
          <strong>{{ formatBytes(databaseStorage.database_bytes) }}</strong>
        </div>
        <div class="table-wrap">
          <table class="data-table data-table--compact">
            <thead>
              <tr>
                <th>Table</th>
                <th>Table data</th>
                <th>Indexes</th>
                <th>TOAST</th>
                <th>Total</th>
                <th>Share</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="relation in databaseStorage.relations"
                :key="relation.table_name"
              >
                <td class="storage-name">{{ relation.table_name }}</td>
                <td>{{ formatBytes(relation.table_bytes) }}</td>
                <td>{{ formatBytes(relation.index_bytes) }}</td>
                <td>{{ formatBytes(relation.toast_bytes) }}</td>
                <td class="storage-total">{{ formatBytes(relation.total_bytes) }}</td>
                <td class="storage-share">
                  <div class="storage-share-value">
                    {{ storagePercent(relation.total_bytes).toFixed(1) }}%
                  </div>
                  <div class="progress-track">
                    <div
                      class="progress-bar"
                      :style="{ width: `${storagePercent(relation.total_bytes)}%` }"
                    ></div>
                  </div>
                </td>
              </tr>
              <tr v-if="databaseStorage.other_bytes > 0">
                <td class="storage-name">Other PostgreSQL storage</td>
                <td colspan="3">System catalogs and database overhead</td>
                <td class="storage-total">{{ formatBytes(databaseStorage.other_bytes) }}</td>
                <td class="storage-share">
                  <div class="storage-share-value">
                    {{ storagePercent(databaseStorage.other_bytes).toFixed(1) }}%
                  </div>
                  <div class="progress-track">
                    <div
                      class="progress-bar progress-bar--muted"
                      :style="{ width: `${storagePercent(databaseStorage.other_bytes)}%` }"
                    ></div>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.database-total {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-6);
  margin-bottom: var(--space-6);
  color: var(--text-secondary);
  font-size: var(--fs-base);
}

.database-total strong {
  color: var(--text-primary);
  font-size: var(--fs-lg);
}

.storage-name {
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.storage-total {
  color: var(--text-primary);
  font-weight: 600;
}

.storage-share {
  min-width: 90px;
}

.storage-share-value {
  margin-bottom: var(--space-2);
}
</style>

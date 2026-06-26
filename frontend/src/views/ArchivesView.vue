<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { FilterMatchMode } from '@primevue/core/api'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import ArchiveFileBrowser from '../components/ArchiveFileBrowser.vue'
import RestoreWizard from '../components/RestoreWizard.vue'
import ArchiveDiff from '../components/ArchiveDiff.vue'
import FileSearch from '../components/FileSearch.vue'
import BaseHostLink from '../components/BaseHostLink.vue'

interface RepoOption {
  id: number
  hostname: string
  target_name: string
  enabled: boolean
}

interface ArchiveEntry {
  name: string
  start: string
  hostname: string
  comment: string
  original_size: number
  deduplicated_size: number
  matched: boolean | null
  agent_hostname: string | null
}

const repos = ref<RepoOption[]>([])
const reposLoading = ref(false)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchive = ref<ArchiveEntry | null>(null)

const showPassphraseDialog = ref(false)

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const archiveFilters = ref({
  name: { value: '', matchMode: FilterMatchMode.CONTAINS },
  start: { value: '', matchMode: FilterMatchMode.CONTAINS },
  hostname: { value: '', matchMode: FilterMatchMode.CONTAINS },
  original_size: { value: '', matchMode: FilterMatchMode.CONTAINS },
})

async function loadRepos(): Promise<void> {
  reposLoading.value = true
  reposError.value = null
  try {
    const res = await apiClient.get<RepoOption[]>('/repos')
    repos.value = res.data
  } catch (e: unknown) {
    reposError.value = extractError(e)
  } finally {
    reposLoading.value = false
  }
}

async function onRepoChange(): Promise<void> {
  archives.value = []
  selectedArchive.value = null
  archivesError.value = null
  if (selectedRepoId.value === null) return
  await loadArchives()
}

async function loadArchives(): Promise<void> {
  if (selectedRepoId.value === null) return
  archivesLoading.value = true
  archivesError.value = null
  try {
    const res = await apiClient.get<ArchiveEntry[]>(`/repos/${selectedRepoId.value}/archives`)
    archives.value = res.data
  } catch (e: unknown) {
    archivesError.value = extractError(e)
  } finally {
    archivesLoading.value = false
  }
}

function selectArchive(archive: ArchiveEntry): void {
  selectedArchive.value = archive
}

const showRestoreWizard = ref(false)
const showArchiveDiff = ref(false)

const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showPassphraseDialog, () => {
  showPassphraseDialog.value = false
})

async function revealPassphrase(): Promise<void> {
  if (selectedRepoId.value === null) return
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(
      `/repos/${selectedRepoId.value}/passphrase`,
    )
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

onMounted(loadRepos)
</script>

<template>
  <div class="archives-view">
    <div class="page-header">
      <h1 class="page-title">Archives</h1>
    </div>

    <div
      v-if="reposLoading"
      class="state-msg"
    >
      Loading repositories...
    </div>
    <div
      v-else-if="reposError"
      class="state-msg state-error"
    >
      {{ reposError }}
    </div>
    <template v-else>
      <div class="repo-selector">
        <label class="selector-label">Repository</label>
        <select
          v-model="selectedRepoId"
          class="select-input"
          @change="onRepoChange"
        >
          <option
            :value="null"
            disabled
          >
            — select a repository —
          </option>
          <option
            v-for="repo in repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.hostname }} / {{ repo.target_name }}
          </option>
        </select>
        <span
          v-if="repos.length === 0"
          class="muted-hint"
          >No repositories configured yet.</span
        >
        <button
          v-if="selectedRepoId !== null"
          class="btn btn-sm btn-ghost passphrase-btn"
          :disabled="passphraseLoading"
          @click="revealPassphrase"
        >
          {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
        </button>
      </div>

      <div
        v-if="selectedRepoId !== null"
        class="main-layout"
      >
        <!-- Archive list -->
        <div class="panel archives-panel">
          <div class="panel-header">
            <span class="panel-title">Archives</span>
            <div class="panel-actions">
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archives.length < 1"
                @click="showRestoreWizard = true"
              >
                Restore
              </button>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archives.length < 2"
                @click="showArchiveDiff = true"
              >
                Diff
              </button>
              <button
                class="btn btn-sm btn-ghost"
                :disabled="archivesLoading"
                @click="loadArchives"
              >
                {{ archivesLoading ? '...' : '&#8635;' }}
              </button>
            </div>
          </div>

          <div
            v-if="archivesLoading"
            class="state-msg"
          >
            <span class="spinner" />
            Loading archives...
          </div>
          <div
            v-else-if="archivesError"
            class="state-msg state-error"
          >
            {{ archivesError }}
          </div>
          <div
            v-else-if="archives.length === 0"
            class="state-msg"
          >
            No archives found.
          </div>
          <DataTable
            v-else
            v-model:filters="archiveFilters"
            :value="sortedArchives"
            :row-class="
              (data: ArchiveEntry) =>
                selectedArchive?.name === data.name ? 'selected clickable' : 'clickable'
            "
            filter-display="row"
            table-class="data-table"
            @row-click="(e: { data: ArchiveEntry }) => selectArchive(e.data)"
          >
            <Column
              field="name"
              header="Name"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-mono">{{ data.name }}</span>
              </template>
            </Column>
            <Column
              field="start"
              header="Date"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-date">{{ formatDate(data.start) }}</span>
              </template>
            </Column>
            <Column
              field="hostname"
              header="Host"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <BaseHostLink
                  v-if="data.matched === true && data.agent_hostname"
                  :hostname="data.agent_hostname"
                  class="host-link"
                  @click.stop
                />
                <BaseHostLink
                  v-else-if="data.matched !== true"
                  :hostname="data.hostname"
                  class="unmatched-host-link"
                  @click.stop
                />
                <span
                  v-else
                  class="td-host"
                  >{{ data.hostname }}</span
                >
              </template>
            </Column>
            <Column
              field="matched"
              header=""
              style="width: 3rem"
            >
              <template #body="{ data }">
                <span
                  v-if="data.matched === true"
                  class="match-icon match-ok"
                  title="Matched"
                  >&#10003;</span
                >
                <span
                  v-else-if="data.matched !== true"
                  class="match-icon match-warn"
                  title="Unmatched"
                  >&#9888;</span
                >
              </template>
            </Column>
            <Column
              field="original_size"
              header="Size"
              :sortable="true"
              :show-filter-menu="false"
            >
              <template #filter="{ filterModel, filterCallback }">
                <input
                  v-model="filterModel.value"
                  class="filter-input"
                  type="text"
                  placeholder="Filter..."
                  @input="filterCallback()"
                />
              </template>
              <template #body="{ data }">
                <span class="td-size">{{ formatBytes(data.original_size) }}</span>
              </template>
            </Column>
          </DataTable>
        </div>

        <!-- File browser -->
        <ArchiveFileBrowser
          :repo-id="selectedRepoId"
          :archive-name="selectedArchive?.name ?? null"
        />
      </div>

      <FileSearch
        :repo-id="selectedRepoId"
        :archives="archives.map((a) => ({ name: a.name }))"
      />
    </template>

    <!-- Passphrase Dialog -->
    <Teleport to="body">
      <div
        v-if="showPassphraseDialog"
        class="overlay"
        @click.self="showPassphraseDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">
              {{ passphrase ? 'Repository Passphrase' : 'Error' }}
            </h2>
            <button
              class="close-btn"
              @click="showPassphraseDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <template v-if="passphrase">
              <p class="passphrase-warning">Keep this passphrase secure. Do not share it.</p>
              <div class="passphrase-box">
                <code class="passphrase-text">{{ passphrase }}</code>
                <button
                  class="btn btn-sm btn-ghost"
                  @click="passphrase && copyToClipboard(passphrase)"
                >
                  {{ passphraseCopied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
            </template>
            <div
              v-else-if="passphraseError"
              class="form-error"
            >
              {{ passphraseError }}
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showPassphraseDialog = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>
    <!-- Restore Wizard -->
    <RestoreWizard
      :open="showRestoreWizard"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showRestoreWizard = false"
    />

    <!-- Archive Diff -->
    <ArchiveDiff
      :open="showArchiveDiff"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showArchiveDiff = false"
    />
  </div>
</template>

<style scoped>
.archives-view {
  max-width: 1300px;
  color: var(--text-primary);
}

.state-msg {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 1.5rem;
  color: var(--text-muted);
  font-size: 0.875rem;
}

.state-error {
  color: var(--danger);
}

.repo-selector {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.selector-label {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.select-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.55rem 0.75rem;
  font-size: 0.875rem;
  min-width: 280px;
  transition: border-color 0.15s;
}

.select-input:focus {
  outline: none;
  border-color: var(--accent);
}

.muted-hint {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.main-layout {
  display: grid;
  grid-template-columns: 480px 1fr;
  gap: 1rem;
  align-items: start;
}

.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
}

.panel-actions {
  display: flex;
  gap: 0.25rem;
}

.panel-title {
  font-size: 0.8rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.data-table th {
  text-align: left;
  padding: 0.5rem 1rem;
  color: var(--text-muted);
  font-weight: 600;
  font-size: 0.75rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid var(--border);
}

.data-table td {
  padding: 0.6rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.data-table tr:last-child td {
  border-bottom: none;
}

.data-table tr.clickable {
  cursor: pointer;
  transition: background 0.1s;
}

.data-table tr.clickable:hover {
  background: var(--bg-hover);
}

.data-table tr.selected td {
  background: var(--accent-subtle);
  color: var(--text-primary);
}

.td-mono {
  font-family: var(--mono);
  font-size: 0.8rem;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.td-date {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.td-host {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.host-link {
  font-size: 0.8rem;
  color: var(--accent);
  text-decoration: none;
}

.host-link:hover {
  text-decoration: underline;
}

.unmatched-host-link {
  font-size: 0.8rem;
  color: var(--warning);
  text-decoration: none;
}

.unmatched-host-link:hover {
  text-decoration: underline;
}

.match-icon {
  font-size: 0.9rem;
}

.match-ok {
  color: var(--success, #22c55e);
}

.match-warn {
  color: var(--warning);
}

.td-size {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.filter-input {
  width: 100%;
  background: var(--bg-input, var(--bg-card));
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.35rem 0.5rem;
  font-size: 0.78rem;
}

.filter-input:focus {
  outline: none;
  border-color: var(--accent);
}

.spinner {
  display: inline-block;
  width: 1rem;
  height: 1rem;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* Buttons */

.passphrase-btn {
  margin-left: auto;
}

.passphrase-warning {
  color: var(--warning);
  font-size: 0.875rem;
  font-weight: 500;
  margin-bottom: 0.75rem;
}

.passphrase-box {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem 1rem;
}

.passphrase-text {
  flex: 1;
  font-family: var(--mono);
  font-size: 0.82rem;
  color: var(--text-primary);
  word-break: break-all;
  background: transparent;
  padding: 0;
}
</style>

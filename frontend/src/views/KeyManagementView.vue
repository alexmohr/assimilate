<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useClipboard } from '../composables/useClipboard'
import { extractError } from '../utils/error'
import BaseSpinner from '../components/BaseSpinner.vue'

interface Repo {
  id: number
  name: string
  path: string
}

const repos = ref<Repo[]>([])
const reposLoading = ref(true)
const reposError = ref('')

const selectedRepoId = ref<number | null>(null)

const exportedKey = ref('')
const exportLoading = ref(false)
const exportError = ref('')

const importKeyData = ref('')
const importLoading = ref(false)
const importError = ref('')
const importSuccess = ref(false)

const { copied, copy: copyToClipboard } = useClipboard()

onMounted(async (): Promise<void> => {
  try {
    const res = await apiClient.get<Repo[]>('/repos')
    repos.value = res.data
    if (res.data.length > 0) {
      selectedRepoId.value = res.data[0].id
    }
  } catch (e: unknown) {
    reposError.value = extractError(e, 'Failed to load repositories')
  } finally {
    reposLoading.value = false
  }
})

async function exportKey(): Promise<void> {
  if (selectedRepoId.value === null) return
  exportLoading.value = true
  exportError.value = ''
  exportedKey.value = ''
  try {
    const res = await apiClient.post<string>(
      `/repos/${String(selectedRepoId.value)}/key/export`,
      null,
      { responseType: 'text', transformResponse: [(data: unknown) => data] },
    )
    exportedKey.value = res.data
  } catch (e: unknown) {
    exportError.value = extractError(e, 'Failed to export key')
  } finally {
    exportLoading.value = false
  }
}

async function importKey(): Promise<void> {
  if (selectedRepoId.value === null || !importKeyData.value.trim()) return
  importLoading.value = true
  importError.value = ''
  importSuccess.value = false
  try {
    await apiClient.post(`/repos/${String(selectedRepoId.value)}/key/import`, {
      key_data: importKeyData.value,
    })
    importSuccess.value = true
    importKeyData.value = ''
    setTimeout((): void => {
      importSuccess.value = false
    }, 3000)
  } catch (e: unknown) {
    importError.value = extractError(e, 'Failed to import key')
  } finally {
    importLoading.value = false
  }
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">Key Management</h1>
    </div>

    <BaseSpinner
      v-if="reposLoading"
      size="lg"
    />
    <div
      v-else-if="reposError"
      class="state-msg error"
    >
      {{ reposError }}
    </div>
    <template v-else>
      <div class="info-card">
        <div class="card-header">
          <h3 class="info-title">Repository</h3>
        </div>
        <p class="info-description">
          Select a repository to manage its encryption key.
        </p>
        <select
          v-model="selectedRepoId"
          class="form-input repo-select"
        >
          <option
            v-for="repo in repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.name }} — {{ repo.path }}
          </option>
        </select>
      </div>

      <div class="info-card">
        <div class="card-header">
          <h3 class="info-title">Export Key</h3>
          <button
            class="btn btn-sm btn-primary"
            :disabled="exportLoading || selectedRepoId === null"
            @click="exportKey"
          >
            {{ exportLoading ? 'Exporting…' : 'Export' }}
          </button>
        </div>
        <p class="info-description">
          Export the borg encryption key for the selected repository. Store it safely as a backup.
        </p>

        <div
          v-if="exportError"
          class="state-msg error"
        >
          {{ exportError }}
        </div>
        <div
          v-if="exportedKey"
          class="key-box"
        >
          <pre class="key-text">{{ exportedKey }}</pre>
          <button
            class="btn btn-sm btn-ghost"
            @click="copyToClipboard(exportedKey)"
          >
            {{ copied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </div>

      <div class="info-card">
        <div class="card-header">
          <h3 class="info-title">Import Key</h3>
        </div>
        <p class="info-description">
          Paste a previously exported borg key to import it into the selected repository.
        </p>

        <div class="import-form">
          <textarea
            v-model="importKeyData"
            class="form-input key-textarea"
            placeholder="Paste borg key data here…"
            rows="8"
          />
          <div class="import-actions">
            <button
              class="btn btn-primary"
              :disabled="importLoading || !importKeyData.trim() || selectedRepoId === null"
              @click="importKey"
            >
              {{ importLoading ? 'Importing…' : 'Import Key' }}
            </button>
            <span
              v-if="importSuccess"
              class="save-success"
            >
              Key imported successfully
            </span>
          </div>
          <div
            v-if="importError"
            class="state-msg error"
          >
            {{ importError }}
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.page {
  max-width: 800px;
}

.page-header {
  margin-bottom: 1.5rem;
}

.page-title {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--text-primary);
}

.info-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.5rem;

  & + & {
    margin-top: 0.75rem;
  }
}

.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.info-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
}

.info-description {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin-bottom: 1rem;
}

.state-msg {
  font-size: 0.875rem;
  color: var(--text-muted);
}

.state-msg.error {
  color: var(--danger);
}

.repo-select {
  width: 100%;
  max-width: 400px;
}

.key-box {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 1rem;
}

.key-text {
  flex: 1;
  font-family: var(--font-mono);
  font-size: 0.75rem;
  line-height: 1.5;
  color: var(--text-primary);
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0;
}

.import-form {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.key-textarea {
  width: 100%;
  font-family: var(--font-mono);
  font-size: 0.75rem;
  line-height: 1.5;
  resize: vertical;
}

.import-actions {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.save-success {
  font-size: 0.875rem;
  color: var(--success);
  font-weight: 500;
}
</style>

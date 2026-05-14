<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { formatBytes, formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import BaseSpinner from '../components/BaseSpinner.vue'

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
}

interface ContentEntry {
  type: string
  path: string
  size: number
  mtime: string
  mode: string
}

interface BreadcrumbSegment {
  label: string
  path: string
}

const repos = ref<RepoOption[]>([])
const reposLoading = ref(false)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchive = ref<ArchiveEntry | null>(null)

const currentPath = ref('/')
const contents = ref<ContentEntry[]>([])
const contentsLoading = ref(false)
const contentsError = ref<string | null>(null)
const showPassphraseDialog = ref(false)

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const breadcrumbs = computed<BreadcrumbSegment[]>(() => {
  const path = currentPath.value
  if (path === '/') return [{ label: '/', path: '/' }]
  const parts = path.replace(/^\//, '').split('/')
  const segments: BreadcrumbSegment[] = [{ label: '/', path: '/' }]
  let accumulated = ''
  for (const part of parts) {
    accumulated += `/${part}`
    segments.push({ label: part, path: accumulated })
  }
  return segments
})

const dirs = computed(() =>
  contents.value.filter((e) => e.type === 'd').sort((a, b) => a.path.localeCompare(b.path)),
)

const files = computed(() =>
  contents.value.filter((e) => e.type !== 'd').sort((a, b) => a.path.localeCompare(b.path)),
)

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
  contents.value = []
  currentPath.value = '/'
  archivesError.value = null
  contentsError.value = null
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

async function selectArchive(archive: ArchiveEntry): Promise<void> {
  selectedArchive.value = archive
  currentPath.value = '/'
  contents.value = []
  contentsError.value = null
  await loadContents('/')
}

async function loadContents(path: string): Promise<void> {
  if (selectedRepoId.value === null || !selectedArchive.value) return
  contentsLoading.value = true
  contentsError.value = null
  currentPath.value = path
  try {
    const res = await apiClient.get<ContentEntry[]>(
      `/repos/${selectedRepoId.value}/archives/${encodeURIComponent(selectedArchive.value.name)}/contents`,
      { params: { path } },
    )
    contents.value = res.data
  } catch (e: unknown) {
    contentsError.value = extractError(e)
  } finally {
    contentsLoading.value = false
  }
}

function navigateTo(path: string): void {
  loadContents(path)
}

function entryName(entry: ContentEntry): string {
  return entry.path.split('/').pop() ?? entry.path
}

function downloadFile(entry: ContentEntry): void {
  if (selectedRepoId.value === null || !selectedArchive.value) return
  const url = `/api/repos/${selectedRepoId.value}/archives/${encodeURIComponent(selectedArchive.value.name)}/extract?path=${encodeURIComponent(entry.path)}`
  const a = document.createElement('a')
  a.href = url
  a.download = entryName(entry)
  document.body.appendChild(a)
  a.click()
  document.body.removeChild(a)
}

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
            <button
              class="btn btn-sm btn-ghost"
              :disabled="archivesLoading"
              @click="loadArchives"
            >
              {{ archivesLoading ? '...' : '&#8635;' }}
            </button>
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
          <table
            v-else
            class="data-table"
          >
            <thead>
              <tr>
                <th>Name</th>
                <th>Date</th>
                <th>Host</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="archive in sortedArchives"
                :key="archive.name"
                :class="['clickable', { selected: selectedArchive?.name === archive.name }]"
                @click="selectArchive(archive)"
              >
                <td class="td-mono">
                  {{ archive.name }}
                </td>
                <td class="td-date">
                  {{ formatDate(archive.start) }}
                </td>
                <td class="td-host">
                  {{ archive.hostname }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <!-- File browser -->
        <div
          v-if="selectedArchive"
          class="panel browser-panel"
        >
          <div class="panel-header">
            <span class="panel-title">Files — {{ selectedArchive.name }}</span>
          </div>

          <div class="breadcrumb">
            <button
              v-for="(seg, i) in breadcrumbs"
              :key="seg.path"
              class="crumb"
              :class="{ 'crumb-last': i === breadcrumbs.length - 1 }"
              @click="navigateTo(seg.path)"
            >
              {{ seg.label }}
            </button>
          </div>

          <BaseSpinner
            v-if="contentsLoading"
            size="sm"
          />
          <div
            v-else-if="contentsError"
            class="state-msg state-error"
          >
            {{ contentsError }}
          </div>
          <div
            v-else-if="contents.length === 0"
            class="state-msg"
          >
            Empty directory.
          </div>
          <table
            v-else
            class="data-table"
          >
            <thead>
              <tr>
                <th>Name</th>
                <th>Size</th>
                <th>Modified</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="entry in dirs"
                :key="entry.path"
                class="clickable"
                @click="navigateTo(entry.path)"
              >
                <td class="td-name">
                  <span class="icon-dir">&#128193;</span>
                  {{ entryName(entry) }}
                </td>
                <td class="td-size muted">—</td>
                <td class="td-date">
                  {{ formatDate(entry.mtime) }}
                </td>
                <td />
              </tr>
              <tr
                v-for="entry in files"
                :key="entry.path"
              >
                <td class="td-name">
                  <span class="icon-file">&#128196;</span>
                  {{ entryName(entry) }}
                </td>
                <td class="td-size">
                  {{ formatBytes(entry.size) }}
                </td>
                <td class="td-date">
                  {{ formatDate(entry.mtime) }}
                </td>
                <td class="td-action">
                  <button
                    class="btn btn-sm btn-ghost"
                    @click="downloadFile(entry)"
                  >
                    Download
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>

        <div
          v-else
          class="panel browser-panel empty-browser"
        >
          <span class="muted">Select an archive to browse its contents.</span>
        </div>
      </div>
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

.muted {
  color: var(--text-muted);
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
  grid-template-columns: 380px 1fr;
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

.td-name {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-family: var(--mono);
  font-size: 0.82rem;
}

.td-size {
  font-size: 0.8rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.td-action {
  text-align: right;
}

.icon-dir,
.icon-file {
  font-size: 0.9rem;
  flex-shrink: 0;
}

.breadcrumb {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.1rem;
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.crumb {
  background: none;
  border: none;
  color: var(--accent);
  cursor: pointer;
  font-size: 0.82rem;
  font-family: var(--mono);
  padding: 0.15rem 0.3rem;
  border-radius: var(--radius-sm);
  transition:
    background 0.1s,
    color 0.1s;
}

.crumb:hover {
  background: var(--accent-subtle);
  color: var(--accent-hover);
}

.crumb-last {
  color: var(--text-primary);
  cursor: default;
}

.crumb-last:hover {
  background: none;
  color: var(--text-primary);
}

.crumb:not(.crumb-last)::after {
  content: ' /';
  color: var(--text-muted);
  margin-left: 0.2rem;
}

.empty-browser {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
  font-size: 0.875rem;
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

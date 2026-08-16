<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes, formatDate, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useToast } from '../composables/useToast'
import { useClipboard } from '../composables/useClipboard'
import { cronToHuman } from '../utils/cron'
import { repoOpLabel } from '../utils/repoOp'
import BaseModal from './BaseModal.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { ActiveRepoOp, RepoOpKind, RepoWithStats } from '../types/repo'

interface EditForm {
  name: string
  repo_path: string
  ssh_user: string
  ssh_host: string
  ssh_port: number
  compression: CompressionType
  encryption: EncryptionType
  enabled: boolean
  sync_schedule: string | null
}

type CompressionType = 'lz4' | 'zstd' | 'zlib' | 'none'

type EncryptionType =
  | 'repokey'
  | 'repokey-blake2'
  | 'keyfile'
  | 'keyfile-blake2'
  | 'authenticated'
  | 'authenticated-blake2'
  | 'none'

function classifyLastOpKind(kind: string | null): RepoOpKind | 'unknown' {
  if (
    kind === 'agent_backup' ||
    kind === 'server_sync' ||
    kind === 'break_lock' ||
    kind === 'delete_archive' ||
    kind === 'agent_check' ||
    kind === 'agent_verify' ||
    kind === 'compact_repo'
  ) {
    return kind
  }
  return 'unknown'
}

function lastOpLabel(kind: string | null): string {
  switch (classifyLastOpKind(kind)) {
    case 'agent_backup':
      return 'Agent backup'
    case 'server_sync':
      return 'Server sync'
    case 'break_lock':
      return 'Break lock'
    case 'delete_archive':
      return 'Delete archive'
    case 'agent_check':
      return 'Integrity check'
    case 'agent_verify':
      return 'Verify'
    case 'compact_repo':
      return 'Compact repository'
    case 'unknown':
      return kind ?? 'Unknown'
  }
}

const VALID_COMPRESSION_BASES: CompressionType[] = ['lz4', 'zstd', 'zlib', 'none']

function normalizeCompression(raw: string): CompressionType {
  const base = raw.split(',')[0] as CompressionType
  return VALID_COMPRESSION_BASES.includes(base) ? base : 'lz4'
}

const props = defineProps<{
  repo: RepoWithStats
  isAdmin: boolean
  currentOp: ActiveRepoOp | null
  /** Label for the running import phase; the parent tracks it over the socket. */
  importPhaseVerb: string
}>()

const emit = defineEmits<{ saved: []; 'import-reset': [] }>()

const { success: toastSuccess, error: toastError } = useToast()
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

const isEditing = ref(false)
const editLoading = ref(false)
const editError = ref<string | null>(null)
const editForm = reactive<EditForm>({
  name: '',
  repo_path: '',
  ssh_user: '',
  ssh_host: '',
  ssh_port: 22,
  compression: 'lz4',
  encryption: 'repokey-blake2',
  enabled: true,
  sync_schedule: null,
})

const showPassphraseDialog = ref(false)
const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)

const syncLoading = ref(false)
const resetImportLoading = ref(false)

const repo = computed(() => props.repo)
const isAdmin = computed(() => props.isAdmin)
const currentOp = computed(() => props.currentOp)
const importPhaseVerb = computed(() => props.importPhaseVerb)

function startEdit(): void {
  if (!repo.value) return
  editForm.name = props.repo.name
  editForm.repo_path = props.repo.repo_path
  editForm.ssh_user = props.repo.ssh_user
  editForm.ssh_host = props.repo.ssh_host
  editForm.ssh_port = props.repo.ssh_port
  editForm.compression = normalizeCompression(props.repo.compression)
  editForm.encryption = props.repo.encryption as EncryptionType
  editForm.enabled = props.repo.enabled
  editForm.sync_schedule = props.repo.sync_schedule ?? null
  editError.value = null
  isEditing.value = true
}

function cancelEdit(): void {
  isEditing.value = false
  editError.value = null
}

async function saveEdit(): Promise<void> {
  editLoading.value = true
  editError.value = null
  try {
    const connRes = await apiClient.post<{
      ssh_ok: boolean
      borg_installed: boolean
      error?: string
    }>('/ssh/test-connection', {
      ssh_host: editForm.ssh_host.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_port: editForm.ssh_port,
    })
    if (!connRes.data.ssh_ok) {
      editError.value = connRes.data.error ?? 'Cannot reach repository host — changes not saved'
      return
    }
    await apiClient.put(`/repos/${props.repo.id}`, {
      name: editForm.name.trim(),
      repo_path: editForm.repo_path.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_host: editForm.ssh_host.trim(),
      ssh_port: editForm.ssh_port,
      compression: editForm.compression,
      encryption: editForm.encryption,
      enabled: editForm.enabled,
      sync_schedule: editForm.sync_schedule,
    })
    isEditing.value = false
    emit('saved')
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

async function revealPassphrase(): Promise<void> {
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await apiClient.get<{ passphrase: string }>(`/repos/${props.repo.id}/passphrase`)
    passphrase.value = res.data.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

const showAcceptHostKeyDialog = ref(false)
const hostKeyCheckLoading = ref(false)
const hostKeyMismatch = ref(false)
const acceptHostKeyLoading = ref(false)
const acceptHostKeyError = ref<string | null>(null)
const expectedHostKey = ref<string | null>(null)

/**
 * Scans the repository host's current SSH key and compares it against the one
 * on record, so a changed key surfaces here rather than as a failed backup.
 */
async function checkHostKeyMismatch(): Promise<void> {
  hostKeyCheckLoading.value = true
  expectedHostKey.value = null
  hostKeyMismatch.value = false
  try {
    const res = await apiClient.post<{ ssh_host_key: string }>(
      `/repos/${props.repo.id}/ssh-host-key/scan`,
    )
    const sshHostKey = res.data.ssh_host_key
    if (props.repo.ssh_host_key !== sshHostKey) {
      expectedHostKey.value = sshHostKey
      hostKeyMismatch.value = true
    }
  } catch (e: unknown) {
    logger.debug('host key scan failed', e)
  } finally {
    hostKeyCheckLoading.value = false
  }
}

async function acceptHostKey(): Promise<void> {
  if (!expectedHostKey.value) return
  acceptHostKeyLoading.value = true
  acceptHostKeyError.value = null
  try {
    await apiClient.post(`/repos/${props.repo.id}/ssh-host-key`, {
      ssh_host_key: expectedHostKey.value,
    })
    showAcceptHostKeyDialog.value = false
    emit('saved')
    await checkHostKeyMismatch()
    toastSuccess('SSH host key accepted.')
  } catch (e: unknown) {
    acceptHostKeyError.value = extractError(e)
  } finally {
    acceptHostKeyLoading.value = false
  }
}

watch(() => props.repo.id, checkHostKeyMismatch)
onMounted(checkHostKeyMismatch)

async function syncRepo(): Promise<void> {
  syncLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/sync?build_index=true`)
    toastSuccess('Full resync started. Archive contents are being indexed in the background.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    syncLoading.value = false
  }
}

async function resetImport(): Promise<void> {
  resetImportLoading.value = true
  try {
    await apiClient.post(`/repos/${props.repo.id}/reset-import`)
    toastSuccess('Import state reset.')
    emit('import-reset')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    resetImportLoading.value = false
  }
}
</script>

<template>
  <div class="info-card">
    <div class="info-card-header">
      <h3 class="info-title">Repository Information</h3>
      <div class="info-header-actions">
        <template v-if="isAdmin && !isEditing">
          <button
            v-if="!repo.importing"
            class="btn btn-sm btn-ghost"
            :disabled="syncLoading"
            @click="syncRepo"
          >
            {{ syncLoading ? 'Syncing...' : 'Full Resync' }}
          </button>
          <button
            v-if="repo.importing || repo.import_error"
            class="btn btn-sm btn-ghost btn-danger-text"
            :disabled="resetImportLoading"
            @click="resetImport"
          >
            {{ resetImportLoading ? 'Resetting...' : 'Cancel Import' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="passphraseLoading"
            @click="revealPassphrase"
          >
            {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
          </button>
          <button
            v-if="hostKeyMismatch"
            class="btn btn-sm btn-ghost btn-warning-text"
            :disabled="hostKeyCheckLoading"
            @click="showAcceptHostKeyDialog = true"
          >
            {{ hostKeyCheckLoading ? 'Checking...' : 'Accept SSH Key' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            @click="startEdit"
          >
            Edit
          </button>
        </template>
      </div>
    </div>

    <template v-if="!isEditing">
      <dl class="info-grid">
        <dt>Name</dt>
        <dd class="mono">{{ repo.name }}</dd>
        <dt>SSH Target</dt>
        <dd class="mono">{{ repo.ssh_user }}@{{ repo.ssh_host }}:{{ repo.ssh_port }}</dd>
        <dt>SSH Host Key</dt>
        <dd class="mono ssh-host-key">
          {{ repo.ssh_host_key ?? 'Not set' }}
        </dd>
        <dt>Repo Path</dt>
        <dd class="mono">{{ repo.repo_path }}</dd>
        <dt>Compression</dt>
        <dd>{{ repo.compression }}</dd>
        <dt>Encryption</dt>
        <dd>{{ repo.encryption }}</dd>
        <dt>Status</dt>
        <dd>
          <span
            class="badge repo-status-badge"
            :class="
              repo.import_error
                ? 'badge--danger'
                : repo.importing
                  ? 'badge--warning badge--pulse'
                  : repo.enabled
                    ? 'badge--success'
                    : 'badge--neutral'
            "
            :title="repo.import_error ?? undefined"
          >
            {{
              repo.import_error
                ? 'Import Failed'
                : repo.importing
                  ? repo.import_total > 0
                    ? `${importPhaseVerb} ${repo.import_progress}/${repo.import_total}`
                    : `${importPhaseVerb}\u2026`
                  : repo.enabled
                    ? 'Enabled'
                    : 'Disabled'
            }}
          </span>
          <div
            v-if="repo.importing && repo.import_total > 0"
            class="import-progress"
          >
            <div class="import-progress-track">
              <div
                class="import-progress-bar"
                :style="{
                  width: `${Math.round((repo.import_progress / repo.import_total) * 100)}%`,
                }"
              ></div>
            </div>
            <span class="import-progress-label">
              {{ Math.round((repo.import_progress / repo.import_total) * 100) }}%
            </span>
          </div>
          <p
            v-if="repo.importing && repo.import_status_message"
            class="import-status-msg"
          >
            {{ repo.import_status_message }}
          </p>
        </dd>
        <dt>Archives</dt>
        <dd>{{ repo.archive_count }}</dd>
        <dt>Original Size</dt>
        <dd>{{ formatBytes(repo.total_original_size) }}</dd>
        <dt>Compressed</dt>
        <dd>{{ formatBytes(repo.total_compressed_size) }}</dd>
        <dt>Deduplicated</dt>
        <dd>{{ formatBytes(repo.total_deduplicated_size) }}</dd>
        <dt>Last Backup</dt>
        <dd>{{ relativeTime(repo.last_backup_at ?? '') }}</dd>
        <dt>Disk Sync</dt>
        <dd>
          <template v-if="repo.sync_schedule">
            {{ cronToHuman(repo.sync_schedule) ?? repo.sync_schedule }}
          </template>
          <template v-else>Disabled</template>
        </dd>
        <dt>Last Synced</dt>
        <dd>{{ repo.last_synced_at ? formatDate(repo.last_synced_at) : 'Never' }}</dd>
        <dt>Last Operation</dt>
        <dd>
          <template v-if="repo.last_op_kind">
            {{ lastOpLabel(repo.last_op_kind) }}
            <template v-if="repo.last_op_by && repo.last_op_by !== 'server'">
              by {{ repo.last_op_by }}
            </template>
            <template v-if="repo.last_op_at">
              — {{ relativeTime(repo.last_op_at ?? '') }}
            </template>
          </template>
          <template v-else>Never</template>
        </dd>
        <template v-if="currentOp">
          <dt>Current Operation</dt>
          <dd class="current-op-running">{{ repoOpLabel(currentOp) }}</dd>
        </template>
        <dt>Agents</dt>
        <dd>{{ repo.agent_count }}</dd>
      </dl>
    </template>

    <template v-else>
      <div class="edit-form">
        <div class="form-grid">
          <div class="field field-full">
            <label class="field-label">Name</label>
            <input
              v-model="editForm.name"
              class="input"
              placeholder="e.g. Web Server Backup"
            />
          </div>
          <div class="field">
            <label class="field-label">SSH User</label>
            <input
              v-model="editForm.ssh_user"
              class="input mono"
            />
          </div>
          <div class="field">
            <label class="field-label">SSH Host</label>
            <input
              v-model="editForm.ssh_host"
              class="input mono"
            />
          </div>
          <div class="field field-narrow">
            <label class="field-label">SSH Port</label>
            <input
              v-model.number="editForm.ssh_port"
              class="input"
              type="number"
              min="1"
              max="65535"
            />
          </div>
          <div class="field field-full">
            <label class="field-label">Repo Path</label>
            <input
              v-model="editForm.repo_path"
              class="input mono"
            />
          </div>
          <div class="field">
            <label class="field-label">Compression</label>
            <select
              v-model="editForm.compression"
              class="input"
            >
              <option value="lz4">lz4</option>
              <option value="zstd">zstd</option>
              <option value="zlib">zlib</option>
              <option value="none">none</option>
            </select>
          </div>
          <div class="field">
            <label class="field-label">Encryption</label>
            <select
              v-model="editForm.encryption"
              class="input"
            >
              <option value="repokey">repokey</option>
              <option value="repokey-blake2">repokey-blake2</option>
              <option value="keyfile">keyfile</option>
              <option value="keyfile-blake2">keyfile-blake2</option>
              <option value="authenticated">authenticated</option>
              <option value="authenticated-blake2">authenticated-blake2</option>
              <option value="none">none</option>
            </select>
          </div>
          <div class="field field-full toggle-row">
            <span class="toggle-row-label">Enabled</span>
            <ToggleSwitch v-model="editForm.enabled" />
          </div>
          <div class="field field-full toggle-row">
            <span class="toggle-row-label">Disk Sync</span>
            <ToggleSwitch
              :model-value="editForm.sync_schedule !== null"
              @update:model-value="editForm.sync_schedule = $event ? '0 0,12 * * *' : null"
            />
          </div>
          <div
            v-if="editForm.sync_schedule !== null"
            class="field field-full"
          >
            <label class="field-label">Sync Schedule (cron)</label>
            <input
              v-model="editForm.sync_schedule"
              class="input mono"
              placeholder="0 0,12 * * *"
            />
            <span class="field-hint">Cron expression for automatic disk sync</span>
          </div>
        </div>
        <div
          v-if="editError"
          class="form-error"
        >
          {{ editError }}
        </div>
        <div class="edit-actions">
          <button
            class="btn btn-ghost"
            @click="cancelEdit"
          >
            Cancel
          </button>
          <button
            class="btn btn-primary"
            :disabled="editLoading"
            @click="saveEdit"
          >
            {{ editLoading ? 'Saving...' : 'Save Changes' }}
          </button>
        </div>
      </div>
    </template>
  </div>

  <!-- Passphrase Dialog -->
  <BaseModal
    :open="showPassphraseDialog"
    :title="passphrase ? 'Repository Passphrase' : 'Error'"
    @close="showPassphraseDialog = false"
  >
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

    <template #footer>
      <button
        class="btn btn-primary"
        @click="showPassphraseDialog = false"
      >
        Done
      </button>
    </template>
  </BaseModal>

  <!-- SSH Host Key Dialog -->
  <BaseModal
    :open="showAcceptHostKeyDialog"
    title="Accept SSH Host Key"
    @close="showAcceptHostKeyDialog = false"
  >
    <p class="break-lock-warning">
      A different SSH host key was detected for <code>{{ repo?.ssh_host }}</code
      >. Verify the key below before accepting it.
    </p>
    <div
      v-if="expectedHostKey"
      class="ssh-key-box mono"
    >
      {{ expectedHostKey }}
    </div>
    <div
      v-if="acceptHostKeyError"
      class="form-error"
    >
      {{ acceptHostKeyError }}
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showAcceptHostKeyDialog = false"
      >
        Cancel
      </button>
      <button
        v-if="expectedHostKey"
        class="btn btn-primary"
        :disabled="acceptHostKeyLoading"
        @click="acceptHostKey"
      >
        {{ acceptHostKeyLoading ? 'Accepting...' : 'Accept Key' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.info-card-header {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem 1rem;
  margin-bottom: 1.25rem;
}

.info-header-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.edit-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.edit-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  padding-top: 0.5rem;
  border-top: 1px solid var(--border);
}

.import-progress {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.4rem;
}

.import-progress-track {
  flex: 1;
  height: 4px;
  background: var(--border);
  border-radius: var(--radius-pill);
  overflow: hidden;
}

.import-progress-bar {
  height: 100%;
  background: var(--accent);
  border-radius: var(--radius-pill);
  transition: width var(--duration-value) ease;
}

.import-progress-label {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  white-space: nowrap;
}

.import-status-msg {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin: 0.4rem 0 0;
  word-break: break-word;
}

.passphrase-warning {
  color: var(--warning);
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
  color: var(--text-primary);
  word-break: break-all;
  background: transparent;
  padding: 0;
}

.current-op-running {
  color: var(--warning);
  font-weight: 500;
}

.field-full {
  grid-column: 1 / -1;
}

.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0 1rem;
}

.field-narrow {
  max-width: 120px;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
  justify-content: space-between;
  margin-top: 0.5rem;
}

.toggle-row-label {
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

.ssh-host-key {
  word-break: break-all;
}

.ssh-key-box {
  margin-top: 0.75rem;
  padding: 0.85rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  font-size: var(--fs-sm);
  line-height: 1.5;
  word-break: break-all;
}

.break-lock-warning {
  color: var(--danger);
  font-size: var(--fs-base);
  line-height: 1.5;
}
</style>

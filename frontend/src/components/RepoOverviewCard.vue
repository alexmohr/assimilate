<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { testRepoConnection, updateRepo } from '../api/repos'
import { listRepoHosts, scanRepoHostKey, type RepoHost } from '../api/repoHosts'
import { formatBytes, formatDate, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { cronToHuman } from '../utils/cron'
import { repoOpLabel } from '../utils/repoOp'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import EditFormActions from './EditFormActions.vue'
import CronBuilder from './CronBuilder.vue'
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
}>()

const emit = defineEmits<{ saved: [] }>()

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

const repo = computed(() => props.repo)
const isAdmin = computed(() => props.isAdmin)
const currentOp = computed(() => props.currentOp)

/** CronBuilder always expects a string; sync_schedule is only ever null while
    disk sync is off, which is when this field is hidden. */
const syncScheduleCron = computed<string>({
  get: () => editForm.sync_schedule ?? '0 0,12 * * *',
  set: (val) => {
    editForm.sync_schedule = val
  },
})

/**
 * The known repository hosts, for the Host picker. A repository is moved by
 * picking another host, or by naming a new one; the host's port comes with
 * it. Loaded when editing starts, and best-effort - without the list the
 * hostname and port can still be typed in.
 */
const knownHosts = ref<RepoHost[]>([])
const NEW_HOST = 'new'
const hostChoice = ref<number | typeof NEW_HOST>(NEW_HOST)

async function loadKnownHosts(): Promise<void> {
  try {
    knownHosts.value = await listRepoHosts()
  } catch (e: unknown) {
    logger.debug('repository hosts failed to load', e)
    knownHosts.value = []
  }
}

/**
 * The picker's options: every known host, and always the repository's own -
 * so the current choice has an option to select while the list is loading or
 * if it failed to load.
 */
const hostOptions = computed<Pick<RepoHost, 'id' | 'ssh_host' | 'ssh_port'>[]>(() => {
  const own = {
    id: props.repo.repo_host.id,
    ssh_host: props.repo.ssh_host,
    ssh_port: props.repo.ssh_port,
  }
  const others = knownHosts.value.filter((h) => h.id !== own.id)
  return [own, ...others].sort((a, b) => a.ssh_host.localeCompare(b.ssh_host))
})

watch(hostChoice, (choice) => {
  if (choice === NEW_HOST) return
  const host = hostOptions.value.find((h) => h.id === choice)
  if (!host) return
  editForm.ssh_host = host.ssh_host
  editForm.ssh_port = host.ssh_port
})

function startEdit(): void {
  if (!repo.value) return
  hostChoice.value = props.repo.repo_host.id
  void loadKnownHosts()
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
    const connRes = await testRepoConnection({
      ssh_host: editForm.ssh_host.trim(),
      ssh_user: editForm.ssh_user.trim(),
      ssh_port: editForm.ssh_port,
    })
    if (!connRes.ssh_ok) {
      editError.value = connRes.error ?? 'Cannot reach repository host — changes not saved'
      return
    }
    await updateRepo(props.repo.id, {
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

/**
 * Whether the host now presents a different SSH key than the one pinned for
 * it - the signature of a reinstall or of a man-in-the-middle, so it surfaces
 * here rather than only as the next failed backup. Accepting it is done on the
 * host, once for every repository on it. A failed scan is not evidence of a
 * change: the host may simply be asleep.
 */
const hostKeyChanged = ref(false)

async function checkHostKey(): Promise<void> {
  hostKeyChanged.value = false
  if (!props.isAdmin) return
  try {
    const { ssh_host_key: key } = await scanRepoHostKey(props.repo.repo_host.id)
    hostKeyChanged.value = key !== props.repo.ssh_host_key
  } catch (e: unknown) {
    logger.debug('host key scan failed', e)
  }
}

// Checked again when the card moves to another host, and when its host is
// reached somewhere else or a new key is pinned for it - each source on its
// own, so a refreshed repository with the same values does not re-scan.
watch(
  [
    () => props.repo.repo_host.id,
    () => props.repo.ssh_host,
    () => props.repo.ssh_port,
    () => props.repo.ssh_host_key,
  ],
  checkHostKey,
)
onMounted(checkHostKey)
</script>

<template>
  <div>
    <div class="pane-head">
      <HelpHint label="connection details">
        Where this repository lives and how borg writes to it.
      </HelpHint>
      <button
        v-if="isAdmin && !isEditing"
        class="btn btn-sm btn-ghost"
        @click="startEdit"
      >
        Edit
      </button>
    </div>

    <template v-if="!isEditing">
      <p
        v-if="hostKeyChanged"
        class="state-msg state-msg--inline state-warning"
      >
        The host presents a different SSH host key than the one pinned for it.
        <RouterLink :to="`/repo-hosts/${repo.repo_host.id}?section=connection`">
          Review it on the host
        </RouterLink>
      </p>
      <dl class="info-grid">
        <dt>Name</dt>
        <dd class="mono">{{ repo.name }}</dd>
        <dt>
          Host
          <HelpHint label="the repository host">
            The machine this repository lives on. Its address, SSH host key, power and availability
            settings are set on the host, once for every repository on it.
          </HelpHint>
        </dt>
        <dd class="mono">
          <RouterLink
            v-if="isAdmin"
            :to="`/repo-hosts/${repo.repo_host.id}`"
          >
            {{ repo.ssh_host }}:{{ repo.ssh_port }}
          </RouterLink>
          <template v-else>{{ repo.ssh_host }}:{{ repo.ssh_port }}</template>
        </dd>
        <dt>SSH user</dt>
        <dd class="mono">{{ repo.ssh_user }}</dd>
        <dt>Repo path</dt>
        <dd class="mono">{{ repo.repo_path }}</dd>
        <dt>Compression</dt>
        <dd>{{ repo.compression }}</dd>
        <dt>Encryption</dt>
        <dd>{{ repo.encryption }}</dd>
        <dt>Archives</dt>
        <dd>{{ repo.archive_count }}</dd>
        <dt>Original size</dt>
        <dd>{{ formatBytes(repo.total_original_size) }}</dd>
        <dt>Compressed</dt>
        <dd>{{ formatBytes(repo.total_compressed_size) }}</dd>
        <dt>Deduplicated</dt>
        <dd>{{ formatBytes(repo.total_deduplicated_size) }}</dd>
        <dt>Last backup</dt>
        <dd>{{ relativeTime(repo.last_backup_at ?? '') }}</dd>
        <dt>Disk sync</dt>
        <dd>
          <template v-if="repo.sync_schedule">
            {{ cronToHuman(repo.sync_schedule) ?? repo.sync_schedule }}
          </template>
          <template v-else>Disabled</template>
        </dd>
        <dt>Last synced</dt>
        <dd>{{ repo.last_synced_at ? formatDate(repo.last_synced_at) : 'Never' }}</dd>
        <dt>Last operation</dt>
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
          <dt>Current operation</dt>
          <dd class="current-op-running">{{ repoOpLabel(currentOp) }}</dd>
        </template>
        <dt>Agents</dt>
        <dd>{{ repo.agent_count }}</dd>
      </dl>
    </template>

    <template v-else>
      <div class="edit-form">
        <div class="pane-rows">
          <PaneRow
            title="Name"
            label-for="repo-name"
            stack
          >
            <input
              id="repo-name"
              v-model="editForm.name"
              class="input"
              placeholder="e.g. Web Server Backup"
            />
          </PaneRow>
          <PaneRow
            title="Host"
            label-for="repo-host"
            help="moving to another host"
            stack
          >
            <template #help>
              A repository on another host takes that host's port, SSH host key, power and
              availability settings. Change the host itself - its hostname, port or key - on the
              host's own page.
            </template>
            <select
              id="repo-host"
              v-model="hostChoice"
              class="input mono"
            >
              <option
                v-for="host in hostOptions"
                :key="host.id"
                :value="host.id"
              >
                {{ host.ssh_host }}:{{ host.ssh_port }}
              </option>
              <option :value="NEW_HOST">Add a new host...</option>
            </select>
          </PaneRow>
          <PaneRow
            v-if="hostChoice === NEW_HOST"
            class="pane-nest"
            title="New host"
            label-for="repo-ssh-host"
            hint="Hostname and SSH port of a machine no other repository uses yet."
            stack
          >
            <div class="field-row">
              <input
                id="repo-ssh-host"
                v-model="editForm.ssh_host"
                class="input mono"
                aria-label="SSH host"
              />
              <input
                v-model.number="editForm.ssh_port"
                class="input field-narrow"
                type="number"
                min="1"
                max="65535"
                aria-label="SSH port"
              />
            </div>
          </PaneRow>
          <PaneRow
            title="SSH user"
            label-for="repo-ssh-user"
            hint="The user borg logs in as on the host."
            stack
          >
            <input
              id="repo-ssh-user"
              v-model="editForm.ssh_user"
              class="input mono"
            />
          </PaneRow>
          <PaneRow
            title="Repo path"
            label-for="repo-path"
            stack
          >
            <input
              id="repo-path"
              v-model="editForm.repo_path"
              class="input mono"
            />
          </PaneRow>
          <PaneRow title="Compression">
            <select
              v-model="editForm.compression"
              class="input"
              aria-label="Compression"
            >
              <option value="lz4">lz4</option>
              <option value="zstd">zstd</option>
              <option value="zlib">zlib</option>
              <option value="none">none</option>
            </select>
          </PaneRow>
          <PaneRow title="Encryption">
            <select
              v-model="editForm.encryption"
              class="input"
              aria-label="Encryption"
            >
              <option value="repokey">repokey</option>
              <option value="repokey-blake2">repokey-blake2</option>
              <option value="keyfile">keyfile</option>
              <option value="keyfile-blake2">keyfile-blake2</option>
              <option value="authenticated">authenticated</option>
              <option value="authenticated-blake2">authenticated-blake2</option>
              <option value="none">none</option>
            </select>
          </PaneRow>
          <PaneRow title="Enabled">
            <ToggleSwitch
              v-model="editForm.enabled"
              label="Enabled"
            />
          </PaneRow>
          <PaneRow title="Disk sync">
            <ToggleSwitch
              :model-value="editForm.sync_schedule !== null"
              label="Disk sync"
              @update:model-value="editForm.sync_schedule = $event ? '0 0,12 * * *' : null"
            />
          </PaneRow>
          <PaneRow
            v-if="editForm.sync_schedule !== null"
            class="pane-nest"
            title="Sync schedule"
            hint="Cron expression for automatic disk sync."
            stack
          >
            <CronBuilder v-model="syncScheduleCron" />
          </PaneRow>
        </div>
        <EditFormActions
          :saving="editLoading"
          :error="editError"
          save-label="Save changes"
          @cancel="cancelEdit"
          @save="saveEdit"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
.current-op-running {
  color: var(--warning);
  font-weight: 500;
}
</style>

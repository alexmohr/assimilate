<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RefreshCw } from '@lucide/vue'
import { acceptRepoHostKey, scanRepoHostKey, updateRepoHost, type RepoHost } from '../api/repoHosts'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useToast } from '../composables/useToast'
import BaseModal from './BaseModal.vue'
import EditableSection from './EditableSection.vue'
import PaneRow from './PaneRow.vue'

/**
 * Where a repository host is reached, and the SSH host key that proves it is
 * the machine it was.
 *
 * Both belong to the machine rather than to any repository on it: a host has
 * one address, one port and one key, and a key accepted here is the one every
 * repository on the host is verified against. After a reinstall the new key is
 * accepted once, here, instead of once per repository - and never silently:
 * a key that does not match is refused until someone looks at it.
 */
const props = defineProps<{
  host: RepoHost
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [host: RepoHost] }>()

const { success: toastSuccess, error: toastError } = useToast()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const sshHost = ref('')
const sshPort = ref(22)

const repoCount = computed(() => props.host.repositories.length)

const addressHint = computed(() =>
  repoCount.value === 1
    ? 'Applies to the 1 repository on this host. The host key is checked again on the next connection.'
    : `Applies to all ${repoCount.value} repositories on this host. The host key is checked again on the next connection.`,
)

function startEdit(): void {
  sshHost.value = props.host.ssh_host
  sshPort.value = props.host.ssh_port
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const updated = await updateRepoHost(props.host.id, {
      ssh_host: sshHost.value.trim(),
      ssh_port: sshPort.value,
    })
    editing.value = false
    emit('saved', updated)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}

/** `ssh-ed25519 AAAA...` splits into the key type and the key itself. */
const keyType = computed(() => props.host.ssh_host_key?.split(' ')[0] ?? null)
const keyBody = computed(() => props.host.ssh_host_key?.split(' ').slice(1).join(' ') ?? null)

const scanning = ref(false)
const scannedKey = ref<string | null>(null)
const accepting = ref(false)
const acceptError = ref<string | null>(null)

/**
 * A key the host presents now that differs from the pinned one, found by the
 * quiet scan this card runs when it opens. It is only offered for review: a
 * changed key is the signature of a reinstall or of a man-in-the-middle, and
 * which one it is takes a human.
 */
const changedKey = ref<string | null>(null)

async function checkKey(): Promise<void> {
  changedKey.value = null
  if (!props.canEdit) return
  try {
    const { ssh_host_key: key } = await scanRepoHostKey(props.host.id)
    if (key !== props.host.ssh_host_key) changedKey.value = key
  } catch (e: unknown) {
    // The host may simply be asleep; that says nothing about its key.
    logger.debug('host key scan failed', e)
  }
}

watch(() => props.host.id, checkKey)
onMounted(checkKey)

function reviewChangedKey(): void {
  acceptError.value = null
  scannedKey.value = changedKey.value
}

async function scanKey(): Promise<void> {
  scanning.value = true
  try {
    const { ssh_host_key: key } = await scanRepoHostKey(props.host.id)
    if (key === props.host.ssh_host_key) {
      toastSuccess('The host presents the key already pinned for it')
      return
    }
    acceptError.value = null
    scannedKey.value = key
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    scanning.value = false
  }
}

async function acceptKey(): Promise<void> {
  const key = scannedKey.value
  if (!key) return
  accepting.value = true
  acceptError.value = null
  try {
    await acceptRepoHostKey(props.host.id, key)
    scannedKey.value = null
    changedKey.value = null
    toastSuccess('SSH host key accepted')
    emit('saved', { ...props.host, ssh_host_key: key })
  } catch (e: unknown) {
    acceptError.value = extractError(e)
  } finally {
    accepting.value = false
  }
}
</script>

<template>
  <div>
    <EditableSection
      label="Address"
      :editing="editing"
      :can-edit="canEdit"
      :saving="saving"
      :error="error"
      @edit="startEdit"
      @cancel="editing = false"
      @save="save"
    >
      <template #view>
        <dl class="info-grid">
          <dt>Hostname</dt>
          <dd class="mono">{{ host.ssh_host }}</dd>
          <dt>SSH port</dt>
          <dd class="mono">{{ host.ssh_port }}</dd>
        </dl>
      </template>
      <template #edit>
        <div class="pane-rows">
          <PaneRow
            title="Hostname"
            label-for="repo-host-hostname"
            :hint="addressHint"
            stack
          >
            <input
              id="repo-host-hostname"
              v-model="sshHost"
              class="input mono"
              placeholder="nas.example.com"
            />
          </PaneRow>
          <PaneRow
            title="SSH port"
            label-for="repo-host-port"
          >
            <input
              id="repo-host-port"
              v-model.number="sshPort"
              class="input field-narrow"
              type="number"
              min="1"
              max="65535"
            />
          </PaneRow>
        </div>
      </template>
    </EditableSection>
  </div>

  <section class="pane-section">
    <div class="pane-section-head">
      <p class="group-label">SSH host key</p>
      <button
        v-if="canEdit"
        type="button"
        class="btn btn-sm btn-ghost"
        :disabled="scanning"
        @click="scanKey"
      >
        <RefreshCw
          :size="14"
          :class="{ spinning: scanning }"
        />
        {{ scanning ? 'Scanning...' : 'Scan key' }}
      </button>
    </div>
    <p
      v-if="changedKey"
      class="state-msg state-msg--inline state-warning"
    >
      The host now presents a different key than the one pinned for it.
      <button
        type="button"
        class="btn btn-sm btn-ghost btn-warning-text"
        @click="reviewChangedKey"
      >
        Review key
      </button>
    </p>
    <dl class="info-grid">
      <template v-if="host.ssh_host_key">
        <dt>Type</dt>
        <dd class="mono">{{ keyType }}</dd>
        <dt>Key</dt>
        <dd class="mono host-key">{{ keyBody }}</dd>
      </template>
      <template v-else>
        <dt>Key</dt>
        <dd>Not pinned yet</dd>
      </template>
    </dl>
    <p class="field-hint">
      Every repository on this host is verified against this key. A connection that presents another
      one is refused until the new key is accepted here.
    </p>
  </section>

  <BaseModal
    :open="scannedKey !== null"
    title="Accept SSH host key"
    @close="scannedKey = null"
  >
    <p>
      <code>{{ host.ssh_host }}</code> presents
      {{ host.ssh_host_key ? 'a different key than the one pinned for it' : 'this key' }}. Verify it
      before accepting: every repository on the host will trust it.
    </p>
    <div class="ssh-key-box mono">{{ scannedKey }}</div>
    <div
      v-if="acceptError"
      class="form-error"
    >
      {{ acceptError }}
    </div>
    <template #footer>
      <button
        class="btn btn-ghost"
        type="button"
        @click="scannedKey = null"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        type="button"
        :disabled="accepting"
        @click="acceptKey"
      >
        {{ accepting ? 'Accepting...' : 'Accept key' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.host-key {
  word-break: break-all;
}

.ssh-key-box {
  margin-top: var(--space-5);
  padding: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  font-size: var(--fs-sm);
  line-height: 1.5;
  word-break: break-all;
}
</style>

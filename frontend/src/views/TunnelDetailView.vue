<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import {
  deleteTunnel,
  disableTunnel,
  enableTunnel,
  getTunnel,
  reconnectTunnel,
  updateTunnel,
} from '../api/tunnels'
import { listAgents } from '../api/agents'
import { useAsyncAction, type IsCurrent } from '../composables/useAsyncAction'
import { useWebSocket } from '../composables/useWebSocket'
import { badgeClass, tunnelStatusLabel, tunnelStatusTone } from '../utils/badge'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import { logger } from '../utils/logger'
import { tunnelErrorMessage } from '../utils/tunnel'
import BaseSpinner from '../components/BaseSpinner.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import DetailHeader from '../components/DetailHeader.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type { TunnelWithStatus, UpdateTunnelRequest } from '../types/tunnel'

/**
 * One reverse SSH tunnel: where it goes, whether it is up, and everything that
 * can be done to it. The tunnels list is only a way here - its rows used to
 * carry an overflow menu with these actions, and an edit dialog for the
 * settings that now sit inline below.
 */
const props = defineProps<{ id: string }>()
const router = useRouter()

const tunnelId = computed(() => Number(props.id))
const tunnel = ref<TunnelWithStatus | null>(null)
const agentHostname = ref<string | null>(null)
const agentDisplayName = ref<string | null>(null)
const { loading, error, runLatest } = useAsyncAction()

/** The agent's hostname, or its id while the agents list is still unknown. */
const agentLabel = computed(() => agentHostname.value ?? String(tunnel.value?.agent_id ?? ''))

const errorMessage = computed(() => (tunnel.value ? tunnelErrorMessage(tunnel.value.status) : null))

/** Reconnecting only means something for a tunnel that is meant to be up and is not. */
const canReconnect = computed(
  () => tunnel.value !== null && tunnel.value.enabled && tunnel.value.status !== 'connected',
)

const form = ref<UpdateTunnelRequest>({ ssh_host: '', ssh_user: '', ssh_port: 22, tunnel_port: 0 })
const saving = ref(false)
const saveError = ref<string | null>(null)

function resetForm(): void {
  if (!tunnel.value) return
  form.value = {
    ssh_host: tunnel.value.ssh_host,
    ssh_user: tunnel.value.ssh_user,
    ssh_port: tunnel.value.ssh_port,
    tunnel_port: tunnel.value.tunnel_port,
  }
  saveError.value = null
}

const formDirty = computed(
  () =>
    tunnel.value !== null &&
    (form.value.ssh_host !== tunnel.value.ssh_host ||
      form.value.ssh_user !== tunnel.value.ssh_user ||
      form.value.ssh_port !== tunnel.value.ssh_port ||
      form.value.tunnel_port !== tunnel.value.tunnel_port),
)

const formValid = computed(
  () => Boolean(form.value.ssh_host?.trim()) && Boolean(form.value.tunnel_port),
)

function applyUpdate(updated: Partial<TunnelWithStatus>): void {
  if (!tunnel.value) return
  tunnel.value = { ...tunnel.value, ...updated }
}

async function save(): Promise<void> {
  if (!tunnel.value || !formValid.value) return
  saving.value = true
  saveError.value = null
  try {
    applyUpdate(await updateTunnel(tunnel.value.id, form.value))
    resetForm()
  } catch (e: unknown) {
    saveError.value = extractError(e)
  } finally {
    saving.value = false
  }
}

const actionError = ref<string | null>(null)
const toggling = ref(false)

async function setEnabled(enabled: boolean): Promise<void> {
  if (!tunnel.value || toggling.value) return
  toggling.value = true
  actionError.value = null
  try {
    if (enabled) await enableTunnel(tunnel.value.id)
    else await disableTunnel(tunnel.value.id)
    tunnel.value = await getTunnel(tunnel.value.id)
  } catch (e: unknown) {
    actionError.value = extractError(e)
  } finally {
    toggling.value = false
  }
}

const reconnecting = ref(false)

async function reconnect(): Promise<void> {
  if (!tunnel.value) return
  reconnecting.value = true
  actionError.value = null
  try {
    applyUpdate(await reconnectTunnel(tunnel.value.id))
  } catch (e: unknown) {
    actionError.value = extractError(e)
  } finally {
    reconnecting.value = false
  }
}

const showDelete = ref(false)
const deleting = ref(false)
const deleteError = ref<string | null>(null)

async function confirmDelete(): Promise<void> {
  if (!tunnel.value) return
  deleting.value = true
  deleteError.value = null
  try {
    await deleteTunnel(tunnel.value.id)
    showDelete.value = false
    await router.push('/tunnels')
  } catch (e: unknown) {
    deleteError.value = extractError(e)
  } finally {
    deleting.value = false
  }
}

async function loadAgentName(agentId: number, isCurrent: IsCurrent): Promise<void> {
  try {
    const agent = (await listAgents()).find((a) => a.id === agentId)
    if (!isCurrent()) return
    agentHostname.value = agent?.hostname ?? null
    const name = agent?.display_name ?? null
    agentDisplayName.value = name && name !== agent?.hostname ? name : null
  } catch (e: unknown) {
    logger.error('loading the tunnel agent failed', e)
  }
}

/**
 * A route change to another tunnel starts a new load while the previous one
 * may still be in flight; only the latest may write to the page, so a slow
 * answer for the tunnel just left never replaces the one now shown.
 */
async function load(): Promise<void> {
  await runLatest(async (isCurrent) => {
    const loaded = await getTunnel(tunnelId.value)
    if (!isCurrent()) return
    tunnel.value = loaded
    resetForm()
    await loadAgentName(loaded.agent_id, isCurrent)
  })
}

const { onMessage } = useWebSocket()
onMessage('TunnelStatusChanged', (data) => {
  if (tunnel.value && tunnel.value.agent_id === data.agent_id) tunnel.value.status = data.status
})

watch(
  () => props.id,
  async () => {
    tunnel.value = null
    agentHostname.value = null
    agentDisplayName.value = null
    await load()
  },
)
onMounted(load)
</script>

<template>
  <div class="tunnel-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/tunnels"
        class="crumb-link"
      >
        Tunnels
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current mono">{{ tunnel ? agentLabel : '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !tunnel"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="tunnel">
      <DetailHeader
        :name="agentLabel"
        mono
        :subtitle="agentDisplayName ?? 'Reverse SSH tunnel'"
      >
        <template #badges>
          <span
            class="badge"
            :class="badgeClass(tunnelStatusTone(tunnel.status))"
            data-testid="tunnel-status"
          >
            <span class="badge-dot"></span>
            {{ tunnelStatusLabel(tunnel.status) }}
          </span>
        </template>
        <template #meta>
          <span>created {{ formatDate(tunnel.created_at) }}</span>
        </template>
        <template #actions>
          <button
            v-if="canReconnect"
            type="button"
            class="btn btn-sm btn-primary"
            :disabled="reconnecting"
            @click="reconnect"
          >
            {{ reconnecting ? 'Reconnecting...' : 'Reconnect' }}
          </button>
          <ToggleSwitch
            :model-value="tunnel.enabled"
            :disabled="toggling"
            label="Tunnel enabled"
            @update:model-value="setEnabled"
          >
            {{ tunnel.enabled ? 'Enabled' : 'Disabled' }}
          </ToggleSwitch>
        </template>
      </DetailHeader>

      <div
        v-if="actionError"
        class="error-banner"
      >
        {{ actionError }}
      </div>

      <div class="tunnel-grid">
        <section class="panel tunnel-grid-wide">
          <div class="panel-header">
            <h2 class="panel-title">Status</h2>
          </div>
          <template v-if="errorMessage">
            <pre
              class="error-pre"
              data-testid="tunnel-error"
              >{{ errorMessage }}</pre
            >
            <p class="field-hint">
              The server could not open this tunnel. Check that the server's public key is in
              <code>~/.ssh/authorized_keys</code> for <code>{{ tunnel.ssh_user }}</code> on
              <code>{{ tunnel.ssh_host }}</code> and that its sshd allows TCP forwarding, then
              reconnect.
            </p>
          </template>
          <p
            v-else-if="!tunnel.enabled"
            class="field-hint"
          >
            Disabled: the server does not open this tunnel. Turn it on above to connect.
          </p>
          <p
            v-else
            class="field-hint"
          >
            {{
              tunnel.status === 'connected'
                ? 'The SSH session is up and the remote port is bound.'
                : 'The session dropped; the server retries on its own.'
            }}
          </p>
        </section>

        <section class="panel">
          <div class="panel-header">
            <h2 class="panel-title">Route</h2>
          </div>
          <dl class="info-grid">
            <dt>Agent</dt>
            <dd>
              <RouterLink
                v-if="agentHostname"
                :to="`/agents/${agentHostname}`"
                class="mono"
              >
                {{ agentHostname }}
              </RouterLink>
              <span v-else>-</span>
            </dd>
            <dt>SSH target</dt>
            <dd class="mono">{{ tunnel.ssh_user }}@{{ tunnel.ssh_host }}:{{ tunnel.ssh_port }}</dd>
            <dt>Tunnel port</dt>
            <dd class="mono">127.0.0.1:{{ tunnel.tunnel_port }}</dd>
            <dt>Agent connects to</dt>
            <dd class="mono">ws://127.0.0.1:{{ tunnel.tunnel_port }}</dd>
          </dl>
          <p class="field-hint">
            The server opens an SSH session to the target and binds the tunnel port on the agent's
            loopback; the agent reaches the server through it.
          </p>
        </section>

        <section class="panel">
          <div class="panel-header">
            <h2 class="panel-title">Settings</h2>
          </div>
          <div class="field">
            <label
              class="field-label"
              for="tunnel-ssh-host"
              >SSH host <span class="required">*</span></label
            >
            <input
              id="tunnel-ssh-host"
              v-model="form.ssh_host"
              class="input mono"
              placeholder="e.g. 192.168.1.10"
            />
          </div>
          <div class="field-row">
            <div class="field">
              <label
                class="field-label"
                for="tunnel-ssh-user"
                >SSH user</label
              >
              <input
                id="tunnel-ssh-user"
                v-model="form.ssh_user"
                class="input mono"
                placeholder="root"
              />
            </div>
            <div class="field field-narrow">
              <label
                class="field-label"
                for="tunnel-ssh-port"
                >SSH port</label
              >
              <input
                id="tunnel-ssh-port"
                v-model.number="form.ssh_port"
                class="input"
                type="number"
                min="1"
                max="65535"
              />
            </div>
          </div>
          <div class="field">
            <label
              class="field-label"
              for="tunnel-port"
              >Tunnel port <span class="required">*</span></label
            >
            <input
              id="tunnel-port"
              v-model.number="form.tunnel_port"
              class="input"
              type="number"
              min="1"
              max="65535"
            />
          </div>
          <div
            v-if="saveError"
            class="form-error"
          >
            {{ saveError }}
          </div>
          <div class="panel-actions tunnel-form-actions">
            <button
              type="button"
              class="btn btn-sm btn-ghost"
              :disabled="!formDirty || saving"
              @click="resetForm"
            >
              Reset
            </button>
            <button
              type="button"
              class="btn btn-sm btn-primary"
              :disabled="!formDirty || !formValid || saving"
              @click="save"
            >
              {{ saving ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </section>

        <section class="panel tunnel-grid-wide">
          <div class="danger-body">
            <div class="danger-info">
              <span class="danger-heading">Delete tunnel</span>
              <span class="danger-desc">
                Stops the tunnel and removes it. An agent that connects through it goes offline
                until it has another way to reach the server.
              </span>
            </div>
            <button
              type="button"
              class="btn btn-sm btn-danger"
              @click="showDelete = true"
            >
              Delete tunnel
            </button>
          </div>
        </section>
      </div>

      <ConfirmDeleteDialog
        :show="showDelete"
        title="Delete tunnel"
        :submitting="deleting"
        :error="deleteError"
        @cancel="showDelete = false"
        @confirm="confirmDelete"
      >
        Delete the tunnel for <span class="mono">{{ agentLabel }}</span
        >?
      </ConfirmDeleteDialog>
    </template>
  </div>
</template>

<style scoped>
.tunnel-detail {
  max-width: 1200px;
}

.tunnel-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-6);
}

.tunnel-grid-wide {
  grid-column: 1 / -1;
}

.tunnel-form-actions {
  justify-content: flex-end;
}

@media (max-width: 640px) {
  .tunnel-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>

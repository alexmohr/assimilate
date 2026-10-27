<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted } from 'vue'
import { getRepoQuota, updateRepoQuota } from '../api/repos'
import { deleteServerQuota, getServerQuota, upsertServerQuota } from '../api/serverQuotas'
import { formatBytes } from '../utils/format'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useWebSocket } from '../composables/useWebSocket'
import { actionLabel, bytesToGbOrZero, gbToBytes, quotaCeiling, quotaHealth } from '../utils/quota'
import type { QuotaAction } from '../types/generated'
import type { QuotaData } from '../api/repos'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import EditFormActions from './EditFormActions.vue'
import HelpHint from './HelpHint.vue'

type QuotaStatus = 'ok' | 'warning' | 'critical'

/** What the panel shows and edits: the part a repository's and a host's quota share. */
type QuotaSettings = Pick<
  QuotaData,
  'warn_bytes' | 'critical_bytes' | 'warn_action' | 'critical_action' | 'enabled'
>

/**
 * One storage limit: a repository's own, or - given `sshHost` instead of
 * `repoId` - the one every repository on a repository host shares, measured
 * against their combined size. Only the host's can be removed again; a
 * repository's is switched off instead.
 */
const props = defineProps<{
  repoId?: number
  sshHost?: string
  isAdmin: boolean
  /** The repository's own size. A host's combined size comes with its quota. */
  currentUsageBytes?: number
}>()

const quota = ref<QuotaSettings | null>(null)
const hostUsageBytes = ref(0)
const removing = ref(false)

const usageBytes = computed((): number => props.currentUsageBytes ?? hostUsageBytes.value)
const subject = computed((): string =>
  props.sshHost === undefined ? 'this repository' : 'the repositories on this host',
)
const loading = ref(false)
const error = ref<string | null>(null)
const isEditing = ref(false)
const editLoading = ref(false)
const editError = ref<string | null>(null)

const editForm = reactive({
  warn_gb: 0,
  critical_gb: 0,
  warn_action: 'notify_only' as QuotaAction,
  critical_action: 'notify_only' as QuotaAction,
  enabled: true,
})

const quotaStatus = computed((): QuotaStatus => {
  const health = quotaHealth(quota.value, usageBytes.value)
  return health === 'unconfigured' ? 'ok' : health
})

const usagePercent = computed((): number => {
  if (!quota.value || !quota.value.enabled) return 0
  const limit = quotaCeiling(quota.value)
  if (limit === null || limit <= 0) return 0
  return Math.min(100, (usageBytes.value / limit) * 100)
})

const statusLabel = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'Warning'
  if (s === 'critical') return 'Critical'
  return 'OK'
})

const statusBadgeClass = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'badge-warn'
  if (s === 'critical') return 'badge-crit'
  return 'badge-ok'
})

const progressBarClass = computed((): string => {
  const s = quotaStatus.value
  if (s === 'warning') return 'bar-warn'
  if (s === 'critical') return 'bar-crit'
  return 'bar-ok'
})

/** A host has a row whether or not a quota is set on it; `configured` tells which. */
async function fetchQuota(): Promise<QuotaSettings | null> {
  if (props.sshHost === undefined) return getRepoQuota(props.repoId ?? 0)
  const host = await getServerQuota(props.sshHost)
  hostUsageBytes.value = host?.total_deduplicated_size ?? 0
  if (!host?.configured) return null
  return {
    warn_bytes: host.warn_bytes,
    critical_bytes: host.critical_bytes,
    warn_action: host.warn_action,
    critical_action: host.critical_action,
    enabled: host.enabled,
  }
}

async function loadQuota(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    quota.value = await fetchQuota()
  } catch (e: unknown) {
    const status = (e as { response?: { status?: number } }).response?.status
    if (status === 404) {
      quota.value = null
    } else {
      error.value = extractError(e)
    }
  } finally {
    loading.value = false
  }
}

function startEdit(): void {
  if (!quota.value) return
  editForm.warn_gb = bytesToGbOrZero(quota.value.warn_bytes)
  editForm.critical_gb = bytesToGbOrZero(quota.value.critical_bytes)
  editForm.warn_action = quota.value.warn_action
  editForm.critical_action = quota.value.critical_action
  editForm.enabled = quota.value.enabled
  editError.value = null
  isEditing.value = true
}

function startNewQuota(): void {
  editForm.warn_gb = 0
  editForm.critical_gb = 0
  editForm.warn_action = 'notify_only'
  editForm.critical_action = 'notify_only'
  editForm.enabled = true
  editError.value = null
  isEditing.value = true
}

function cancelEdit(): void {
  isEditing.value = false
  editError.value = null
}

async function saveQuota(): Promise<void> {
  editLoading.value = true
  editError.value = null
  try {
    const request = {
      warn_bytes: gbToBytes(editForm.warn_gb),
      critical_bytes: gbToBytes(editForm.critical_gb),
      warn_action: editForm.warn_action,
      critical_action: editForm.critical_action,
      enabled: editForm.enabled,
    }
    if (props.sshHost === undefined) await updateRepoQuota(props.repoId ?? 0, request)
    else await upsertServerQuota(props.sshHost, request)
    isEditing.value = false
    await loadQuota()
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    editLoading.value = false
  }
}

async function removeQuota(): Promise<void> {
  if (props.sshHost === undefined) return
  removing.value = true
  error.value = null
  try {
    await deleteServerQuota(props.sshHost)
    await loadQuota()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    removing.value = false
  }
}

onMounted(loadQuota)

/**
 * A host's combined size grows with every backup to any repository on it, so
 * its panel follows `DataChanged` the way the old Server Quotas page did. The
 * reload is quiet: no spinner, and a failure keeps the figures it has. A
 * repository's own size comes from its page, which already follows the event.
 */
async function refreshQuota(): Promise<void> {
  try {
    quota.value = await fetchQuota()
  } catch (e: unknown) {
    logger.error('background quota refresh failed', e)
  }
}

if (props.sshHost !== undefined) {
  useWebSocket().onMessage('DataChanged', () => {
    void refreshQuota()
  })
}
</script>

<template>
  <div>
    <div class="pane-head">
      <HelpHint label="space limits">
        <template v-if="sshHost === undefined">
          How much space this repository may use, and what Assimilate does as it fills up.
        </template>
        <template v-else>
          How much space the repositories on this host may use together, for when they share one
          disk or one provider quota, and what Assimilate does as it fills up. Each repository can
          still have a quota of its own as well.
        </template>
      </HelpHint>
      <div class="panel-actions">
        <span
          v-if="quota && quota.enabled"
          class="badge"
          :class="statusBadgeClass"
        >
          {{ statusLabel }}
        </span>
        <button
          v-if="isAdmin && !isEditing && quota"
          class="btn btn-sm btn-ghost"
          @click="startEdit"
        >
          Edit
        </button>
        <button
          v-if="isAdmin && !isEditing && quota && sshHost !== undefined"
          class="btn btn-sm btn-ghost btn-danger-text"
          :disabled="removing"
          @click="removeQuota"
        >
          {{ removing ? 'Removing...' : 'Remove' }}
        </button>
      </div>
    </div>

    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading quota...
    </div>

    <div
      v-else-if="error"
      class="state-msg state-msg--inline state-error"
    >
      {{ error }}
    </div>

    <template v-else-if="!quota && !isEditing">
      <div class="muted">No quota configured for {{ subject }}.</div>
      <div
        v-if="sshHost !== undefined"
        class="muted"
      >
        {{ formatBytes(usageBytes) }} used in total.
      </div>
      <button
        v-if="isAdmin"
        class="btn btn-sm btn-ghost quota-empty-action"
        @click="startNewQuota"
      >
        Configure quota
      </button>
    </template>

    <template v-else-if="quota && !isEditing">
      <div
        v-if="!quota.enabled"
        class="muted"
      >
        Quota monitoring is disabled for {{ subject }}.
      </div>
      <template v-else>
        <div class="quota-usage">
          <div class="usage-labels">
            <span class="usage-current">{{ formatBytes(usageBytes) }} used</span>
            <span class="usage-limit">
              {{ formatBytes(quota.critical_bytes ?? quota.warn_bytes) }} limit
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :class="progressBarClass"
              :style="{ width: usagePercent + '%' }"
            />
          </div>
        </div>
        <dl class="quota-details">
          <dt>Warning threshold</dt>
          <dd>{{ formatBytes(quota.warn_bytes) }}</dd>
          <dt>Warning action</dt>
          <dd>{{ actionLabel(quota.warn_action) }}</dd>
          <dt>Critical threshold</dt>
          <dd>{{ formatBytes(quota.critical_bytes) }}</dd>
          <dt>Critical action</dt>
          <dd>{{ actionLabel(quota.critical_action) }}</dd>
        </dl>
      </template>
    </template>

    <template v-else-if="isEditing">
      <div class="edit-form">
        <div class="pane-rows">
          <PaneRow title="Enabled">
            <ToggleSwitch
              v-model="editForm.enabled"
              label="Enabled"
            />
          </PaneRow>
          <PaneRow title="Warning (GB)">
            <input
              v-model.number="editForm.warn_gb"
              class="input field-narrow"
              type="number"
              min="0"
              step="0.1"
              aria-label="Warning (GB)"
            />
          </PaneRow>
          <PaneRow title="Warning action">
            <select
              v-model="editForm.warn_action"
              class="input"
              aria-label="Warning action"
            >
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
            </select>
          </PaneRow>
          <PaneRow title="Critical (GB)">
            <input
              v-model.number="editForm.critical_gb"
              class="input field-narrow"
              type="number"
              min="0"
              step="0.1"
              aria-label="Critical (GB)"
            />
          </PaneRow>
          <PaneRow title="Critical action">
            <select
              v-model="editForm.critical_action"
              class="input"
              aria-label="Critical action"
            >
              <option value="notify_only">Notify only</option>
              <option value="block_backups">Block backups</option>
              <option value="disable_schedule">Disable schedule</option>
            </select>
          </PaneRow>
        </div>
        <EditFormActions
          :saving="editLoading"
          :error="editError"
          @cancel="cancelEdit"
          @save="saveQuota"
        />
      </div>
    </template>
  </div>
</template>

<style scoped>
/* The button sits under the "nothing configured" line rather than beside it,
   so it needs the gap the pane's own layout does not give a sibling. */
.quota-empty-action {
  margin-top: var(--space-5);
}

.quota-usage {
  margin-bottom: var(--space-6);
}

.usage-labels {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-sm);
  margin-bottom: var(--space-3);
}

.usage-current {
  color: var(--text-primary);
  font-weight: 600;
}

.usage-limit {
  color: var(--text-muted);
}

.progress-bar-track {
  height: 8px;
  background: var(--bg-input);
  border-radius: var(--radius-pill);
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  border-radius: var(--radius-pill);
  transition: width var(--duration-slow) ease;
}

.bar-ok {
  background: var(--success);
}

.bar-warn {
  background: var(--warning);
}

.bar-crit {
  background: var(--danger);
}

.quota-details {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-3) var(--space-6);
  margin: 0;
  font-size: var(--fs-base);
}

.quota-details dt {
  color: var(--text-muted);
}

.quota-details dd {
  margin: 0;
  color: var(--text-primary);
}
</style>

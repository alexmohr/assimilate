<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import ToggleSwitch from './ToggleSwitch.vue'

const props = withDefaults(
  defineProps<{
    sshHost: string
    sshUser?: string
    sshPort?: number
    showCredentials?: boolean
  }>(),
  {
    sshUser: 'root',
    sshPort: 22,
    showCredentials: false,
  },
)

const localHost = ref(props.sshHost)
const localUser = ref(props.sshUser)
const localPort = ref(props.sshPort)
const password = ref('')
const useSftp = ref(true)
const loading = ref(false)
const result = ref<{ success: boolean; already_deployed: boolean; error?: string } | null>(null)

const effectiveHost = computed(() => (props.showCredentials ? localHost.value : props.sshHost))

const canDeploy = computed(() => !loading.value && !!password.value && !!effectiveHost.value.trim())

async function deploy(): Promise<void> {
  loading.value = true
  result.value = null
  try {
    const res = await apiClient.post<{
      success: boolean
      already_deployed: boolean
      error?: string
    }>('/ssh/deploy-key', {
      ssh_host: effectiveHost.value.trim(),
      ssh_user: (props.showCredentials ? localUser.value : props.sshUser).trim(),
      ssh_port: props.showCredentials ? localPort.value : props.sshPort,
      password: password.value,
      use_sftp: useSftp.value,
    })
    result.value = res.data
  } catch (e: unknown) {
    result.value = { success: false, already_deployed: false, error: extractError(e) }
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div>
    <p class="deploy-hint">
      Deploy the server's SSH public key to the target host for passwordless access.
    </p>
    <div class="deploy-fields">
      <template v-if="showCredentials">
        <div class="field">
          <label class="field-label">SSH Host <span class="required">*</span></label>
          <input
            v-model="localHost"
            class="input mono"
            placeholder="e.g. 192.168.1.10"
          />
        </div>
        <div class="deploy-row-fields">
          <div class="field">
            <label class="field-label">SSH User</label>
            <input
              v-model="localUser"
              class="input mono"
              placeholder="root"
            />
          </div>
          <div class="field field-narrow">
            <label class="field-label">SSH Port</label>
            <input
              v-model.number="localPort"
              class="input"
              type="number"
              min="1"
              max="65535"
            />
          </div>
        </div>
      </template>
      <div class="field">
        <label class="field-label">SSH Password <span class="required">*</span></label>
        <input
          v-model="password"
          class="input"
          type="password"
          placeholder="Password for initial login"
        />
      </div>
      <div class="field toggle-row">
        <span class="toggle-row-label">Use SFTP to deploy key (required for Hetzner)</span>
        <ToggleSwitch v-model="useSftp" />
      </div>
    </div>
    <div class="deploy-row">
      <button
        class="btn btn-sm btn-primary"
        :disabled="!canDeploy"
        @click="deploy"
      >
        {{ loading ? 'Deploying...' : 'Deploy Key' }}
      </button>
      <span
        v-if="result"
        class="deploy-result"
        :class="result.success || result.already_deployed ? 'result-ok' : 'result-error'"
      >
        <template v-if="result.already_deployed">Key already deployed</template>
        <template v-else-if="result.success">Key deployed successfully</template>
        <template v-else>{{ result.error ?? 'Deployment failed' }}</template>
      </span>
    </div>
  </div>
</template>

<style scoped>
.deploy-hint {
  font-size: 0.8rem;
  color: var(--text-muted);
  margin-bottom: 0.75rem;
}

.deploy-fields {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  margin-bottom: 0.75rem;
}

.deploy-fields .field {
  margin-bottom: 0;
}

.deploy-row-fields {
  display: flex;
  gap: 1rem;
}

.deploy-row-fields .field {
  flex: 1;
}

.field-narrow {
  max-width: 120px;
  flex: 0 0 120px !important;
}

.toggle-row {
  display: flex;
  flex-direction: row;
  gap: 1.5rem;
  align-items: center;
}

.toggle-row-label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.deploy-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
}

.deploy-result {
  font-size: 0.8rem;
  font-weight: 500;
}

.result-ok {
  color: var(--success);
}

.result-error {
  color: var(--danger);
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { RefreshCw } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import DependencyPortField from './DependencyPortField.vue'
import ModalFormActions from './ModalFormActions.vue'
import {
  createDependencyHost,
  testDependencyAddress,
  type DependencyHost,
} from '../api/dependencyHosts'
import { listRepoHosts, type RepoHost } from '../api/repoHosts'
import { testResultText } from '../utils/dependencyHost'
import { extractError } from '../utils/error'
import type { DependencyTestResponse } from '../types/generated'

/**
 * A new dependency: where the Assimilate server reaches it, which port says it
 * is up, and whether it is the same machine as a repository host. Wake-on-LAN
 * and "not always online" are left to its Power pane - they are easier to get
 * right once the connection test has said the address works.
 */
const props = defineProps<{ open: boolean }>()

const emit = defineEmits<{
  close: []
  created: [host: DependencyHost]
}>()

const form = reactive({
  name: '',
  address: '',
  port: 445,
  description: '',
  repoHostId: null as number | null,
})

const repoHosts = ref<RepoHost[]>([])
const submitting = ref(false)
const error = ref<string | null>(null)

const testing = ref(false)
const testResult = ref<DependencyTestResponse | null>(null)
const testError = ref<string | null>(null)

/** Fresh every time it opens: a half-filled form from last time is not a draft anyone asked for. */
function reset(): void {
  form.name = ''
  form.address = ''
  form.port = 445
  form.description = ''
  form.repoHostId = null
  error.value = null
  testResult.value = null
  testError.value = null
}

/** Best-effort: without the list the dependency is simply not shared with a repository host. */
async function loadRepoHosts(): Promise<void> {
  try {
    repoHosts.value = await listRepoHosts()
  } catch {
    repoHosts.value = []
  }
}

watch(
  () => props.open,
  (isOpen) => {
    if (!isOpen) return
    reset()
    void loadRepoHosts()
  },
  { immediate: true },
)

// A result is only true of the address and port it was asked for.
watch(
  () => [form.address, form.port],
  () => {
    testResult.value = null
    testError.value = null
  },
)

async function testConnection(): Promise<void> {
  testing.value = true
  testResult.value = null
  testError.value = null
  try {
    testResult.value = await testDependencyAddress(form.address.trim(), form.port)
  } catch (e: unknown) {
    testError.value = extractError(e)
  } finally {
    testing.value = false
  }
}

async function submit(): Promise<void> {
  submitting.value = true
  error.value = null
  try {
    const host = await createDependencyHost({
      name: form.name.trim(),
      address: form.address.trim(),
      port: form.port,
      description: form.description.trim(),
      repo_host_id: form.repoHostId,
    })
    emit('created', host)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="open"
    title="New dependency"
    form
    @close="emit('close')"
    @submit="submit"
  >
    <div class="field">
      <label
        class="field-label"
        for="dependency-name"
        >Name <span class="required">*</span></label
      >
      <input
        id="dependency-name"
        v-model="form.name"
        class="input"
        placeholder="nas-media"
        required
      />
    </div>
    <div class="field">
      <label
        class="field-label"
        for="dependency-address"
        >Address <span class="required">*</span></label
      >
      <input
        id="dependency-address"
        v-model="form.address"
        class="input mono"
        placeholder="nas-media.lan"
        required
      />
      <span class="field-hint">Hostname or IP address, as the Assimilate server reaches it.</span>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="dependency-port"
        >Check</label
      >
      <DependencyPortField
        v-model="form.port"
        input-id="dependency-port"
      />
    </div>
    <div class="field">
      <label
        class="field-label"
        for="dependency-repo-host"
        >Same machine as</label
      >
      <select
        id="dependency-repo-host"
        v-model="form.repoHostId"
        class="input"
      >
        <option :value="null">Not a repository host</option>
        <option
          v-for="repoHost in repoHosts"
          :key="repoHost.id"
          :value="repoHost.id"
        >
          Repository host {{ repoHost.ssh_host }}
        </option>
      </select>
      <span class="field-hint">
        Shares that host's wake settings, and keeps it from being shut down while a backup still
        needs this dependency.
      </span>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="dependency-description"
        >Description</label
      >
      <input
        id="dependency-description"
        v-model="form.description"
        class="input"
        placeholder="Optional"
      />
    </div>
    <p class="field-hint">
      Wake-on-LAN and "Host is not always online" are set on the dependency's Power pane after it is
      created.
    </p>

    <div class="dependency-test">
      <button
        type="button"
        class="btn btn-sm"
        :disabled="testing || !form.address.trim()"
        @click="testConnection"
      >
        <RefreshCw
          :size="14"
          :class="{ spinning: testing }"
        />
        {{ testing ? 'Testing...' : 'Test connection' }}
      </button>
      <p
        v-if="testResult?.reachable"
        class="form-success"
      >
        {{ testResultText(testResult) }}
      </p>
      <p
        v-else-if="testResult"
        class="state-msg state-msg--inline state-warning"
      >
        {{ testResultText(testResult) }}. You can still add it.
      </p>
      <p
        v-else-if="testError"
        class="form-error"
      >
        {{ testError }}
      </p>
    </div>

    <template #footer>
      <ModalFormActions
        :submitting="submitting"
        :disabled="!form.name.trim() || !form.address.trim()"
        :error="error"
        submit-label="Create dependency"
        submitting-label="Creating..."
        @cancel="emit('close')"
      />
    </template>
  </BaseModal>
</template>

<style scoped>
.dependency-test {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-3);
}
</style>

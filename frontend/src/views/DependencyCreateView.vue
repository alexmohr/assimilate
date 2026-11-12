<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { RefreshCw } from '@lucide/vue'
import BaseWizard, { type WizardStep } from '../components/BaseWizard.vue'
import DependencyPortField from '../components/DependencyPortField.vue'
import { createDependencyHost, testDependencyAddress } from '../api/dependencyHosts'
import { listRepoHosts, type RepoHost } from '../api/repoHosts'
import { protocolLabel, testResultText } from '../utils/dependencyHost'
import { extractError } from '../utils/error'
import type { DependencyTestResponse } from '../types/generated'

/**
 * A new dependency, on the same wizard as a new schedule: what it is called
 * and where the Assimilate server reaches it, which port says it is up (with
 * a test of that address), whether it is the same machine as a repository
 * host, and a review. Wake-on-LAN and "not always online" are left to its
 * Power pane, where they are easier to get right once the address works.
 */
type StepId = 'host' | 'check' | 'machine' | 'review'

const STEPS: readonly WizardStep<StepId>[] = [
  { id: 'host', label: 'Host', sub: 'Name and address' },
  { id: 'check', label: 'Check', sub: 'How it is asked' },
  { id: 'machine', label: 'Machine', sub: 'Shared with a host' },
  { id: 'review', label: 'Review', sub: 'Create dependency' },
]

const router = useRouter()

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

/** Best-effort: without the list the dependency is simply not shared with a repository host. */
onMounted(async () => {
  try {
    repoHosts.value = await listRepoHosts()
  } catch {
    repoHosts.value = []
  }
})

// A result is only true of the address and port it was asked for.
watch(
  () => [form.address, form.port],
  () => {
    testResult.value = null
    testError.value = null
  },
)

function blockers(step: StepId): string[] {
  const missing: string[] = []
  if (step === 'host') {
    if (!form.name.trim()) missing.push('a name')
    if (!form.address.trim()) missing.push('an address')
  }
  return missing
}

function repoHostLabel(id: number | null): string {
  if (id === null) return 'Not a repository host'
  const host = repoHosts.value.find((h) => h.id === id)
  return host ? `Repository host ${host.ssh_host}` : `Repository host #${id}`
}

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
    router.push(`/dependency-hosts/${host.id}`)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="dependency-create-view">
    <div class="detail-breadcrumb">
      <RouterLink
        to="/dependencies"
        class="crumb-link"
        >Dependencies</RouterLink
      >
      <span class="crumb-sep">/</span>
      <span class="crumb-current">New</span>
    </div>

    <div class="page-header">
      <h1 class="page-title">New Dependency</h1>
    </div>
    <p class="page-description">
      A machine a backup needs besides its agent and its repository, such as the NAS whose share a
      pre-backup command mounts.
    </p>

    <BaseWizard
      v-slot="{ step, goTo }"
      :steps="STEPS"
      :blockers="blockers"
      label="Dependency steps"
      submit-label="Create dependency"
      :submitting="submitting"
      @cancel="router.push('/dependencies')"
      @submit="submit"
    >
      <template v-if="step === 'host'">
        <p class="pane-lede">What it is called and where the Assimilate server reaches it.</p>
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
          />
          <span class="field-hint"
            >Hostname or IP address, as the Assimilate server reaches it.</span
          >
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
      </template>

      <template v-else-if="step === 'check'">
        <p class="pane-lede">
          The port that says it is up. Before a backup that needs it, Assimilate asks this port and
          waits for an answer.
        </p>
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
      </template>

      <template v-else-if="step === 'machine'">
        <p class="pane-lede">
          Whether it is the same machine as a repository host. It then shares that host's wake
          settings, and the host is not shut down while a backup still needs this dependency.
        </p>
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
        </div>
        <p class="wizard-note">
          Wake-on-LAN and "Host is not always online" are set on the dependency's Power section once
          it exists.
        </p>
      </template>

      <template v-else>
        <p class="pane-lede">
          What will be created. Every block links back to the step that owns it.
        </p>
        <div class="review-grid">
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Host</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('host')"
              >
                Edit
              </button>
            </div>
            <dl class="info-grid">
              <dt>Name</dt>
              <dd>{{ form.name || '—' }}</dd>
              <dt>Address</dt>
              <dd class="mono">{{ form.address || '—' }}</dd>
              <template v-if="form.description">
                <dt>Description</dt>
                <dd>{{ form.description }}</dd>
              </template>
            </dl>
          </div>
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Check</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('check')"
              >
                Edit
              </button>
            </div>
            <dl class="info-grid">
              <dt>Asks</dt>
              <dd>{{ protocolLabel(form.port) }} · port {{ form.port }}</dd>
              <dt>Last test</dt>
              <dd>{{ testResult ? testResultText(testResult) : 'Not tested' }}</dd>
            </dl>
          </div>
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Machine</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('machine')"
              >
                Edit
              </button>
            </div>
            <dl class="info-grid">
              <dt>Same machine as</dt>
              <dd>{{ repoHostLabel(form.repoHostId) }}</dd>
            </dl>
          </div>
        </div>
        <div
          v-if="error"
          class="form-error"
        >
          {{ error }}
        </div>
      </template>
    </BaseWizard>
  </div>
</template>

<style scoped>
.dependency-create-view {
  max-width: 1000px;
  min-width: 0;
}

.dependency-test {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-4);
}
</style>

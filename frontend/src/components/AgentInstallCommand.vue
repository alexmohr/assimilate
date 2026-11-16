<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useClipboard } from '../composables/useClipboard'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import type { InstallTarget } from '../types/enrollment'

// MOCK: the token, key and expiry are placeholders until the enrollment API exists.
defineProps<{ generated: boolean }>()

const target = ref<InstallTarget>('linux')
const expiry = ref('3600')
const hostname = ref('')
const tags = ref('')
const { copied, copy } = useClipboard()

const targets: readonly SegmentedOption<InstallTarget>[] = [
  { value: 'linux', label: 'Linux' },
  { value: 'windows', label: 'Windows' },
]

const SERVER = 'https://backup.example.com'
const TOKEN = 'enr_Zk3qT0mV9xLw2bYhN6cD8sAeJ4uPfR9w'
const SERVER_KEY = '7Qm1xB0tV3kLhYw8N2dCzEoPaU5sRjGf4HiMnXpTcA0='

const command = computed(() =>
  target.value === 'linux'
    ? `curl -fsSL ${SERVER}/install.sh | sudo sh -s -- \\\n  --token ${TOKEN} \\\n  --server-key ${SERVER_KEY}`
    : `& ([scriptblock]::Create((irm ${SERVER}/install.ps1))) \`\n  -Token ${TOKEN} \`\n  -ServerKey ${SERVER_KEY}`,
)

const steps = [
  'Detects the OS and architecture, downloads the agent and verifies its SHA-256.',
  'Installs the service and writes the config with 0600 permissions.',
  'Pins the server key from the command, then enrolls and starts the agent.',
]
</script>

<template>
  <div
    v-if="!generated"
    class="form-stack"
  >
    <p class="field-hint install-intro">
      Run one command on the host. It downloads the agent, installs the service and enrolls itself.
      Use this when the server cannot reach the host over SSH.
    </p>
    <div class="field">
      <label
        class="field-label"
        for="enroll-expiry"
        >Expires after</label
      >
      <select
        id="enroll-expiry"
        v-model="expiry"
        class="input"
      >
        <option value="900">15 minutes</option>
        <option value="3600">1 hour</option>
        <option value="86400">24 hours</option>
      </select>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="enroll-hostname"
        >Restrict to hostname</label
      >
      <input
        id="enroll-hostname"
        v-model="hostname"
        class="input"
        placeholder="Optional, e.g. workstation-01"
      />
      <span class="field-hint">Only a host with this exact hostname can use the token.</span>
    </div>
    <div class="field">
      <label
        class="field-label"
        for="enroll-tags"
        >Tags applied on enrollment</label
      >
      <input
        id="enroll-tags"
        v-model="tags"
        class="input"
        placeholder="Optional, e.g. servers, eu"
      />
    </div>
  </div>

  <div
    v-else
    class="token-notice"
  >
    <div class="install-os">
      <BaseSegmented
        v-model="target"
        :options="targets"
        label="Operating system"
      />
    </div>
    <p class="token-warning">
      Copy this command now. The token is single-use and is not shown again.
    </p>
    <div class="token-box token-box--block">
      <code
        class="token-text token-text--plain"
        data-testid="install-command"
        >{{ command }}</code
      >
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        @click="copy(command)"
      >
        {{ copied ? 'Copied!' : 'Copy' }}
      </button>
    </div>
    <div class="install-meta">
      <span class="badge badge--warning">Expires in 59:41</span>
      <span class="badge badge--neutral">{{ hostname.trim() || 'Any hostname' }}</span>
      <span
        v-if="tags.trim()"
        class="badge badge--neutral"
        >Tags: {{ tags.trim() }}</span
      >
    </div>
    <ol class="field-hint install-steps">
      <li
        v-for="step in steps"
        :key="step"
      >
        {{ step }}
      </li>
    </ol>
  </div>
</template>

<style scoped>
.install-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  margin-block: var(--space-5);
}

.install-intro {
  margin-top: var(--space-5);
}

.install-os {
  margin-block: var(--space-5);
}

.install-steps {
  margin: 0;
  padding-left: var(--space-8);
  list-style: decimal;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { getAgentChannelKey } from '../api/system'
import { listTags } from '../api/tags'
import { LIFETIMES } from '../composables/useInstallCommand'
import { extractError } from '../utils/error'
import {
  AUTOSTART_NOTES,
  installCommand,
  manualSteps,
  type InstallParams,
} from '../utils/installCommand'
import BaseDisclosure from './BaseDisclosure.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import CommandBlock from './CommandBlock.vue'
import EnrollmentTokenList from './EnrollmentTokenList.vue'
import type { CreateEnrollmentTokenResponse } from '../types/generated'
import type { InstallTarget } from '../types/enrollment'
import type { TagRow } from '../types/tag'

/**
 * The Add agent dialog's Install command tab: the form for a new enrollment
 * token, then the command that installs and enrolls an agent with it.
 */
const props = defineProps<{
  /** The token just created, once there is one. */
  created: CreateEnrollmentTokenResponse | null
  /** Why creating or revoking a token failed. */
  error: string | null
  /** Read the token list again whenever this changes. */
  version: number
}>()

const lifetime = defineModel<number>('lifetime', { required: true })
const hostname = defineModel<string>('hostname', { required: true })
const tagIds = defineModel<number[]>('tagIds', { required: true })

const target = ref<InstallTarget>('linux')
const serverKey = ref('')
const agentTags = ref<TagRow[]>([])
const loadError = ref<string | null>(null)
const now = ref(Date.now())

const targets: readonly SegmentedOption<InstallTarget>[] = [
  { value: 'linux', label: 'Linux' },
  { value: 'macos', label: 'macOS' },
  { value: 'windows', label: 'Windows' },
]

const steps = [
  'Detects the OS and architecture, downloads the agent and verifies its SHA-256.',
  'Trades the single-use token for the agent token, which only root can read.',
  'Pins the server key from the command, then installs the service and starts the agent.',
]

const params = computed<InstallParams | null>(() =>
  props.created && serverKey.value
    ? {
        server: window.location.origin,
        token: props.created.plaintext,
        serverKey: serverKey.value,
      }
    : null,
)

const command = computed(() => (params.value ? installCommand(target.value, params.value) : ''))
const manual = computed(() => (params.value ? manualSteps(target.value, params.value) : []))

const MS_PER_SECOND = 1000
const SECONDS_PER_MINUTE = 60

/** Time left on the token as `m:ss`, or null once it has run out. */
const remaining = computed<string | null>(() => {
  const expiresAt = props.created?.token.expires_at
  if (!expiresAt) return null
  const seconds = Math.floor((Date.parse(expiresAt) - now.value) / MS_PER_SECOND)
  if (seconds <= 0) return null
  const rest = String(seconds % SECONDS_PER_MINUTE).padStart(2, '0')
  return `${Math.floor(seconds / SECONDS_PER_MINUTE)}:${rest}`
})

function toggleTag(id: number): void {
  tagIds.value = tagIds.value.includes(id)
    ? tagIds.value.filter((t) => t !== id)
    : [...tagIds.value, id]
}

let timer: ReturnType<typeof setInterval> | undefined

onMounted(async () => {
  timer = setInterval(() => {
    now.value = Date.now()
  }, MS_PER_SECOND)
  try {
    const [key, tags] = await Promise.all([getAgentChannelKey(), listTags('agent')])
    serverKey.value = key.public_key
    agentTags.value = tags
  } catch (e: unknown) {
    loadError.value = extractError(e, 'Failed to load the server key')
  }
})

onUnmounted(() => clearInterval(timer))
</script>

<template>
  <div
    v-if="!created"
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
        v-model.number="lifetime"
        class="input"
      >
        <option
          v-for="option in LIFETIMES"
          :key="option.value"
          :value="option.value"
        >
          {{ option.label }}
        </option>
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
    <div
      v-if="agentTags.length > 0"
      class="field"
    >
      <span class="field-label">Tags applied on enrollment</span>
      <div class="install-tags">
        <label
          v-for="tag in agentTags"
          :key="tag.id"
          class="toggle-row-label"
        >
          <input
            type="checkbox"
            :checked="tagIds.includes(tag.id)"
            @change="toggleTag(tag.id)"
          />
          {{ tag.name }}
        </label>
      </div>
    </div>
    <div
      v-if="error || loadError"
      class="form-error"
    >
      {{ error ?? loadError }}
    </div>
    <EnrollmentTokenList :version="version" />
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
    <CommandBlock
      :text="command"
      test-id="install-command"
    />
    <div class="install-meta">
      <span
        v-if="remaining"
        class="badge badge--warning"
        data-testid="install-expiry"
        >Expires in {{ remaining }}</span
      >
      <span
        v-else
        class="badge badge--danger"
        >Expired</span
      >
      <span class="badge badge--neutral">{{ created.token.hostname ?? 'Any hostname' }}</span>
    </div>
    <ol class="field-hint install-steps">
      <li
        v-for="step in steps"
        :key="step"
      >
        {{ step }}
      </li>
    </ol>
    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>
    <BaseDisclosure title="Install manually">
      <div class="install-manual">
        <p class="field-hint">
          Use these steps instead of the one-line command, for example on a host that cannot pipe a
          script into a shell.
        </p>
        <CommandBlock
          v-for="step in manual"
          :key="step.label"
          :label="step.label"
          :text="step.text"
        />
        <p class="field-hint">{{ AUTOSTART_NOTES[target] }}</p>
      </div>
    </BaseDisclosure>
    <EnrollmentTokenList :version="version" />
  </div>
</template>

<style scoped>
.install-intro {
  margin-top: var(--space-5);
}

.install-tags {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-5);
}

.install-manual {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  padding: var(--space-5);
}

.install-os {
  margin-block: var(--space-5);
}

.install-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  margin-block: var(--space-5);
}

.install-steps {
  margin: 0;
  padding-left: var(--space-8);
  list-style: decimal;
}
</style>

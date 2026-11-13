<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { Check, X } from '@lucide/vue'
import type { DeployJobState, DeployStep, DeployStepResponse } from '../types/generated'
import BaseSpinner from './BaseSpinner.vue'

const props = defineProps<{
  steps: DeployStepResponse[]
  state: DeployJobState
}>()

/** What each step is called while it is ahead or under way, and once it is done. */
const STEP_LABELS: Record<DeployStep, { active: string; done: string }> = {
  connect: { active: 'Connecting over SSH', done: 'Connected over SSH' },
  upload: { active: 'Uploading the binary', done: 'Uploaded the binary' },
  install: {
    active: 'Installing the binary and the new agent token',
    done: 'Installed the binary and the new agent token',
  },
  restart: { active: 'Restarting the agent service', done: 'Restarted the agent service' },
  reconnect: { active: 'Waiting for the agent to reconnect', done: 'Agent reconnected' },
}

function label(step: DeployStepResponse): string {
  const labels = STEP_LABELS[step.step]
  return step.status === 'done' ? labels.done : labels.active
}

function took(step: DeployStepResponse): string | null {
  if (step.duration_ms === null) return null
  return `${(step.duration_ms / 1000).toFixed(1)}s`
}

/**
 * What a failure leaves behind: up to the restart the host still runs what it
 * did, afterwards the new binary and token are already in place.
 */
const aftermath = computed<string | null>(() => {
  const failed = props.steps.findIndex((s) => s.status === 'failed')
  if (failed === -1) return null
  const restart = props.steps.findIndex((s) => s.step === 'restart')
  return failed < restart
    ? 'Nothing was restarted, so the agent keeps running the version it had.'
    : 'The new binary and agent token are already on the host.'
})
</script>

<template>
  <ul class="deploy-steps">
    <li
      v-for="step in steps"
      :key="step.step"
      class="deploy-step"
      :class="{ 'deploy-step--pending': step.status === 'pending' }"
    >
      <span class="deploy-step-icon">
        <Check
          v-if="step.status === 'done'"
          :size="16"
          class="deploy-step-icon--done"
        />
        <BaseSpinner
          v-else-if="step.status === 'running'"
          size="sm"
          :label="label(step)"
        />
        <X
          v-else-if="step.status === 'failed'"
          :size="16"
          class="deploy-step-icon--failed"
        />
        <span
          v-else
          class="status-dot"
        />
      </span>
      <span>{{ label(step) }}</span>
      <span
        v-if="took(step)"
        class="deploy-step-time"
        >{{ took(step) }}</span
      >
    </li>
  </ul>
  <div
    v-if="state === 'running'"
    class="progress-track"
  >
    <div class="progress-bar progress-bar--indeterminate" />
  </div>
  <p
    v-if="state === 'running'"
    class="deploy-steps-note"
  >
    The deploy runs on the server. You can close this window and it will carry on.
  </p>
  <p
    v-else-if="aftermath"
    class="deploy-steps-note"
  >
    {{ aftermath }}
  </p>
</template>

<style scoped>
.deploy-steps {
  list-style: none;
  margin: 0 0 var(--space-5);
  padding: 0;
  border-top: 1px solid var(--border);
}

.deploy-step {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: var(--space-4) 0;
  font-size: var(--fs-base);
  color: var(--text-primary);
}

.deploy-step--pending {
  color: var(--text-muted);
}

.deploy-step-icon {
  display: flex;
  justify-content: center;
  width: var(--space-7);
  flex: none;
}

.deploy-step-icon--done {
  color: var(--success);
}

.deploy-step-icon--failed {
  color: var(--danger);
}

.deploy-step-time {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.deploy-steps-note {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  margin: var(--space-5) 0 0;
}
</style>

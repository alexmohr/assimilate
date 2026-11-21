<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { AlertTriangle } from '@lucide/vue'
import { checkAgentBorg } from '../api/agents'
import { borgProblem } from '../utils/borgStatus'
import { extractError } from '../utils/error'
import type { AgentRow } from '../types/agent'

/**
 * The warning on a host whose agent cannot run borg: what it looked for, how
 * to fix it, and a button to look again. The agent answers over its
 * connection, and the page reloads the agent when that answer lands, so the
 * banner goes away by itself once borg is there.
 */
const props = defineProps<{
  agent: AgentRow
  /** Whether the viewer may ask the agent to look again. */
  canCheck: boolean
}>()

const problem = computed(() => borgProblem(props.agent.borg))
const checking = ref(false)
const error = ref<string | null>(null)

// A new answer ends the wait, whether or not it changed anything.
watch(
  () => props.agent.borg,
  () => {
    checking.value = false
  },
)

async function checkAgain(): Promise<void> {
  checking.value = true
  error.value = null
  try {
    await checkAgentBorg(props.agent.hostname, props.agent.domain)
  } catch (e: unknown) {
    error.value = extractError(e)
    checking.value = false
  }
}
</script>

<template>
  <div
    v-if="problem"
    class="callout callout--warning"
    role="alert"
    data-testid="borg-notice"
  >
    <AlertTriangle :size="16" />
    <div class="borg-notice-body">
      <p>
        <strong>{{ problem.heading }}</strong>
      </p>
      <p>{{ problem.detail }}</p>
      <ul class="borg-notice-steps">
        <li>
          Install borg on this host, for example
          <code class="mono">apt install borgbackup</code>.
        </li>
        <li>
          Or set <code class="mono">BORG_BINARY</code> in the agent's environment to the full path
          of the executable.
        </li>
      </ul>
      <p
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </p>
      <div
        v-if="canCheck"
        class="borg-notice-actions"
      >
        <button
          type="button"
          class="btn btn-sm"
          :disabled="checking || !agent.is_connected"
          @click="checkAgain"
        >
          {{ checking ? 'Checking...' : 'Check again' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.borg-notice-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}

.borg-notice-steps {
  margin: 0;
  padding-left: var(--space-8);
}

.borg-notice-actions {
  display: flex;
  justify-content: flex-end;
}
</style>

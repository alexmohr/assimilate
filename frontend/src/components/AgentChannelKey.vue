<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { getAgentChannelKey } from '../api/system'
import { useClipboard } from '../composables/useClipboard'
import { extractError } from '../utils/error'
import type { AgentChannelKeyResponse } from '../types/generated'

/**
 * The server key an agent pins as `BORG_SERVER_KEY`, with a copy button and
 * the fingerprint agents log, so the two can be compared.
 */
const key = ref<AgentChannelKeyResponse | null>(null)
const error = ref('')
const { copied, copy } = useClipboard()

onMounted(async () => {
  try {
    key.value = await getAgentChannelKey()
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load the server key')
  }
})
</script>

<template>
  <div class="field">
    <span class="field-label">Server key</span>
    <div
      v-if="error"
      class="state-msg state-msg--inline state-error"
    >
      {{ error }}
    </div>
    <template v-else-if="key">
      <div class="token-box token-box--block">
        <code
          class="token-text token-text--plain"
          data-testid="agent-channel-key"
          >{{ key.public_key }}</code
        >
        <button
          type="button"
          class="btn btn-sm btn-ghost"
          @click="copy(key.public_key)"
        >
          {{ copied ? 'Copied!' : 'Copy' }}
        </button>
      </div>
      <span class="field-hint">
        Set it as <code>BORG_SERVER_KEY</code> on the agent. Its fingerprint is
        <code class="mono">{{ key.fingerprint }}</code
        >, as the agent logs it when it connects.
      </span>
    </template>
  </div>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import type { AppendOnlyLoginResponse, AppendOnlySetupFailure } from '../types/generated'

/**
 * Shown only when the server's own key cannot edit the host's
 * `authorized_keys`: asks for the host's SSH password once. The password goes
 * to the server for this one request and is never stored.
 */
defineProps<{
  failure: AppendOnlySetupFailure
  login: AppendOnlyLoginResponse
  busy: boolean
}>()

const emit = defineEmits<{ submit: [password: string] }>()

const password = ref('')

function submit(): void {
  if (password.value === '') return
  const value = password.value
  // Dropped before the request goes out, so it is not held in the field.
  password.value = ''
  emit('submit', value)
}
</script>

<template>
  <form
    class="panel"
    @submit.prevent="submit"
  >
    <h3 class="panel-title">Assimilate cannot log in</h3>
    <p class="field-hint password-lede">
      {{ failure.message }} Enter the SSH password for
      <span class="mono">{{ login.ssh_user }}@{{ login.ssh_host }}</span> to continue. It is used
      once and not stored.
    </p>
    <div class="panel-actions">
      <input
        v-model="password"
        class="input"
        type="password"
        autocomplete="off"
        aria-label="SSH password"
        :placeholder="`Password for ${login.ssh_user}@${login.ssh_host}`"
      />
      <button
        type="submit"
        class="btn btn-primary btn-sm"
        :disabled="busy || password === ''"
      >
        {{ busy ? 'Working...' : 'Set up' }}
      </button>
    </div>
  </form>
</template>

<style scoped>
.password-lede {
  margin: var(--space-4) 0;
}
</style>

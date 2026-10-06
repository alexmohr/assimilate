<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import BaseModal from './BaseModal.vue'
import ModalFormActions from './ModalFormActions.vue'
import { setRepoPassphrase } from '../api/repos'
import { extractError } from '../utils/error'

/**
 * Records the passphrase the repository's key already has - the step a
 * repository created by a config import waits on, since passphrases are never
 * exported. borg checks it before the server stores it, so a typo comes back
 * as an error here rather than as a failed sync later.
 */
const props = defineProps<{ open: boolean; repoId: number }>()

const emit = defineEmits<{
  close: []
  saved: []
}>()

const passphrase = ref('')
const submitting = ref(false)
const error = ref<string | null>(null)

// The plaintext lives only as long as the dialog is open.
watch(
  () => props.open,
  () => {
    passphrase.value = ''
    error.value = null
  },
)

async function submit(): Promise<void> {
  submitting.value = true
  error.value = null
  try {
    await setRepoPassphrase(props.repoId, passphrase.value)
    passphrase.value = ''
    emit('saved')
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
    title="Set passphrase"
    size="sm"
    form
    @close="emit('close')"
    @submit="submit"
  >
    <div class="field">
      <label
        class="field-label"
        for="repo-set-passphrase"
        >Passphrase <span class="required">*</span></label
      >
      <input
        id="repo-set-passphrase"
        v-model="passphrase"
        class="input"
        type="password"
        autocomplete="off"
        placeholder="Repository encryption passphrase"
        required
      />
      <span class="field-hint"
        >The passphrase the repository was initialized with. It is checked against the repository
        before it is saved; the repository key itself is not changed.</span
      >
    </div>

    <template #footer>
      <ModalFormActions
        :submitting="submitting"
        :disabled="!passphrase"
        :error="error"
        submit-label="Save passphrase"
        submitting-label="Checking..."
        @cancel="emit('close')"
      />
    </template>
  </BaseModal>
</template>

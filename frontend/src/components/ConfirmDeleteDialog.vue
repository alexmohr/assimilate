<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import BaseModal from './BaseModal.vue'

withDefaults(
  defineProps<{
    show: boolean
    title: string
    submitting: boolean
    error?: string | null
    /** Verb shown on the confirm button. Defaults to a delete. */
    confirmLabel?: string
    submittingLabel?: string
  }>(),
  { error: null, confirmLabel: 'Delete', submittingLabel: 'Deleting...' },
)

const emit = defineEmits<{ cancel: []; confirm: [] }>()
</script>

<template>
  <BaseModal
    :open="show"
    :title="title"
    @close="emit('cancel')"
  >
    <p class="confirm-text">
      <slot />
    </p>
    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>
    <template #footer>
      <button
        type="button"
        class="btn btn-ghost"
        @click="emit('cancel')"
      >
        Cancel
      </button>
      <button
        type="button"
        class="btn btn-danger"
        :disabled="submitting"
        @click="emit('confirm')"
      >
        {{ submitting ? submittingLabel : confirmLabel }}
      </button>
    </template>
  </BaseModal>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
withDefaults(
  defineProps<{
    submitting: boolean
    disabled?: boolean
    error?: string | null
    submitLabel: string
    submittingLabel: string
    /** 'button' for handlers invoked directly; 'submit' relies on BaseModal's `form` mode. */
    type?: 'submit' | 'button'
  }>(),
  { disabled: false, error: null, type: 'submit' },
)

const emit = defineEmits<{ cancel: []; confirm: [] }>()
</script>

<template>
  <!-- Goes in BaseModal's #footer slot, which supplies the button row. The
       error takes a full row of its own above the buttons. -->
  <div
    v-if="error"
    class="form-error"
  >
    {{ error }}
  </div>
  <button
    type="button"
    class="btn btn-ghost"
    @click="emit('cancel')"
  >
    Cancel
  </button>
  <button
    :type="type"
    class="btn btn-primary"
    :disabled="disabled || submitting"
    @click="type === 'button' ? emit('confirm') : undefined"
  >
    {{ submitting ? submittingLabel : submitLabel }}
  </button>
</template>

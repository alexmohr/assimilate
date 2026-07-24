<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
defineProps<{
  error: string | null
  submitting: boolean
  submitText: string
  cancelText?: string
  disabled?: boolean
  variant?: 'primary' | 'danger'
}>()

defineEmits<{
  cancel: []
}>()
</script>

<template>
  <div
    v-if="error"
    class="modal-error"
  >
    {{ error }}
  </div>
  <div class="modal-actions">
    <button
      type="button"
      class="btn btn-ghost"
      @click="$emit('cancel')"
    >
      {{ cancelText || 'Cancel' }}
    </button>
    <button
      type="submit"
      :class="['btn', variant === 'danger' ? 'btn-danger' : 'btn-primary']"
      :disabled="submitting || disabled"
    >
      {{ submitting ? `${submitText}...` : submitText }}
    </button>
  </div>
</template>

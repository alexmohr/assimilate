<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
withDefaults(
  defineProps<{
    show: boolean
    title: string
    submitting: boolean
    error?: string | null
  }>(),
  { error: null },
)

const emit = defineEmits<{ cancel: []; confirm: [] }>()
</script>

<template>
  <div
    v-if="show"
    class="overlay"
    @click.self="emit('cancel')"
  >
    <div class="dialog">
      <div class="dialog-header">
        <h2 class="dialog-title">{{ title }}</h2>
        <button
          class="close-btn"
          @click="emit('cancel')"
        >
          &times;
        </button>
      </div>
      <div class="dialog-body">
        <p class="confirm-text">
          <slot />
        </p>
        <div
          v-if="error"
          class="form-error"
        >
          {{ error }}
        </div>
      </div>
      <div class="dialog-footer">
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
          {{ submitting ? 'Deleting...' : 'Delete' }}
        </button>
      </div>
    </div>
  </div>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * An info card that flips between a read-only view and an inline edit form.
 *
 * The read/edit/error/Cancel-Save shell was repeated verbatim for every
 * editable settings card; only the two bodies ever differed.
 */
defineProps<{
  title: string
  /** Whether the edit form is showing. Owned by the parent. */
  editing: boolean
  /** Hides the Edit button entirely, e.g. for read-only imported hosts. */
  canEdit?: boolean
  saving?: boolean
  error?: string | null
}>()

const emit = defineEmits<{
  edit: []
  cancel: []
  save: []
}>()
</script>

<template>
  <div class="info-card">
    <h3 class="info-title">{{ title }}</h3>
    <template v-if="!editing">
      <slot name="view" />
      <span
        v-if="$slots.hint"
        class="field-hint"
      >
        <slot name="hint" />
      </span>
      <div class="info-actions">
        <button
          v-if="canEdit"
          class="btn btn-sm btn-ghost"
          @click="emit('edit')"
        >
          Edit
        </button>
      </div>
    </template>
    <template v-else>
      <slot name="edit" />
      <div
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </div>
      <div class="info-actions">
        <button
          class="btn btn-sm btn-ghost"
          :disabled="saving"
          @click="emit('cancel')"
        >
          Cancel
        </button>
        <button
          class="btn btn-sm btn-primary"
          :disabled="saving"
          @click="emit('save')"
        >
          {{ saving ? 'Saving...' : 'Save' }}
        </button>
      </div>
    </template>
  </div>
</template>

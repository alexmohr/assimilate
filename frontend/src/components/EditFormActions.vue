<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The footer of an inline edit form inside an `.info-card`: the failure line
 * and the Cancel/Save pair.
 *
 * `EditableInfoCard` owns this for cards whose header is nothing but a title.
 * The quota and repository-overview cards drive their own `.info-card-header`
 * because they carry extra actions up there, so they keep the header and share
 * only the footer - which they had a verbatim copy of each.
 */
withDefaults(
  defineProps<{
    saving: boolean
    error?: string | null
    /** Overrides the idle label, e.g. "Save Changes". */
    saveLabel?: string
  }>(),
  { error: null, saveLabel: 'Save' },
)

const emit = defineEmits<{ cancel: []; save: [] }>()
</script>

<template>
  <div
    v-if="error"
    class="form-error"
  >
    {{ error }}
  </div>
  <div class="edit-actions">
    <button
      class="btn btn-ghost"
      @click="emit('cancel')"
    >
      Cancel
    </button>
    <button
      class="btn btn-primary"
      :disabled="saving"
      @click="emit('save')"
    >
      {{ saving ? 'Saving...' : saveLabel }}
    </button>
  </div>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * A settings pane that flips between a read-only view and an inline edit form.
 *
 * The read/edit/error/Cancel-Save shell was repeated verbatim for every
 * editable settings card; only the two bodies ever differed.
 *
 * It draws no heading and no panel of its own: the settings rail beside it has
 * already named the section, and a bordered card repeating that name one line
 * below the rail is what this shape used to do. The Edit button sits in the
 * pane head rather than in a footer under the body, so it stays put as the
 * body grows.
 */
import HelpHint from './HelpHint.vue'

defineProps<{
  /** The sentence that says what the section is for, disclosed behind a `HelpHint`. */
  lede?: string
  /** Accessible name for that `HelpHint`, e.g. "host power". Required whenever `lede` is set. */
  ledeLabel?: string
  /**
   * Accessible name for the `#hint` slot's `HelpHint`, e.g. "command
   * inheritance". Required whenever that slot is filled.
   */
  hintLabel?: string
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
  <div
    v-if="lede || (canEdit && !editing)"
    class="pane-head"
  >
    <HelpHint
      v-if="lede"
      :label="ledeLabel ?? 'this section'"
    >
      {{ lede }}
    </HelpHint>
    <button
      v-if="canEdit && !editing"
      class="btn btn-sm btn-ghost"
      type="button"
      @click="emit('edit')"
    >
      Edit
    </button>
  </div>
  <template v-if="!editing">
    <slot name="view" />
    <HelpHint
      v-if="$slots.hint"
      :label="hintLabel ?? 'this section'"
    >
      <slot name="hint" />
    </HelpHint>
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
        type="button"
        :disabled="saving"
        @click="emit('cancel')"
      >
        Cancel
      </button>
      <button
        class="btn btn-sm btn-primary"
        type="button"
        :disabled="saving"
        @click="emit('save')"
      >
        {{ saving ? 'Saving...' : 'Save' }}
      </button>
    </div>
  </template>
</template>

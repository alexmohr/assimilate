<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import EditableSection from './EditableSection.vue'
import FileSourcesEditor from './FileSourcesEditor.vue'
import FileSourcesView from './FileSourcesView.vue'
import { parseLines } from '../utils/validation'
import type { FileSet } from '../types/fileSet'
import type { HookCommand } from '../types/generated'

/**
 * A file set as one section: the saved values, an Edit button, the edit form,
 * Cancel and Save. A host's Sources › Files and a schedule's own files are
 * the same thing, so both render exactly this; only what Save does differs,
 * and that is the caller's.
 *
 * The form edits a copy, so Cancel leaves the saved values alone.
 */
const props = withDefaults(
  defineProps<{
    value: FileSet
    lede?: string
    canEdit?: boolean
    saving?: boolean
    error?: string | null
    /** The timeout a command without its own inherits, in seconds, if one is known. */
    defaultTimeoutSeconds?: number
    /** What that inherited timeout is, in words, for the read-only view. */
    inheritedTimeout?: string
  }>(),
  {
    lede: undefined,
    canEdit: true,
    saving: false,
    error: null,
    defaultTimeoutSeconds: undefined,
    inheritedTimeout: undefined,
  },
)

const emit = defineEmits<{ save: [value: FileSet] }>()

/** Owned by the caller only through `close()`, so a failed save keeps the form open. */
const editing = ref(false)

const paths = ref('')
const excludes = ref('')
const includes = ref('')
const fileChangePatterns = ref('')
const preCommands = ref<HookCommand[]>([])
const postCommands = ref<HookCommand[]>([])

function startEdit(): void {
  paths.value = props.value.paths
  excludes.value = props.value.excludes
  includes.value = props.value.includes
  fileChangePatterns.value = props.value.fileChangePatterns
  preCommands.value = [...props.value.preCommands]
  postCommands.value = [...props.value.postCommands]
  editing.value = true
}

function save(): void {
  emit('save', {
    paths: paths.value,
    excludes: excludes.value,
    includes: includes.value,
    fileChangePatterns: fileChangePatterns.value,
    preCommands: preCommands.value,
    postCommands: postCommands.value,
  })
}

/** The caller closes the form once its save went through. */
function close(): void {
  editing.value = false
}

defineExpose({ close, editing })
</script>

<template>
  <EditableSection
    :lede="lede"
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <FileSourcesView
        :paths="parseLines(value.paths)"
        :excludes="parseLines(value.excludes)"
        :includes="parseLines(value.includes)"
        :file-change-patterns-raw="value.fileChangePatterns"
        :pre-commands="value.preCommands"
        :post-commands="value.postCommands"
        :inherited-timeout="inheritedTimeout"
      >
        <template
          v-if="$slots.viewPaths"
          #paths
        >
          <slot name="viewPaths" />
        </template>
      </FileSourcesView>
    </template>

    <template
      v-if="$slots.hint"
      #hint
    >
      <slot name="hint" />
    </template>

    <template #edit>
      <FileSourcesEditor
        v-model:paths="paths"
        v-model:excludes="excludes"
        v-model:includes="includes"
        v-model:file-change-patterns="fileChangePatterns"
        v-model:pre-commands="preCommands"
        v-model:post-commands="postCommands"
        id-prefix="file-sources"
        :default-timeout-seconds="defaultTimeoutSeconds"
      >
        <template
          v-if="$slots.editPaths"
          #paths
        >
          <slot name="editPaths" />
        </template>
      </FileSourcesEditor>
    </template>
  </EditableSection>
</template>

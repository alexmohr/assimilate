<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import CommandListEditor from './CommandListEditor.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import type { HookCommand } from '../types/generated'

/**
 * What a backup reads as files: paths, excludes (with optional include
 * exceptions), file change patterns, and the commands that run around it.
 *
 * One form for both places that define a file set: a host's Sources › Files,
 * and a schedule that sets its own files instead of taking each host's. They
 * mean the same thing, so they look and behave the same. Textareas hold one
 * entry per line; the caller parses them.
 */
const props = withDefaults(
  defineProps<{
    /** Prefix for element ids, so two editors on one page do not clash. */
    idPrefix: string
    /** Shows the include patterns field, which only a schedule has so far. */
    withIncludes?: boolean
    /** The timeout a command without its own inherits, shown in its placeholder. */
    defaultTimeoutSeconds?: number
  }>(),
  { withIncludes: false, defaultTimeoutSeconds: undefined },
)

const paths = defineModel<string>('paths', { required: true })
const excludes = defineModel<string>('excludes', { required: true })
const includes = defineModel<string>('includes', { default: '' })
const fileChangePatterns = defineModel<string>('fileChangePatterns', { required: true })
const preCommands = defineModel<HookCommand[]>('preCommands', { required: true })
const postCommands = defineModel<HookCommand[]>('postCommands', { required: true })
</script>

<template>
  <div class="file-sources-editor">
    <div class="file-sources-field">
      <label
        class="group-label group-label--lg"
        :for="`${props.idPrefix}-paths`"
        >Backup paths</label
      >
      <!-- A caller that splits paths by host swaps the textarea for its own fields. -->
      <slot name="paths">
        <textarea
          :id="`${props.idPrefix}-paths`"
          v-model="paths"
          class="input file-sources-area"
          placeholder="Directories to back up, one per line"
          spellcheck="false"
        />
      </slot>
      <span
        v-if="$slots.pathsHint"
        class="field-hint"
        ><slot name="pathsHint"
      /></span>
    </div>

    <div class="file-sources-field">
      <label
        class="group-label group-label--lg"
        :for="`${props.idPrefix}-excludes`"
        >Exclude patterns</label
      >
      <textarea
        :id="`${props.idPrefix}-excludes`"
        v-model="excludes"
        class="input file-sources-area"
        placeholder="Exclude patterns, one per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;*/__pycache__"
        spellcheck="false"
      />
    </div>

    <div
      v-if="withIncludes"
      class="file-sources-field"
    >
      <label
        class="group-label group-label--lg"
        :for="`${props.idPrefix}-includes`"
        >Include patterns</label
      >
      <textarea
        :id="`${props.idPrefix}-includes`"
        v-model="includes"
        class="input file-sources-area"
        placeholder="Exceptions to the excludes, one per line&#10;e.g. /home/keep&#10;pp:/var/keep"
        spellcheck="false"
      />
      <span class="field-hint">
        Checked first: a path matching one of these is backed up even if an exclude covers it.
      </span>
    </div>

    <div class="file-sources-field">
      <span class="group-label group-label--lg">File change patterns</span>
      <FileChangePatternsEditor v-model="fileChangePatterns">
        <template #hint>
          Glob patterns matched against the full warning message, with actions:
          <code>ignore</code> (no warning), <code>warn</code> (default), <code>fatal</code> (fail
          backup). <code>*</code> does not match <code>/</code> - to cover every file under a
          directory, end the pattern with <code>**</code>, e.g. <code>/data/wal/**</code>.
        </template>
      </FileChangePatternsEditor>
    </div>

    <fieldset class="file-sources-cmds">
      <legend class="group-label group-label--lg">Pre-backup commands</legend>
      <CommandListEditor
        v-model="preCommands"
        placeholder="e.g. systemctl stop myapp"
        aria-label="Pre-backup commands"
        :default-timeout-seconds="defaultTimeoutSeconds"
      />
    </fieldset>

    <fieldset class="file-sources-cmds">
      <legend class="group-label group-label--lg">Post-backup commands</legend>
      <CommandListEditor
        v-model="postCommands"
        placeholder="e.g. systemctl start myapp"
        aria-label="Post-backup commands"
        :default-timeout-seconds="defaultTimeoutSeconds"
      />
    </fieldset>
  </div>
</template>

<style scoped>
.file-sources-editor {
  display: flex;
  flex-direction: column;
  gap: var(--space-7);
}

.file-sources-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
}

.file-sources-area {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

/* A native fieldset/legend groups a command list's variable-length fields
   under one accessible name, since a single `<label for>` can't target more
   than one field. Reset to blend in: without `min-width: 0`, a fieldset's
   UA-default intrinsic sizing can force it wider than its container. */
.file-sources-cmds {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
  border: 0;
  margin: 0;
  padding: 0;
  min-width: 0;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { X } from '@lucide/vue'

/**
 * A list of hook commands, each its own resizable multi-line field. One
 * command's contents used to be one line in a single shared textarea, which
 * made a real multi-statement script unreadable and impossible to write. A
 * command is still just a shell string executed via `sh -c`, so a list entry
 * may itself contain newlines - it is a whole script, not a line.
 */
defineProps<{
  placeholder?: string
}>()

const commands = defineModel<string[]>({ required: true })

function addCommand(): void {
  commands.value = [...commands.value, '']
}

function removeCommand(index: number): void {
  const next = [...commands.value]
  next.splice(index, 1)
  commands.value = next
}

function updateCommand(index: number, value: string): void {
  const next = [...commands.value]
  next[index] = value
  commands.value = next
}
</script>

<template>
  <div class="cmd-list-editor">
    <div
      v-for="(cmd, index) in commands"
      :key="index"
      class="cmd-list-row"
    >
      <textarea
        :value="cmd"
        class="input cmd-list-script"
        :placeholder="placeholder ?? 'e.g. systemctl stop myapp'"
        spellcheck="false"
        rows="1"
        @input="updateCommand(index, ($event.target as HTMLTextAreaElement).value)"
      />
      <button
        type="button"
        class="btn btn-sm btn-danger"
        title="Remove command"
        aria-label="Remove command"
        @click="removeCommand(index)"
      >
        <X :size="14" />
      </button>
    </div>
    <button
      type="button"
      class="btn btn-sm btn-ghost"
      @click="addCommand()"
    >
      + Add command
    </button>
  </div>
</template>

<style scoped>
.cmd-list-editor {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.cmd-list-row {
  display: flex;
  gap: var(--space-4);
  align-items: flex-start;
}

.cmd-list-script {
  flex: 1;
  min-height: 60px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.cmd-list-row .btn {
  flex-shrink: 0;
  margin-top: var(--space-1);
}
</style>

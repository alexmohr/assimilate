<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from '@lucide/vue'

/**
 * A list of hook commands, each its own resizable multi-line field. One
 * command's contents used to be one line in a single shared textarea, which
 * made a real multi-statement script unreadable and impossible to write. A
 * command is still just a shell string executed via `sh -c`, so a list entry
 * may itself contain newlines - it is a whole script, not a line.
 */
const props = defineProps<{
  placeholder?: string
  /** Accessible name applied to every row's field, since a single `<label
   * for>` can't target a variable-length list of textareas. */
  ariaLabel?: string
}>()

const commands = defineModel<string[]>({ required: true })

interface CommandRow {
  id: number
  value: string
}

// A plain counter rather than crypto.randomUUID(): the id only needs to be
// unique among sibling rows for Vue's :key, not globally or cryptographically
// random, and randomUUID is restricted to secure contexts (HTTPS/localhost) -
// this app documents plain-HTTP deployment as supported
// (ASSIMILATE_SECURE_COOKIES=false, docker-compose's unencrypted :8080),
// where randomUUID is undefined and would throw on every mount.
let nextRowId = 0

function makeRow(value: string): CommandRow {
  return { id: nextRowId++, value }
}

// Keyed on a stable per-row id rather than array index: with an index key,
// removing a row from the middle/start makes Vue reuse DOM nodes by
// position, force-patching the wrong textarea's content (and stealing focus
// from whichever one the user was actually editing).
const rows = ref<CommandRow[]>(commands.value.map(makeRow))

function sameContent(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index])
}

// Regenerating `rows` from `commands` always mints fresh row ids, which the
// deep watch below then sees as a change and re-emits - since an emitted
// array is never referentially equal to the array `commands` held before,
// that would otherwise loop forever. Comparing content first means an emit
// that just echoes `rows`' own current values is a no-op here, and only a
// genuinely different incoming list (e.g. switching to another schedule)
// regenerates rows - and picks up fresh ids, correctly resetting identity
// for what's now unrelated content.
watch(commands, (value) => {
  if (
    sameContent(
      value,
      rows.value.map((row) => row.value),
    )
  )
    return
  rows.value = value.map(makeRow)
})

// Guarded the same way as the watcher above: when `rows` was just
// regenerated from an incoming `commands` change, this deep watch still
// fires (the row objects are new), and mapping it straight back out would
// re-emit a referentially-new but content-identical array on every external
// reassignment. Comparing content first makes that echo a no-op.
watch(
  rows,
  (value) => {
    const next = value.map((row) => row.value)
    if (sameContent(next, commands.value)) return
    commands.value = next
  },
  { deep: true },
)

function addCommand(): void {
  rows.value = [...rows.value, makeRow('')]
}

function removeCommand(index: number): void {
  const next = [...rows.value]
  next.splice(index, 1)
  rows.value = next
}
</script>

<template>
  <div class="cmd-list-editor">
    <div
      v-for="(row, index) in rows"
      :key="row.id"
      class="cmd-list-row"
    >
      <textarea
        v-model="row.value"
        class="input cmd-list-script"
        :placeholder="placeholder ?? 'e.g. systemctl stop myapp'"
        :aria-label="props.ariaLabel ? `${props.ariaLabel} ${index + 1}` : undefined"
        spellcheck="false"
        rows="1"
      />
      <button
        type="button"
        class="btn btn-sm btn-danger"
        :title="`Remove command ${index + 1}`"
        :aria-label="`Remove command ${index + 1}`"
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

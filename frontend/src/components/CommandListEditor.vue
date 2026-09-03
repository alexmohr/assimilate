<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { X } from '@lucide/vue'
import { MAX_HOOK_COMMAND_TIMEOUT_SECONDS } from '../utils/hookCommands'
import type { HookCommand } from '../types/generated'

/**
 * A list of hook commands, each its own resizable multi-line field with its
 * own optional timeout. One command's contents used to be one line in a single
 * shared textarea, which made a real multi-statement script unreadable and
 * impossible to write. A command is still just a shell string executed via
 * `sh -c`, so a list entry may itself contain newlines - it is a whole script,
 * not a line.
 *
 * The timeout is per command because the alternative - one budget for every
 * hook on the schedule - has to be set for the slowest of them, which leaves a
 * genuinely stuck `systemctl stop` sitting for as long as a hypervisor dump
 * legitimately needs.
 */
const props = defineProps<{
  placeholder?: string
  /** Accessible name applied to every row's field, since a single `<label
   * for>` can't target a variable-length list of textareas. */
  ariaLabel?: string
  /** The schedule's hook timeout, shown as the timeout field's placeholder so
   * an empty field reads as "inherits this" rather than "no timeout". Absent
   * where the commands are an agent's defaults, which have no one schedule to
   * inherit from. */
  defaultTimeoutSeconds?: number
}>()

const commands = defineModel<HookCommand[]>({ required: true })

interface CommandRow {
  id: number
  value: string
  timeoutSeconds: number | null
}

// A plain counter rather than crypto.randomUUID(): the id only needs to be
// unique among sibling rows for Vue's :key, not globally or cryptographically
// random, and randomUUID is restricted to secure contexts (HTTPS/localhost) -
// this app documents plain-HTTP deployment as supported
// (ASSIMILATE_SECURE_COOKIES=false, docker-compose's unencrypted :8080),
// where randomUUID is undefined and would throw on every mount.
let nextRowId = 0

function makeRow(command: HookCommand): CommandRow {
  return { id: nextRowId++, value: command.command, timeoutSeconds: command.timeout_seconds }
}

function toCommand(row: CommandRow): HookCommand {
  return { command: row.value, timeout_seconds: row.timeoutSeconds }
}

// Keyed on a stable per-row id rather than array index: with an index key,
// removing a row from the middle/start makes Vue reuse DOM nodes by
// position, force-patching the wrong textarea's content (and stealing focus
// from whichever one the user was actually editing).
const rows = ref<CommandRow[]>(commands.value.map(makeRow))

function sameContent(a: HookCommand[], b: HookCommand[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (command, index) =>
        command.command === b[index].command &&
        command.timeout_seconds === b[index].timeout_seconds,
    )
  )
}

// Regenerating `rows` from `commands` always mints fresh row ids, which the
// deep watch below then sees as a change and re-emits - since an emitted
// array is never referentially equal to the array `commands` held before,
// that would otherwise loop forever. Comparing content first means an emit
// that just echoes `rows`' own current values is a no-op here, and only a
// genuinely different incoming list (e.g. switching to another schedule)
// regenerates rows.
//
// Reusing the existing row wherever its value is unchanged (rather than
// mapping every incoming value through makeRow) matters even when only one
// entry actually differs: minting a fresh id for every row - not just the
// changed one - would destroy and recreate the DOM node for every untouched
// row too, the same DOM-identity/focus-loss problem the stable per-row id
// was introduced to prevent in the first place.
watch(commands, (value) => {
  if (sameContent(value, rows.value.map(toCommand))) return
  rows.value = value.map((command, index) => {
    const existing = rows.value[index]
    const unchanged =
      existing !== undefined &&
      existing.value === command.command &&
      existing.timeoutSeconds === command.timeout_seconds
    return unchanged ? existing : makeRow(command)
  })
})

// Guarded the same way as the watcher above: when `rows` was just
// regenerated from an incoming `commands` change, this deep watch still
// fires (the row objects are new), and mapping it straight back out would
// re-emit a referentially-new but content-identical array on every external
// reassignment. Comparing content first makes that echo a no-op.
watch(
  rows,
  (value) => {
    const next = value.map(toCommand)
    if (sameContent(next, commands.value)) return
    commands.value = next
  },
  { deep: true },
)

function addCommand(): void {
  rows.value = [...rows.value, makeRow({ command: '', timeout_seconds: null })]
}

function removeCommand(index: number): void {
  const next = [...rows.value]
  next.splice(index, 1)
  rows.value = next
}

/** An emptied or unparseable field means "inherit", not "no timeout". */
function updateTimeout(row: CommandRow, event: Event): void {
  const raw = (event.target as HTMLInputElement).value.trim()
  const parsed = Number.parseInt(raw, 10)
  row.timeoutSeconds = Number.isNaN(parsed) ? null : parsed
}
</script>

<template>
  <div class="cmd-list-editor">
    <!-- With no rows, the old single shared textarea's placeholder was still
         visible; a still-empty list here would otherwise show no guidance at
         all until "+ Add command" is clicked. -->
    <p
      v-if="rows.length === 0"
      class="field-hint"
    >
      {{ placeholder ?? 'e.g. systemctl stop myapp' }}
    </p>
    <div
      v-for="(row, index) in rows"
      :key="row.id"
      class="cmd-list-row"
    >
      <div class="cmd-list-body">
        <textarea
          v-model="row.value"
          class="input cmd-list-script"
          :placeholder="placeholder ?? 'e.g. systemctl stop myapp'"
          :aria-label="props.ariaLabel ? `${props.ariaLabel} ${index + 1}` : undefined"
          spellcheck="false"
          rows="1"
        />
        <div class="cmd-list-timeout">
          <span class="field-hint">Timeout (seconds)</span>
          <input
            :value="row.timeoutSeconds ?? ''"
            type="number"
            min="1"
            :max="MAX_HOOK_COMMAND_TIMEOUT_SECONDS"
            class="input input-sm cmd-list-timeout-input"
            :placeholder="
              defaultTimeoutSeconds ? String(defaultTimeoutSeconds) : 'schedule default'
            "
            :aria-label="
              props.ariaLabel ? `${props.ariaLabel} ${index + 1} timeout in seconds` : undefined
            "
            @input="(event) => updateTimeout(row, event)"
          />
          <span class="field-hint">Leave empty to use the schedule's hook command timeout.</span>
        </div>
      </div>
      <button
        type="button"
        class="btn btn-sm btn-danger"
        :title="`Remove ${props.ariaLabel ?? 'command'} ${index + 1}`"
        :aria-label="`Remove ${props.ariaLabel ?? 'command'} ${index + 1}`"
        @click="removeCommand(index)"
      >
        <X :size="14" />
      </button>
    </div>
    <button
      type="button"
      class="btn btn-sm btn-ghost"
      :aria-label="props.ariaLabel ? `+ Add command (${props.ariaLabel})` : undefined"
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

.cmd-list-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.cmd-list-script {
  min-height: 60px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.cmd-list-timeout {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.cmd-list-timeout-input {
  width: 9rem;
  flex: none;
}

.cmd-list-row .btn {
  flex-shrink: 0;
  margin-top: var(--space-1);
}
</style>

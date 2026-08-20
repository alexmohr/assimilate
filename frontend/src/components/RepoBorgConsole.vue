<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { execRepoCommand } from '../api/repos'
import { extractError } from '../utils/error'
import type { ExecBorgResponse } from '../types/generated'

const props = defineProps<{ repoId: number }>()

/** Suggestions, not a whitelist - the server decides what it will run. */
const SUGGESTED_COMMANDS = [
  'info',
  'list',
  'check',
  'compact',
  'prune',
  'delete',
  'diff',
  'rename',
  'recreate',
] as const

const command = ref('')
const loading = ref(false)
const error = ref<string | null>(null)
const result = ref<ExecBorgResponse | null>(null)

async function run(): Promise<void> {
  const trimmed = command.value.trim()
  if (!trimmed) return
  loading.value = true
  error.value = null
  result.value = null
  try {
    const args = trimmed.split(/\s+/).filter((s) => s.length > 0)
    result.value = await execRepoCommand(props.repoId, args)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

/**
 * borg reserves exit 1 for warnings, which are not failures - a `check` that
 * finds a repairable inconsistency exits 1 and the operator still wants to
 * read the output.
 */
function exitClass(code: number): string {
  if (code === 0) return 'exit-ok'
  if (code === 1) return 'exit-warn'
  return 'exit-err'
}
</script>

<template>
  <div>
    <p class="pane-lede console-lede">
      Execute borg commands directly against this repository. The repository URL and passphrase are
      injected automatically. Use <code class="console-code">::archive</code> notation to reference
      a specific archive.
    </p>
    <div class="console-input-row">
      <span class="console-prefix">borg</span>
      <input
        v-model="command"
        class="input console-input"
        placeholder="info"
        :disabled="loading"
        aria-label="borg command"
        @keydown.enter="run"
      />
      <button
        class="btn btn-sm btn-primary"
        :disabled="loading || !command.trim()"
        @click="run"
      >
        {{ loading ? 'Running...' : 'Run' }}
      </button>
    </div>
    <div class="console-hints">
      <span class="console-hint-label">Commands:</span>
      <code
        v-for="cmd in SUGGESTED_COMMANDS"
        :key="cmd"
        class="console-hint-cmd"
        @click="command = cmd"
        >{{ cmd }}</code
      >
    </div>
    <div
      v-if="error"
      class="console-error"
    >
      {{ error }}
    </div>
    <div
      v-if="result"
      class="console-output"
    >
      <div class="console-output-header">
        <span class="console-output-label">Output</span>
        <span :class="exitClass(result.exit_code)">exit {{ result.exit_code }}</span>
      </div>
      <pre
        v-if="result.stdout"
        class="console-pre"
        >{{ result.stdout }}</pre
      >
      <pre
        v-if="result.stderr"
        class="console-pre console-pre-stderr"
        >{{ result.stderr }}</pre
      >
      <span
        v-if="!result.stdout && !result.stderr"
        class="console-empty"
        >(no output)</span
      >
    </div>
  </div>
</template>

<style scoped>
/* The lede doubles as this pane's instructions, so it needs the gap a
   `.pane-head` would have put under it. */
.console-lede {
  margin-bottom: var(--space-5);
}

.console-code {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-1) var(--space-2);
}

.console-input-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.console-prefix {
  font-family: var(--mono);
  font-size: var(--fs-base);
  color: var(--text-muted);
  flex-shrink: 0;
}

.console-input {
  flex: 1;
  font-family: var(--mono);
  font-size: var(--fs-base);
}

.console-hints {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-3);
  margin-top: var(--space-4);
}

.console-hint-label {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.console-hint-cmd {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  padding: var(--space-1) var(--space-3);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--accent);
  transition: background var(--duration-fast);
}

.console-hint-cmd:hover {
  background: var(--accent-subtle);
}

.console-error {
  margin-top: var(--space-5);
  padding: var(--space-4) var(--space-5);
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
  color: var(--danger);
}

.console-output {
  margin-top: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.console-output-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-5);
  background: var(--bg-input);
  border-bottom: 1px solid var(--border);
  font-size: var(--fs-xs);
}

.console-output-label {
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  font-size: var(--fs-2xs);
}

.console-pre {
  margin: 0;
  padding: var(--space-5);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-primary);
  background: var(--bg-base);
  max-height: 400px;
  overflow-y: auto;
}

.console-pre-stderr {
  color: var(--warning);
  border-top: 1px solid var(--border);
}

.console-empty {
  display: block;
  padding: var(--space-5);
  font-size: var(--fs-sm);
  color: var(--text-muted);
  font-style: italic;
}

.exit-ok {
  color: var(--success);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.exit-warn {
  color: var(--warning);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.exit-err {
  color: var(--danger);
  font-family: var(--mono);
  font-size: var(--fs-xs);
}
</style>

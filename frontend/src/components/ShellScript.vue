<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { highlightShell, type ShellToken, type ShellTokenKind } from '../utils/shellHighlight'

/**
 * A hook command rendered read-only.
 *
 * A hook command is a whole script, not a line: the editor has always let one
 * span several lines, but every read-only rendering of it was an inline
 * `<code>`, where the browser collapses the newlines and indentation into
 * single spaces and a ten-line script arrives as one unreadable paragraph.
 * `.detail-pre` is the shared block that keeps them, and the spans colour it.
 */
const props = defineProps<{ source: string }>()

/** One class per token kind; the kinds are exhaustive, so no fallback. */
const CLASS_BY_KIND: Record<ShellTokenKind, string> = {
  comment: 'sh-comment',
  string: 'sh-string',
  variable: 'sh-variable',
  keyword: 'sh-keyword',
  command: 'sh-command',
  operator: 'sh-operator',
  number: 'sh-number',
  text: 'sh-text',
}

const tokens = computed<ShellToken[]>(() => highlightShell(props.source))
</script>

<template>
  <div class="detail-pre">
    <span
      v-for="(token, index) in tokens"
      :key="index"
      :class="CLASS_BY_KIND[token.kind]"
      v-text="token.text"
    />
  </div>
</template>

<style scoped>
.sh-comment {
  color: var(--syntax-comment);
  font-style: italic;
}

.sh-string,
.sh-number {
  color: var(--syntax-string);
}

.sh-variable {
  color: var(--syntax-variable);
}

.sh-keyword {
  color: var(--syntax-keyword);
  font-weight: 600;
}

.sh-command {
  color: var(--text-primary);
  font-weight: 600;
}

.sh-operator {
  color: var(--syntax-operator);
}

.sh-text {
  color: var(--text-secondary);
}
</style>

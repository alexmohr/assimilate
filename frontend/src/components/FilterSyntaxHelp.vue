<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * Toolbar button explaining what the search box next to it understands.
 *
 * The field list comes from `utils/filterQuery`, so a field added to the parser
 * cannot ship without a line here explaining it - which is the failure mode a
 * hand-written cheat sheet has.
 */
import { ref } from 'vue'
import { CircleQuestionMark } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import { FILTER_FIELD_HELP } from '../utils/filterQuery'

interface CombinatorHelp {
  example: string
  meaning: string
}

const COMBINATORS: readonly CombinatorHelp[] = [
  { example: 'borg-backup', meaning: 'Bare text matches any of the fields above' },
  { example: 'agent:k3s host:borg-backup', meaning: 'A space means both must match' },
  { example: 'agent:k3s | agent:nas', meaning: 'A pipe means either may match' },
  { example: 'agent:"web server"', meaning: 'Quote a value that contains spaces' },
]

const open = ref(false)
</script>

<template>
  <button
    type="button"
    class="filter-toggle"
    aria-label="Filter syntax"
    title="Filter syntax"
    aria-haspopup="dialog"
    @click="open = true"
  >
    <CircleQuestionMark :size="14" />
  </button>

  <BaseModal
    :open="open"
    title="Filter syntax"
    size="md"
    @close="open = false"
  >
    <p class="syntax-lede">
      Type text to search every field, or scope a term to one field with
      <code>field:value</code>. Matching ignores case and matches on part of a value.
    </p>

    <div class="table-wrap">
      <table class="data-table">
        <thead>
          <tr>
            <th>Field</th>
            <th>Matches</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in FILTER_FIELD_HELP"
            :key="row.field"
          >
            <td class="cell-mono">{{ row.example }}</td>
            <td>{{ row.description }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="table-wrap syntax-table-gap">
      <table class="data-table">
        <thead>
          <tr>
            <th>Combining terms</th>
            <th>Meaning</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in COMBINATORS"
            :key="row.example"
          >
            <td class="cell-mono">{{ row.example }}</td>
            <td>{{ row.meaning }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </BaseModal>
</template>

<style scoped>
.syntax-lede {
  color: var(--text-secondary);
  font-size: var(--fs-sm);
  line-height: 1.5;
  margin-bottom: var(--space-5);
}

.syntax-table-gap {
  margin-top: var(--space-5);
}
</style>

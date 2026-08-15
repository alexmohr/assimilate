<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { Trash2 } from '@lucide/vue'
import { formatDate } from '../utils/format'

interface ApiToken {
  id: number
  user_id: number
  name: string
  created_at: string
  last_used_at: string | null
}

defineProps<{
  tokens: ApiToken[]
}>()

defineEmits<{
  delete: [token: ApiToken]
}>()
</script>

<template>
  <table class="tokens-table">
    <thead>
      <tr>
        <th>Name</th>
        <th>Created</th>
        <th>Last Used</th>
        <th>Actions</th>
      </tr>
    </thead>
    <tbody>
      <tr
        v-for="token in tokens"
        :key="token.id"
      >
        <td class="cell-name">
          {{ token.name }}
        </td>
        <td class="cell-date">
          {{ formatDate(token.created_at) }}
        </td>
        <td class="cell-date">
          {{ formatDate(token.last_used_at, 'Never') }}
        </td>
        <td>
          <button
            class="btn btn-sm btn-ghost btn-danger-text"
            title="Delete"
            @click="$emit('delete', token)"
          >
            <Trash2 :size="14" />
          </button>
        </td>
      </tr>
    </tbody>
  </table>
</template>

<style scoped>
.tokens-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.tokens-table th {
  text-align: left;
  padding: 0.7rem 1rem;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
}

.tokens-table td {
  padding: 0.65rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.tokens-table tr:last-child td {
  border-bottom: none;
}

.tokens-table tr:hover td {
  background: var(--bg-hover);
}

.cell-name {
  font-weight: 600;
  color: var(--text-primary);
}

.cell-date {
  font-size: 0.8rem;
  color: var(--text-muted);
}
</style>

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
  <div class="table-wrap table-wrap--framed">
    <table class="data-table">
      <thead>
        <tr>
          <th>Name</th>
          <th>Created</th>
          <th>Last used</th>
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
  </div>
</template>

<style scoped>
.cell-name {
  font-weight: 600;
  color: var(--text-primary);
}
</style>

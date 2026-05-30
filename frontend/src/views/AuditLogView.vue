<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { ShieldAlert } from '@lucide/vue'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import { apiClient } from '../api/client'
import { formatDateShort } from '../utils/format'
import { extractError } from '../utils/error'
import { useAuthStore } from '../stores/auth'

interface AuditEntry {
  id: number
  user_id: number | null
  username: string
  action: string
  target_type: string | null
  target_id: number | null
  details: Record<string, unknown> | null
  ip_address: string | null
  created_at: string
}

interface AuditResponse {
  items: AuditEntry[]
  total: number
  page: number
  per_page: number
}

const authStore = useAuthStore()
const isAdmin = computed(() => authStore.user?.role === 'admin')

const entries = ref<AuditEntry[]>([])
const total = ref(0)
const loading = ref(false)
const error = ref<string | null>(null)
const expandedRows = ref<AuditEntry[]>([])

const filters = reactive({
  action: '',
  user: '',
  from: '',
  to: '',
})

const page = ref(1)
const perPage = ref(25)

const perPageOptions = [10, 25, 50]

async function fetchAuditLog(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const params: Record<string, string | number> = {
      page: page.value,
      per_page: perPage.value,
    }
    if (filters.action) params.action = filters.action
    if (filters.user) params.user_id = filters.user
    if (filters.from) params.from = filters.from
    if (filters.to) params.to = filters.to

    const res = await apiClient.get<AuditResponse>('/audit-log', { params })
    entries.value = res.data.items
    total.value = res.data.total
  } catch (e: unknown) {
    error.value = extractError(e)
    entries.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

function onPageChange(event: { page: number; rows: number }): void {
  page.value = event.page + 1
  perPage.value = event.rows
  fetchAuditLog()
}

function clearFilters(): void {
  filters.action = ''
  filters.user = ''
  filters.from = ''
  filters.to = ''
  page.value = 1
  fetchAuditLog()
}

function applyFilters(): void {
  page.value = 1
  fetchAuditLog()
}

watch(perPage, () => {
  page.value = 1
  fetchAuditLog()
})

onMounted(fetchAuditLog)
</script>

<template>
  <div
    v-if="!isAdmin"
    class="access-denied"
  >
    <p>You do not have permission to view the audit log.</p>
  </div>

  <div
    v-else
    class="audit-log"
  >
    <div class="page-header">
      <h1 class="page-title">Audit Log</h1>
      <span class="row-count">{{ total }} entries</span>
    </div>

    <section class="filters">
      <div class="filter-row">
        <div class="filter-group">
          <label class="filter-label">Action</label>
          <input
            v-model="filters.action"
            class="filter-input"
            type="text"
            placeholder="e.g. create, update, delete"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">User</label>
          <input
            v-model="filters.user"
            class="filter-input"
            type="text"
            placeholder="Username"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">From</label>
          <input
            v-model="filters.from"
            class="date-input"
            type="date"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">To</label>
          <input
            v-model="filters.to"
            class="date-input"
            type="date"
          />
        </div>
        <div class="filter-actions">
          <button
            class="btn btn-sm btn-primary"
            @click="applyFilters"
          >
            Apply
          </button>
          <button
            class="btn btn-sm btn-ghost"
            @click="clearFilters"
          >
            Clear
          </button>
        </div>
      </div>
    </section>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />

    <div
      v-else-if="error"
      class="state-msg state-error"
    >
      {{ error }}
    </div>

    <EmptyState
      v-else-if="entries.length === 0"
      :icon="ShieldAlert"
      title="No audit entries"
      description="Mutation operations will appear here once they occur."
    />

    <div
      v-else
      class="table-wrap"
    >
      <DataTable
        v-model:expanded-rows="expandedRows"
        :value="entries"
        :rows="perPage"
        :total-records="total"
        :lazy="true"
        :paginator="true"
        :rows-per-page-options="perPageOptions"
        :first="(page - 1) * perPage"
        data-key="id"
        table-class="audit-table"
        @page="onPageChange"
      >
        <Column
          header="Timestamp"
          field="created_at"
          :sortable="true"
        >
          <template #body="{ data }">
            <span class="cell-ts">{{ formatDateShort(data.created_at) }}</span>
          </template>
        </Column>
        <Column
          header="User"
          field="username"
          :sortable="true"
        >
          <template #body="{ data }">
            <span class="cell-user">{{ data.username }}</span>
          </template>
        </Column>
        <Column
          header="Action"
          field="action"
          :sortable="true"
        >
          <template #body="{ data }">
            <span
              class="badge"
              :class="actionBadgeClass(data.action)"
            >
              {{ data.action }}
            </span>
          </template>
        </Column>
        <Column
          header="Target"
          field="target_type"
        >
          <template #body="{ data }">
            <span class="cell-target">{{ data.target_type }} #{{ data.target_id }}</span>
          </template>
        </Column>
        <Column
          header="IP"
          field="ip_address"
        >
          <template #body="{ data }">
            <span class="cell-ip mono">{{ data.ip_address ?? '—' }}</span>
          </template>
        </Column>
        <Column
          header="Details"
          :expander="true"
        />
        <template #expansion="{ data }">
          <div class="detail-expansion">
            <pre
              v-if="data.details"
              class="detail-pre"
              >{{ data.details }}</pre
            >
            <span
              v-else
              class="muted"
              >No additional details.</span
            >
          </div>
        </template>
        <template #empty>
          <div class="state-msg">No audit entries match the current filters.</div>
        </template>
      </DataTable>
    </div>

    <div class="per-page-selector">
      <label class="filter-label">Rows per page</label>
      <select
        v-model="perPage"
        class="select-input"
      >
        <option
          v-for="opt in perPageOptions"
          :key="opt"
          :value="opt"
        >
          {{ opt }}
        </option>
      </select>
    </div>
  </div>
</template>

<script lang="ts">
function actionBadgeClass(action: string): string {
  const a = action.toLowerCase()
  if (a === 'delete' || a === 'remove') return 'badge-danger'
  if (a === 'create' || a === 'add') return 'badge-success'
  if (a === 'update' || a === 'edit') return 'badge-warning'
  return 'badge-neutral'
}
</script>

<style scoped>
.audit-log {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
  color: var(--text-primary);
}

.access-denied {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.page-title {
  font-size: 1.25rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.row-count {
  font-size: 0.85rem;
  color: var(--text-muted);
}

.filters {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
}

.filter-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 1rem;
}

.filter-group {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.filter-label {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
}

.filter-input,
.date-input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.4rem 0.6rem;
  font-size: 0.875rem;
  outline: none;
  transition: border-color 0.15s;
}

.filter-input:focus,
.date-input:focus {
  border-color: var(--accent);
}

.filter-actions {
  display: flex;
  gap: 0.5rem;
  align-self: flex-end;
}

.select-input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.4rem 0.6rem;
  font-size: 0.875rem;
  outline: none;
}

.table-wrap {
  overflow-x: auto;
  border-radius: var(--radius);
  border: 1px solid var(--border);
}

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.cell-ts {
  color: var(--text-muted);
  white-space: nowrap;
  font-size: 0.8rem;
}

.cell-user {
  font-weight: 600;
  color: var(--text-primary);
}

.cell-target {
  color: var(--text-secondary);
  font-size: 0.85rem;
}

.cell-ip {
  color: var(--text-muted);
  font-size: 0.8rem;
}

.mono {
  font-family: var(--mono);
}

.badge {
  display: inline-block;
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: capitalize;
}

.badge-danger {
  background: var(--danger-subtle);
  color: var(--danger);
}

.badge-success {
  background: var(--success-subtle, oklch(0.95 0.05 145));
  color: var(--success);
}

.badge-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-neutral {
  background: color-mix(in srgb, var(--text-muted) 15%, transparent);
  color: var(--text-secondary);
}

.detail-expansion {
  padding: 1rem 1.5rem;
  background: var(--bg-base);
}

.detail-pre {
  margin: 0;
  padding: 0.75rem 1rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-size: 0.8rem;
  font-family: var(--mono);
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-primary);
}

.muted {
  color: var(--text-muted);
  font-size: 0.875rem;
}

.per-page-selector {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  align-self: flex-end;
}
</style>

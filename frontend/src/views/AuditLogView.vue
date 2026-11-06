<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { ChevronLeft, ChevronRight, ShieldAlert } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import LogList from '../components/LogList.vue'
import LogRow from '../components/LogRow.vue'
import { getAuditLog } from '../api/auditLog'
import { formatDate, formatDateShort, formatEventType } from '../utils/format'
import { useAuthStore } from '../stores/auth'
import { useAsyncAction } from '../composables/useAsyncAction'
import type { AuditEntryResponse } from '../types/generated'
import { auditActionTone, badgeClass } from '../utils/badge'
import { hasAuditDetails } from '../utils/auditDetails'

type AuditEntry = AuditEntryResponse

const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)

const entries = ref<AuditEntry[]>([])
const total = ref(0)
const { loading, error, run } = useAsyncAction()
const expandedId = ref<number | null>(null)

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
  await run(async () => {
    const res = await getAuditLog({
      page: page.value,
      per_page: perPage.value,
      action: filters.action,
      user_id: filters.user,
      from: filters.from,
      to: filters.to,
    })
    entries.value = res.items
    total.value = Number(res.total)
  })
}

const pageCount = computed((): number => Math.max(1, Math.ceil(total.value / perPage.value)))

/** "26-50 of 112": the entries this page shows. */
const rangeLabel = computed((): string => {
  const first = (page.value - 1) * perPage.value + 1
  const last = Math.min(page.value * perPage.value, total.value)
  return `${first}-${last} of ${total.value}`
})

function goToPage(next: number): void {
  page.value = Math.min(Math.max(1, next), pageCount.value)
  expandedId.value = null
  fetchAuditLog()
}

function toggle(entry: AuditEntry): void {
  expandedId.value = expandedId.value === entry.id ? null : entry.id
}

/** "delete_user" as "Delete user". */
function actionLabel(action: AuditEntry['action']): string {
  const words = formatEventType(action)
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** "repository #4", or nothing for an action with no single target. */
function targetLabel(entry: AuditEntry): string | null {
  if (entry.target_type === null) return null
  return entry.target_id === null ? entry.target_type : `${entry.target_type} #${entry.target_id}`
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
            class="input input-sm action-filter"
            type="text"
            placeholder="e.g. create, update, delete"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">User</label>
          <input
            v-model="filters.user"
            class="input input-sm"
            type="text"
            placeholder="Username"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">From</label>
          <input
            v-model="filters.from"
            class="input input-sm date-input"
            type="date"
          />
        </div>
        <div class="filter-group">
          <label class="filter-label">To</label>
          <input
            v-model="filters.to"
            class="input input-sm date-input"
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
      class="error-banner"
    >
      {{ error }}
    </div>

    <EmptyState
      v-else-if="entries.length === 0"
      :icon="ShieldAlert"
      title="No audit entries"
      description="Mutation operations will appear here once they occur."
    />

    <template v-else>
      <LogList data-testid="audit-entries">
        <LogRow
          v-for="entry in entries"
          :key="entry.id"
          :tone="auditActionTone(entry.action)"
          :title="entry.username"
          :subtitle="actionLabel(entry.action)"
          :facts="[targetLabel(entry), entry.ip_address]"
          :time="formatDateShort(entry.created_at)"
          :time-title="formatDate(entry.created_at)"
          :expanded="expandedId === entry.id"
          @toggle="toggle(entry)"
        >
          <template #detail>
            <dl class="info-grid">
              <dt>Time</dt>
              <dd>{{ formatDate(entry.created_at) }}</dd>
              <dt>User</dt>
              <dd>{{ entry.username }}</dd>
              <dt>Action</dt>
              <dd>
                <span
                  class="badge"
                  :class="badgeClass(auditActionTone(entry.action))"
                  >{{ actionLabel(entry.action) }}</span
                >
              </dd>
              <dt>Target</dt>
              <dd>{{ targetLabel(entry) ?? '\u2014' }}</dd>
              <dt>IP address</dt>
              <dd class="audit-ip">{{ entry.ip_address ?? '\u2014' }}</dd>
            </dl>
            <pre
              v-if="hasAuditDetails(entry)"
              class="detail-pre"
              >{{ entry.details }}</pre
            >
          </template>
        </LogRow>
      </LogList>

      <div class="audit-pager">
        <span class="row-count">{{ rangeLabel }}</span>
        <div class="audit-pager-controls">
          <label
            class="filter-label"
            for="audit-per-page"
            >Rows per page</label
          >
          <select
            id="audit-per-page"
            v-model="perPage"
            class="input input-sm select-input"
          >
            <option
              v-for="opt in perPageOptions"
              :key="opt"
              :value="opt"
            >
              {{ opt }}
            </option>
          </select>
          <button
            class="btn btn-sm btn-ghost"
            aria-label="Previous page"
            :disabled="page <= 1"
            @click="goToPage(page - 1)"
          >
            <ChevronLeft :size="14" />
          </button>
          <span class="audit-page">Page {{ page }} of {{ pageCount }}</span>
          <button
            class="btn btn-sm btn-ghost"
            aria-label="Next page"
            :disabled="page >= pageCount"
            @click="goToPage(page + 1)"
          >
            <ChevronRight :size="14" />
          </button>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.audit-log {
  display: flex;
  flex-direction: column;
  gap: var(--space-7);
  color: var(--text-primary);
}

.access-denied {
  text-align: center;
  padding: var(--space-10);
  color: var(--text-muted);
}

.page-header {
  justify-content: space-between;
}

.filter-actions {
  display: flex;
  gap: var(--space-4);
  align-self: flex-end;
}

.audit-ip {
  font-family: var(--mono);
}

/* The count on the left, the page controls together at the right end. */
.audit-pager {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

.audit-pager-controls {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin-left: auto;
}

.audit-page {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  white-space: nowrap;
}
</style>

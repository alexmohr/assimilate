<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import BaseDisclosure from './BaseDisclosure.vue'
import type { EnrollmentTokenRow, EnrollmentTokenStatus } from '../types/enrollment'

// MOCK: dummy rows until the enrollment API exists.
const rows: EnrollmentTokenRow[] = [
  {
    id: 1,
    prefix: 'enr_Zk3q',
    created_by: 'admin',
    hostname: null,
    status: 'pending',
    detail: 'Expires in 59 min',
  },
  {
    id: 2,
    prefix: 'enr_p0Lw',
    created_by: 'admin',
    hostname: 'db-02',
    status: 'used',
    detail: 'Used by db-02',
  },
  {
    id: 3,
    prefix: 'enr_8fAa',
    created_by: 'operator1',
    hostname: null,
    status: 'expired',
    detail: 'Expired 2 h ago',
  },
]

const TONE: Record<EnrollmentTokenStatus, string> = {
  pending: 'badge--warning',
  used: 'badge--success',
  expired: 'badge--neutral',
  revoked: 'badge--danger',
}

const LABEL: Record<EnrollmentTokenStatus, string> = {
  pending: 'Pending',
  used: 'Used',
  expired: 'Expired',
  revoked: 'Revoked',
}
</script>

<template>
  <BaseDisclosure
    title="Pending tokens"
    :badge="String(rows.filter((r) => r.status === 'pending').length)"
    badge-tone="warning"
  >
    <ul class="token-rows">
      <li
        v-for="row in rows"
        :key="row.id"
        class="token-row"
      >
        <span class="mono">{{ row.prefix }}…</span>
        <span class="token-row-detail cell-muted">
          {{ row.hostname ?? 'Any host' }} · {{ row.detail }}
        </span>
        <span
          class="badge"
          :class="TONE[row.status]"
          >{{ LABEL[row.status] }}</span
        >
        <button
          v-if="row.status === 'pending'"
          type="button"
          class="btn btn-sm"
        >
          Revoke
        </button>
      </li>
    </ul>
  </BaseDisclosure>
</template>

<style scoped>
.token-rows {
  list-style: none;
  margin: 0;
  padding: 0;
}

.token-row {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: var(--space-4) var(--space-5);
  border-top: 1px solid var(--border-subtle);
}

.token-row-detail {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-sm);
}
</style>

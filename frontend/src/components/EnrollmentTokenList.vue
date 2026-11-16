<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
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
  <section class="panel panel--sectioned">
    <div class="panel-header">
      <h2 class="panel-title">Enrollment tokens</h2>
    </div>
    <div class="table-wrap">
      <table class="data-table">
        <thead>
          <tr>
            <th>Token</th>
            <th>Created by</th>
            <th>Restricted to</th>
            <th>Status</th>
            <th>Details</th>
            <th />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in rows"
            :key="row.id"
          >
            <td class="cell-mono">{{ row.prefix }}…</td>
            <td>{{ row.created_by }}</td>
            <td :class="{ 'cell-muted': !row.hostname }">{{ row.hostname ?? 'Any host' }}</td>
            <td>
              <span
                class="badge"
                :class="TONE[row.status]"
                >{{ LABEL[row.status] }}</span
              >
            </td>
            <td class="cell-muted">{{ row.detail }}</td>
            <td>
              <button
                v-if="row.status === 'pending'"
                type="button"
                class="btn btn-sm"
              >
                Revoke
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>

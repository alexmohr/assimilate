<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { listEnrollmentTokens, revokeEnrollmentToken } from '../api/enrollment'
import { extractError } from '../utils/error'
import BaseDisclosure from './BaseDisclosure.vue'
import type { EnrollmentTokenResponse, EnrollmentTokenStatus } from '../types/generated'

/**
 * The tokens no host has used yet, and the ones that were: what the server
 * knows about each, with a way to revoke one that is still pending.
 */
const props = defineProps<{
  /** Read the list again whenever this changes. */
  version: number
}>()

const tokens = ref<EnrollmentTokenResponse[]>([])
const error = ref<string | null>(null)
const busyId = ref<number | null>(null)

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

const pendingCount = computed(() => tokens.value.filter((t) => t.status === 'pending').length)

const MS_PER_MINUTE = 60_000

function minutesLeft(token: EnrollmentTokenResponse): number {
  return Math.max(1, Math.ceil((Date.parse(token.expires_at) - Date.now()) / MS_PER_MINUTE))
}

function detail(token: EnrollmentTokenResponse): string {
  const where = token.hostname ?? 'Any host'
  const DETAILS: Record<EnrollmentTokenStatus, string> = {
    pending: `Expires in ${minutesLeft(token)} min`,
    used: token.used_by ? `Used by ${token.used_by}` : 'Used',
    expired: 'Not used in time',
    revoked: 'Revoked',
  }
  return `${where} · ${DETAILS[token.status]}`
}

async function load(): Promise<void> {
  try {
    tokens.value = (await listEnrollmentTokens()).tokens
    error.value = null
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load the tokens')
  }
}

async function revoke(token: EnrollmentTokenResponse): Promise<void> {
  busyId.value = token.id
  try {
    await revokeEnrollmentToken(token.id)
    await load()
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to revoke the token')
  } finally {
    busyId.value = null
  }
}

onMounted(load)
watch(() => props.version, load)
</script>

<template>
  <BaseDisclosure
    v-if="tokens.length > 0 || error"
    title="Enrollment tokens"
    :badge="pendingCount > 0 ? `${pendingCount} pending` : null"
    badge-tone="warning"
  >
    <div
      v-if="error"
      class="form-error"
    >
      {{ error }}
    </div>
    <ul class="token-rows">
      <li
        v-for="token in tokens"
        :key="token.id"
        class="token-row"
        data-testid="enrollment-token-row"
      >
        <span class="mono">{{ token.hint }}...</span>
        <span class="token-row-detail cell-muted">{{ detail(token) }}</span>
        <span
          class="badge"
          :class="TONE[token.status]"
          >{{ LABEL[token.status] }}</span
        >
        <button
          v-if="token.status === 'pending'"
          type="button"
          class="btn btn-sm"
          :disabled="busyId === token.id"
          @click="revoke(token)"
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

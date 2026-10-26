<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { listServerQuotas } from '../api/serverQuotas'
import type { ServerQuotaResponse } from '../types/generated'
import { formatBytes } from '../utils/format'
import { logger } from '../utils/logger'
import { quotaCeiling, quotaHealth } from '../utils/quota'

/**
 * The repository overview's tile for the quota its host shares: how full the
 * host is against it, and the way to the host's Storage quota section, where
 * it is set. Admin only, like the host page it links to. Renders nothing
 * until it knows, and nothing at all if it cannot find out.
 */
const props = defineProps<{ sshHost: string; repoHostId: number }>()

const quota = ref<ServerQuotaResponse | null>(null)

onMounted(async () => {
  try {
    quota.value = (await listServerQuotas()).find((q) => q.ssh_host === props.sshHost) ?? null
  } catch (e: unknown) {
    logger.error('Failed to load the host quota', e)
  }
})

const to = computed(() => ({
  path: `/repo-hosts/${props.repoHostId}`,
  query: { section: 'quota' },
}))

const configured = computed(() => quota.value !== null && quota.value.configured)

const percent = computed((): number | null => {
  const q = quota.value
  if (!q?.configured || !q.enabled) return null
  const ceiling = quotaCeiling(q)
  if (ceiling === null || ceiling <= 0) return null
  return Math.round((q.total_deduplicated_size / ceiling) * 100)
})

const barClass = computed((): string => {
  const q = quota.value
  if (!q) return ''
  const health = quotaHealth(q, q.total_deduplicated_size)
  if (health === 'critical') return 'progress-bar--danger'
  if (health === 'warning') return 'progress-bar--warning'
  return ''
})

const value = computed((): string => {
  if (!configured.value) return 'Not set'
  if (percent.value === null) return 'Off'
  return `${percent.value}%`
})
</script>

<template>
  <RouterLink
    v-if="quota"
    :to="to"
    class="tile tile--link"
  >
    <span class="stat-label">Host quota</span>
    <span class="stat-value stat-value--lg">{{ value }}</span>
    <div
      v-if="percent !== null"
      class="progress-track"
    >
      <div
        class="progress-bar"
        :class="barClass"
        :style="{ width: Math.min(100, percent) + '%' }"
      />
    </div>
    <span class="stat-sub">
      <template v-if="configured">
        {{ formatBytes(quota.total_deduplicated_size) }} of
        {{ formatBytes(quotaCeiling(quota)) }} on {{ quota.ssh_host }}
      </template>
      <template v-else>
        {{ formatBytes(quota.total_deduplicated_size) }} used on {{ quota.ssh_host }}
      </template>
    </span>
  </RouterLink>
</template>

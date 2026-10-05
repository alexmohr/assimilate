<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Network } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import { listDependencyHosts, type DependencyHost } from '../api/dependencyHosts'
import { useWebSocket } from '../composables/useWebSocket'
import { badgeClass } from '../utils/badge'
import { protocolLabel, reachabilityBadge } from '../utils/dependencyHost'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'

/**
 * The Agents page's Dependencies tab: every machine a backup needs besides
 * its agent and its repository, one card each. The whole card is the link to
 * the dependency's page - there is nothing to do on the card itself.
 */
defineProps<{ isAdmin: boolean }>()

const emit = defineEmits<{
  /** The empty state's New, for the page that owns the dialog. */
  new: []
  /** How many there are, for the tab's tally. */
  count: [count: number]
}>()

const hosts = ref<DependencyHost[]>([])
const loading = ref(false)
const error = ref<string | null>(null)

/**
 * The spinner and the error only take over while nothing is listed: a
 * background reload that fails leaves the cards it already has.
 */
async function load(): Promise<void> {
  const first = hosts.value.length === 0
  if (first) loading.value = true
  try {
    hosts.value = await listDependencyHosts()
    error.value = null
    emit('count', hosts.value.length)
  } catch (e: unknown) {
    if (hosts.value.length === 0) error.value = extractError(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void load()
})

function lastCheckedText(host: DependencyHost): string {
  return host.last_checked_at ? relativeTime(host.last_checked_at) : 'Never'
}

function waitingText(count: number): string {
  return count === 1 ? '1 run waiting' : `${count} runs waiting`
}
</script>

<template>
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
    v-else-if="hosts.length === 0"
    :icon="Network"
    title="No dependencies yet"
    description="A dependency is a machine a backup needs besides its agent and its repository, such as an SMB or NFS server whose share a pre-backup command mounts. Assimilate checks it before the backup runs, can wake it, and waits for it when it is not always online."
    :action="isAdmin ? 'New' : undefined"
    @action="emit('new')"
  />
  <div
    v-else
    class="card-grid card-grid--compact"
  >
    <RouterLink
      v-for="host in hosts"
      :key="host.id"
      :to="`/dependency-hosts/${host.id}`"
      class="entity-card entity-card--compact"
      :class="{ 'entity-card--notable': host.last_check_reachable === false }"
    >
      <div class="cc-head">
        <span class="card-name">{{ host.name }}</span>
        <div class="cc-head-end">
          <span
            class="badge"
            :class="badgeClass(reachabilityBadge(host.last_check_reachable).tone)"
          >
            <span class="badge-dot" />
            {{ reachabilityBadge(host.last_check_reachable).label }}
          </span>
        </div>
      </div>
      <div class="cc-facts">
        <span>{{ protocolLabel(host.port) }}</span>
        <span class="cc-sep">·</span>
        <span class="mono">{{ host.address }}:{{ host.port }}</span>
        <template v-if="host.power.repo_host">
          <span class="cc-sep">·</span>
          <span>
            same machine as <b>{{ host.power.repo_host.ssh_host }}</b>
          </span>
        </template>
      </div>
      <div class="card-meta">
        <span
          v-if="host.waiting_count > 0"
          class="badge badge--info"
        >
          <span class="badge-dot" />
          {{ waitingText(host.waiting_count) }}
        </span>
        <span class="badge badge--neutral">
          {{ host.intermittent ? 'Not always online' : 'Always online' }}
        </span>
        <span
          v-if="host.power.effective_wake_enabled"
          class="badge badge--neutral"
        >
          Wake
        </span>
      </div>
      <div class="card-stats">
        <div class="stat">
          <span class="stat-value">{{ host.schedule_count }}</span>
          <span class="stat-label">Schedules</span>
        </div>
        <div class="stat">
          <span class="stat-value">{{ host.agent_default_count }}</span>
          <span class="stat-label">Agent defaults</span>
        </div>
        <div class="stat">
          <span
            class="stat-value"
            :class="{ 'stat-value--warning': host.last_check_reachable === false }"
            >{{ lastCheckedText(host) }}</span
          >
          <span class="stat-label">Last checked</span>
        </div>
      </div>
    </RouterLink>
  </div>
</template>

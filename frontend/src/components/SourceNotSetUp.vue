<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import type { RouteLocationRaw } from 'vue-router'
import { domainParams } from '../utils/agent'
import { listSeparator } from '../utils/format'
import type { AgentRow } from '../types/agent'
import type { HostSourceKind } from '../composables/useHostSources'

/**
 * A schedule takes virtual machines or databases, but these of its hosts have
 * none set up: none found by a scan, or none selected. Their archives will
 * hold none. Each host links to the list where it sets them up.
 */
const props = defineProps<{ hosts: readonly AgentRow[]; kind: HostSourceKind }>()

const COPY: Record<HostSourceKind, { noun: string; title: string }> = {
  vms: { noun: 'virtual machines', title: 'No virtual machines will be backed up' },
  databases: { noun: 'databases', title: 'No databases will be dumped' },
}

const copy = computed(() => COPY[props.kind])

function settingsLink(host: AgentRow): RouteLocationRaw {
  return {
    path: `/agents/${encodeURIComponent(host.hostname)}`,
    query: { tab: 'sources', section: props.kind, ...domainParams(host.domain) },
  }
}

function hostName(host: AgentRow): string {
  return host.display_name ?? host.hostname
}
</script>

<template>
  <div
    class="notice-bar dump-mismatch"
    role="status"
    :data-testid="`source-not-set-up-${kind}`"
  >
    <TriangleAlert :size="16" />
    <div class="notice-bar-text">
      <span class="notice-bar-title">
        {{ copy.title }} on {{ hosts.length === 1 ? '1 host' : `${hosts.length} hosts` }}
      </span>
      <span>
        This schedule takes {{ copy.noun }}, but none are set up on
        <template
          v-for="(host, i) in hosts"
          :key="host.id"
        >
          <RouterLink
            :to="settingsLink(host)"
            class="dump-mismatch-host"
            >{{ hostName(host) }}</RouterLink
          >{{ listSeparator(i, hosts.length, '.') }}
        </template>
        Scan or select them in each host's Sources tab, or turn {{ copy.noun }} off for this
        schedule.
      </span>
    </div>
  </div>
</template>

<style scoped>
.dump-mismatch {
  align-items: flex-start;
}

.dump-mismatch > svg {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--warning);
}

.dump-mismatch-host {
  font-weight: 600;
  color: var(--accent);
}
</style>

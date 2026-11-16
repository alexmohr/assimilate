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
import type { SourceKind } from '../composables/useHostSources'

/**
 * A schedule takes each host's files, virtual machines or databases, but these
 * of its hosts have none set up: no backup paths, or no VMs or database sources
 * found by a scan or selected. Their archives will hold none. Each host links
 * to the list where it sets them up.
 */
const props = defineProps<{ hosts: readonly AgentRow[]; kind: SourceKind }>()

const COPY: Record<SourceKind, { title: string; takes: string; missing: string; fix: string }> = {
  files: {
    title: 'No files will be backed up',
    takes: "each host's files",
    missing: 'no backup paths are set on',
    fix: "Add paths in each host's Sources tab, or give this schedule its own files.",
  },
  vms: {
    title: 'No virtual machines will be backed up',
    takes: 'virtual machines',
    missing: 'none are set up on',
    fix: "Scan or select them in each host's Sources tab, or turn virtual machines off for this schedule.",
  },
  databases: {
    title: 'No databases will be dumped',
    takes: 'databases',
    missing: 'none are set up on',
    fix: "Scan or select them in each host's Sources tab, or turn databases off for this schedule.",
  },
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
        This schedule takes {{ copy.takes }}, but {{ copy.missing }}
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
        {{ copy.fix }}
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

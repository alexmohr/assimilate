<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { TriangleAlert } from '@lucide/vue'
import type { RouteLocationRaw } from 'vue-router'
import { domainParams } from '../utils/agent'
import { listSeparator } from '../utils/format'
import type { AgentRow } from '../types/agent'

/**
 * A schedule asks for database dumps, but these of its hosts have dumps
 * turned off on the host itself: their archives will hold no databases. Each
 * host links to its database settings, where dumps are allowed.
 */
defineProps<{ hosts: readonly AgentRow[] }>()

function settingsLink(host: AgentRow): RouteLocationRaw {
  return {
    path: `/agents/${encodeURIComponent(host.hostname)}`,
    query: { tab: 'sources', section: 'databases', ...domainParams(host.domain) },
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
    data-testid="database-dump-mismatch"
  >
    <TriangleAlert :size="16" />
    <div class="notice-bar-text">
      <span class="notice-bar-title">
        {{
          hosts.length === 1
            ? 'No databases will be dumped on 1 host'
            : `No databases will be dumped on ${hosts.length} hosts`
        }}
      </span>
      <span>
        This schedule asks for database dumps, but database dumps are turned off on
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
        Their backups run without them until dumps are allowed in each host's database settings.
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

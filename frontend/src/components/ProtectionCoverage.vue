<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { RouterLink } from 'vue-router'
import type { DashboardOverview } from '../types/dashboard'

defineProps<{ protection: DashboardOverview['protection'] }>()
</script>

<template>
  <section
    id="protection-coverage"
    class="panel"
  >
    <h2 class="panel-title">Protection Coverage</h2>
    <div class="coverage-score">
      <strong>{{ protection.protected_hosts }}/{{ protection.eligible_hosts }}</strong>
      <span>eligible hosts protected</span>
    </div>
    <p
      v-if="protection.unassigned_hosts.length === 0 && protection.never_succeeded_targets === 0"
      class="coverage-ok"
    >
      All eligible hosts protected
    </p>
    <dl class="coverage-facts">
      <div>
        <dt>Unassigned hosts</dt>
        <dd>{{ protection.unassigned_hosts.length }}</dd>
      </div>
      <div>
        <dt>Targets never succeeded</dt>
        <dd>{{ protection.never_succeeded_targets }}</dd>
      </div>
      <div>
        <dt>Hosts covered only by disabled schedules</dt>
        <dd>{{ protection.disabled_only_hosts.length }}</dd>
      </div>
    </dl>
    <div
      v-if="protection.unassigned_hosts.length > 0"
      class="host-links"
    >
      <RouterLink
        v-for="host in protection.unassigned_hosts"
        :key="host.client_id"
        :to="`/clients/${encodeURIComponent(host.hostname)}`"
      >
        {{ host.hostname }}
      </RouterLink>
    </div>
  </section>
</template>

<style scoped>
.coverage-score {
  display: flex;
  align-items: baseline;
  gap: 0.6rem;
  margin-bottom: 1rem;
}

.coverage-score strong {
  font-size: 2rem;
}

.coverage-score span,
.coverage-ok {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.coverage-ok {
  color: var(--success);
}

.coverage-facts {
  display: grid;
  gap: 0.6rem;
  margin: 0;
}

.coverage-facts div {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  padding-top: 0.6rem;
  border-top: 1px solid var(--border);
}

dt,
dd {
  margin: 0;
  font-size: 0.75rem;
}

dd {
  font-weight: 700;
}

.host-links {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
  margin-top: 1rem;
}

.host-links a {
  padding: 0.25rem 0.5rem;
  border-radius: 99px;
  background: var(--bg-base);
  color: var(--accent);
  font-size: 0.7rem;
  text-decoration: none;
}
</style>

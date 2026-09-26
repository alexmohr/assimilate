<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { getRepoAvailability } from '../api/availability'
import type { HostAvailabilityResponse } from '../types/generated'
import type { RepoWithStats } from '../types/repo'
import { humanizeMinutes } from '../utils/duration'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'

/**
 * A repository's Power pane: what its host does, read-only.
 *
 * Waking the machine, shutting it down and whether it is expected to be online
 * are facts about the host, set once on it for every repository it holds - so
 * this only shows them, and links to where they are changed. What is the
 * repository's own is the list of schedules waiting on it.
 */
const props = defineProps<{
  repo: RepoWithStats
  isAdmin: boolean
}>()

const availability = ref<HostAvailabilityResponse | null>(null)
const loadError = ref<string | null>(null)

async function load(): Promise<void> {
  const repoId = props.repo.id
  try {
    const loaded = await getRepoAvailability(repoId)
    if (props.repo.id !== repoId) return
    availability.value = loaded
    loadError.value = null
  } catch (e: unknown) {
    if (props.repo.id !== repoId) return
    loadError.value = extractError(e)
  }
}

onMounted(load)
watch(
  () => props.repo.id,
  () => {
    availability.value = null
    void load()
  },
)

const power = computed(() => props.repo.power)
const hostLink = computed(() => `/repo-hosts/${props.repo.repo_host.id}?section=power`)

const giveUpText = computed(() => {
  const minutes = availability.value?.catch_up_give_up_minutes ?? 0
  return minutes === 0 ? 'Never' : humanizeMinutes(minutes)
})

/** See `AgentPowerCard`: a missing address while it is in use is redacted. */
const macText = computed(() => power.value.wake_mac_address ?? 'Hidden')
</script>

<template>
  <div>
    <div class="pane-section-head">
      <p class="group-label">Wake-on-LAN</p>
      <RouterLink
        v-if="isAdmin"
        class="btn btn-sm btn-ghost"
        :to="hostLink"
      >
        Edit on host
      </RouterLink>
    </div>
    <dl class="info-grid">
      <dt>Wake host before backup</dt>
      <dd>{{ power.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
      <template v-if="power.wake_enabled || power.shutdown_after_backup">
        <dt>MAC address</dt>
        <dd class="mono">{{ macText }}</dd>
        <dt>Wait for host</dt>
        <dd>{{ power.wake_timeout_seconds }} seconds</dd>
      </template>
      <dt>Shut down host after backup</dt>
      <dd>{{ power.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
    </dl>
    <p class="field-hint">
      Set on the host <span class="mono">{{ repo.ssh_host }}</span> for every repository on it.
    </p>
  </div>

  <section class="pane-section">
    <p class="group-label">When the host is offline</p>
    <div
      v-if="loadError"
      class="state-msg state-msg--inline state-error"
    >
      {{ loadError }}
    </div>
    <template v-else-if="availability">
      <dl class="info-grid">
        <dt>Host is not always online</dt>
        <dd>{{ availability.intermittent ? 'Yes' : 'No' }}</dd>
        <template v-if="availability.intermittent">
          <template v-if="availability.catch_up_recheck_minutes !== null">
            <dt>Re-check every</dt>
            <dd>{{ humanizeMinutes(availability.catch_up_recheck_minutes) }}</dd>
          </template>
          <dt>Stop waiting after</dt>
          <dd>{{ giveUpText }}</dd>
        </template>
      </dl>
      <div
        v-if="availability.waiting.length > 0"
        class="rows"
      >
        <div
          v-for="wait in availability.waiting"
          :key="wait.schedule_id"
          class="agent-row"
        >
          <i
            class="agent-row-stripe agent-row-stripe--warning"
            aria-hidden="true"
          />
          <RouterLink
            class="agent-row-name"
            :to="`/schedules/${wait.schedule_id}`"
          >
            {{ wait.schedule_name }}
          </RouterLink>
          <span class="agent-row-stats">
            missed {{ relativeTime(wait.pending_for) }}
            <template v-if="wait.next_probe_at">
              · next check {{ relativeTime(wait.next_probe_at) }}
            </template>
          </span>
        </div>
      </div>
    </template>
  </section>
</template>

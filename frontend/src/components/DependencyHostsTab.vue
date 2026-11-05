<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { ChevronRight, Network } from '@lucide/vue'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import { listDependencyHosts, type DependencyHost } from '../api/dependencyHosts'
import { getMachine } from '../api/machines'
import { useWebSocket } from '../composables/useWebSocket'
import { getConfiguredTimezone } from '../composables/useTimezone'
import type { MachineWaitResponse } from '../types/generated'
import { badgeClass } from '../utils/badge'
import {
  countLabel,
  dependencyWaits,
  matchesDependencyFilter,
  matchesDependencySearch,
  protocolLabel,
  reachabilityBadge,
  sortDependencies,
  waitingRunsText,
  type DependencyFilter,
} from '../utils/dependencyHost'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'

/**
 * The Dependencies page's list: every machine a backup needs besides its
 * agent and its repository, one row each in a single panel, the ones that
 * need looking at first. A row opens the dependency's page; its Used by
 * counts open that page's Used by section. Checking and waking live on the
 * dependency's own page, not here.
 */
defineProps<{ isAdmin: boolean }>()

const emit = defineEmits<{
  /** The empty state's New, for the page that owns the dialog. */
  new: []
}>()

const router = useRouter()

const hosts = ref<DependencyHost[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
/** The runs waiting on each dependency, by its id; missing until fetched. */
const waits = ref<Record<number, MachineWaitResponse[]>>({})

const search = ref('')
const filter = ref<DependencyFilter>('all')

/**
 * The runs a dependency's row names under it come from its machine, which
 * lists every wait on it. Only dependencies something is waiting on are
 * asked, once per machine; a failure leaves the row with its count alone.
 */
async function loadWaits(list: DependencyHost[]): Promise<void> {
  const waiting = list.filter((h) => h.waiting_count > 0)
  const machineIds = [...new Set(waiting.map((h) => h.machine_id))]
  const results = await Promise.allSettled(machineIds.map((id) => getMachine(id)))
  const byMachine = new Map<number, MachineWaitResponse[]>()
  results.forEach((result, i) => {
    if (result.status === 'fulfilled') byMachine.set(machineIds[i], result.value.waiting)
  })
  const next: Record<number, MachineWaitResponse[]> = {}
  for (const host of waiting) {
    const machineWaits = byMachine.get(host.machine_id)
    if (machineWaits) next[host.id] = dependencyWaits(machineWaits, host.name)
  }
  waits.value = next
}

/**
 * The spinner and the error only take over while nothing is listed: a
 * background reload that fails leaves the rows it already has.
 */
async function load(): Promise<void> {
  const first = hosts.value.length === 0
  if (first) loading.value = true
  try {
    hosts.value = await listDependencyHosts()
    error.value = null
  } catch (e: unknown) {
    if (hosts.value.length === 0) error.value = extractError(e)
    return
  } finally {
    loading.value = false
  }
  await loadWaits(hosts.value)
}

onMounted(load)

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void load()
})

interface FilterPill {
  value: DependencyFilter
  label: string
  dot: string | null
  count: number
}

/** "All" always; the others only when something matches them. */
const pills = computed((): FilterPill[] => {
  const all: FilterPill[] = [
    { value: 'all', label: 'All', dot: null, count: 0 },
    { value: 'not_answering', label: 'Not answering', dot: 'status-dot--warning', count: 0 },
    { value: 'waiting', label: 'Runs waiting', dot: 'status-dot--info', count: 0 },
    { value: 'not_checked', label: 'Not checked', dot: '', count: 0 },
  ]
  for (const pill of all) {
    pill.count = hosts.value.filter((h) => matchesDependencyFilter(h, pill.value)).length
  }
  return all.filter((pill) => pill.value === 'all' || pill.count > 0 || pill.value === filter.value)
})

const visible = computed((): DependencyHost[] =>
  sortDependencies(
    hosts.value.filter(
      (h) => matchesDependencyFilter(h, filter.value) && matchesDependencySearch(h, search.value),
    ),
  ),
)

/**
 * Summed over the dependencies, so a schedule that needs two of them counts
 * twice: the list carries counts, not the schedules behind them.
 */
const scheduleTotal = computed((): number =>
  hosts.value.reduce((sum, h) => sum + h.schedule_count, 0),
)

function statusDot(host: DependencyHost): string {
  if (host.last_check_reachable === false) return 'status-dot--warning'
  if (host.last_check_reachable === true) return 'status-dot--success'
  return ''
}

function lastCheckedText(host: DependencyHost): string {
  return host.last_checked_at ? relativeTime(host.last_checked_at) : 'Never'
}

function detailPath(host: DependencyHost): string {
  return `/dependency-hosts/${host.id}`
}

function usedByPath(host: DependencyHost): string {
  return `${detailPath(host)}?section=used-by`
}

/** The power section lists every run waiting on the dependency. */
function waitsPath(host: DependencyHost): string {
  return `${detailPath(host)}?section=power`
}

/** "14:05" today, "Nov 3, 14:05" on an earlier day. */
function clockTime(iso: string): string {
  const date = new Date(iso)
  const timeZone = getConfiguredTimezone()
  const day = (d: Date): string => d.toLocaleDateString(undefined, { timeZone })
  return date.toLocaleString(
    undefined,
    day(date) === day(new Date())
      ? { hour: '2-digit', minute: '2-digit', hour12: false, timeZone }
      : {
          month: 'short',
          day: 'numeric',
          hour: '2-digit',
          minute: '2-digit',
          hour12: false,
          timeZone,
        },
  )
}

function waitText(wait: MachineWaitResponse): string {
  const name = wait.schedule_name || `Schedule #${wait.schedule_id}`
  const parts = [wait.hostname ? `${name} on ${wait.hostname}` : name]
  parts.push(`waiting since ${clockTime(wait.pending_for)}`)
  if (wait.give_up_at) parts.push(`gives up ${clockTime(wait.give_up_at)}`)
  return parts.join(' · ')
}

function open(host: DependencyHost): void {
  void router.push(detailPath(host))
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
    class="dependencies-list"
  >
    <div class="dependencies-toolbar">
      <div
        class="filter-pills"
        role="group"
        aria-label="Filter dependencies"
      >
        <button
          v-for="pill in pills"
          :key="pill.value"
          type="button"
          class="filter-pill"
          :class="{ active: filter === pill.value }"
          :aria-pressed="filter === pill.value"
          @click="filter = pill.value"
        >
          <span
            v-if="pill.dot !== null"
            class="status-dot"
            :class="pill.dot"
          />
          {{ pill.label }} · {{ pill.count }}
        </button>
      </div>
      <input
        v-model="search"
        type="search"
        class="input search-input"
        placeholder="Search by name or address"
        aria-label="Search dependencies"
      />
    </div>

    <section class="list-group">
      <div class="list-group-header">
        <h2 class="list-group-title">All dependencies</h2>
        <span class="list-group-count">
          {{ countLabel(hosts.length, 'host', 'hosts') }} ·
          {{ countLabel(scheduleTotal, 'schedule depends', 'schedules depend') }} on them
        </span>
      </div>
      <div
        class="list-cols"
        aria-hidden="true"
      >
        <span>Dependency</span>
        <span>Availability</span>
        <span>Used by</span>
        <span>Last checked</span>
        <span />
      </div>

      <div
        v-if="visible.length === 0"
        class="list-row dependencies-none"
      >
        <span class="muted">No dependency matches.</span>
      </div>

      <template
        v-for="host in visible"
        :key="host.id"
      >
        <div
          class="list-row list-row--link"
          data-testid="dependency-row"
          :data-dependency-id="host.id"
          @click="open(host)"
        >
          <div class="list-row-main">
            <span
              class="status-dot"
              :class="statusDot(host)"
            />
            <div class="list-row-text">
              <div class="dependency-name-line">
                <RouterLink
                  :to="detailPath(host)"
                  class="list-row-name dependency-name"
                  @click.stop
                >
                  {{ host.name }}
                </RouterLink>
                <span
                  class="badge"
                  :class="badgeClass(reachabilityBadge(host.last_check_reachable).tone)"
                >
                  {{ reachabilityBadge(host.last_check_reachable).label }}
                </span>
              </div>
              <span
                class="list-row-sub"
                data-testid="dependency-address"
              >
                {{ protocolLabel(host.port) }} ·
                <span class="mono">{{ host.address }}:{{ host.port }}</span>
              </span>
              <span
                v-if="host.repo_host"
                class="list-row-sub"
              >
                Same machine as {{ host.repo_host.ssh_host }}
              </span>
            </div>
          </div>

          <div
            class="list-row-chips"
            data-testid="dependency-availability"
          >
            <span class="badge badge--neutral">
              {{ host.intermittent ? 'Not always online' : 'Always online' }}
            </span>
            <span
              v-if="host.wake_enabled"
              class="badge badge--neutral"
            >
              {{ host.intermittent ? 'Wake on demand' : 'Wake' }}
            </span>
          </div>

          <div class="dependency-used-by">
            <span
              v-if="host.schedule_count === 0 && host.agent_default_count === 0"
              class="muted"
            >
              Nothing yet
            </span>
            <template v-else>
              <RouterLink
                v-if="host.schedule_count > 0"
                :to="usedByPath(host)"
                class="dependency-used-link"
                @click.stop
              >
                {{ countLabel(host.schedule_count, 'schedule', 'schedules') }}
              </RouterLink>
              <RouterLink
                v-if="host.agent_default_count > 0"
                :to="usedByPath(host)"
                class="dependency-used-link"
                @click.stop
              >
                {{ countLabel(host.agent_default_count, 'agent default', 'agent defaults') }}
              </RouterLink>
            </template>
          </div>

          <span
            class="list-row-value dependency-checked"
            :class="{
              'dependency-checked--warning': host.last_check_reachable === false,
              muted: host.last_checked_at === null,
            }"
          >
            <span class="list-narrow-only">Checked </span>{{ lastCheckedText(host) }}
          </span>

          <ChevronRight
            :size="16"
            class="list-row-chevron"
          />
        </div>

        <div
          v-if="host.waiting_count > 0"
          class="dependency-waiting"
        >
          <span class="status-dot status-dot--info" />
          <div class="dependency-waiting-text">
            <span class="dependency-waiting-count">{{ waitingRunsText(host.waiting_count) }}</span>
            <span
              v-for="wait in waits[host.id] ?? []"
              :key="`${wait.schedule_id}:${wait.hostname ?? ''}:${wait.pending_for}`"
              class="list-row-sub"
            >
              {{ waitText(wait) }}
            </span>
          </div>
          <RouterLink
            :to="waitsPath(host)"
            class="dependency-waiting-link"
          >
            {{ host.waiting_count === 1 ? 'View run' : 'View runs' }}
          </RouterLink>
        </div>
      </template>
    </section>
  </div>
</template>

<style scoped>
.dependencies-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.dependencies-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-5);
}

.dependencies-list .list-group {
  --list-cols: minmax(0, 2.6fr) minmax(0, 1.6fr) minmax(0, 1.4fr) 7rem 1.25rem;
}

.dependencies-none {
  display: block;
}

.dependency-name-line {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-width: 0;
}

.dependency-name {
  color: var(--text-primary);
  text-decoration: none;
  min-width: 0;
}

.dependency-used-by {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}

.dependency-used-link,
.dependency-waiting-link {
  color: var(--accent);
  text-decoration: none;
  white-space: nowrap;
}

.dependency-used-link:hover,
.dependency-waiting-link:hover {
  text-decoration: underline;
}

.dependency-checked--warning {
  color: var(--warning);
}

/* The runs waiting on the row above, indented to start under its name. */
.dependency-waiting {
  display: flex;
  align-items: center;
  gap: var(--space-5);
  padding: var(--space-4) var(--space-7);
  padding-left: calc(var(--space-7) + 8px + var(--space-5));
  border-top: 1px solid var(--border);
  background: var(--info-subtle);
  font-size: var(--fs-sm);
}

.dependency-waiting-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 1;
  min-width: 0;
}

.dependency-waiting-count {
  font-weight: 600;
}

@media (max-width: 640px) {
  .search-input {
    width: 100%;
  }

  .dependency-used-by {
    flex-direction: row;
    gap: var(--space-4);
  }

  .dependency-waiting {
    flex-wrap: wrap;
    padding: var(--space-4) var(--space-5);
  }
}
</style>

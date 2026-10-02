<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RefreshCw } from '@lucide/vue'
import {
  deleteDependencyHost,
  getDependencyHost,
  listDependencyHostUsage,
  testDependencyHost,
  type DependencyHost,
} from '../api/dependencyHosts'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import { useAuthStore } from '../stores/auth'
import type { DependencyUsageResponse } from '../types/generated'
import { badgeClass } from '../utils/badge'
import { protocolLabel, reachabilityBadge, testResultText } from '../utils/dependencyHost'
import {
  isDependencyHostSection,
  type DependencyHostSection,
} from '../utils/dependencyHostSettings'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import BaseSpinner from '../components/BaseSpinner.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import DependencyAvailabilityCard from '../components/DependencyAvailabilityCard.vue'
import DependencyConnectionCard from '../components/DependencyConnectionCard.vue'
import DependencyPowerCard from '../components/DependencyPowerCard.vue'
import DetailHeader from '../components/DetailHeader.vue'
import SettingsRail, { type SettingsSections } from '../components/SettingsRail.vue'

/**
 * One dependency: a machine a backup needs besides its agent and its
 * repository - where it is checked, how it is woken, whether it is expected
 * to be online, and which backups need it.
 *
 * Open to every signed-in user, read-only: what a backup waits on is worth
 * knowing to anyone who reads its runs. Changing any of it is for admins.
 */
const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)

const hostId = computed(() => Number(props.id))
const host = ref<DependencyHost | null>(null)
const usage = ref<DependencyUsageResponse[]>([])
const { loading, error, run } = useAsyncAction()
const { success: toastSuccess, warning: toastWarning, error: toastError } = useToast()

const section = computed<DependencyHostSection>({
  get() {
    const s = route.query.section
    return isDependencyHostSection(s) ? s : 'connection'
  },
  set(val: DependencyHostSection) {
    router.replace({ query: { ...route.query, section: val } })
  },
})

const sections = computed<SettingsSections<DependencyHostSection>>(() => {
  const base: SettingsSections<DependencyHostSection> = [
    { id: 'connection', label: 'Connection' },
    { id: 'power', label: 'Power' },
    { id: 'used-by', label: 'Used by' },
  ]
  return isAdmin.value ? [...base, { id: 'danger', label: 'Danger zone', danger: true }] : base
})

const subtitle = computed(() =>
  host.value
    ? `Dependency · ${protocolLabel(host.value.port)} ${host.value.address}:${host.value.port}`
    : null,
)

const reachability = computed(() => reachabilityBadge(host.value?.last_check_reachable ?? null))

async function fetchAll(): Promise<void> {
  const id = hostId.value
  const [loaded, used] = await Promise.all([getDependencyHost(id), listDependencyHostUsage(id)])
  if (hostId.value !== id) return
  host.value = loaded
  usage.value = used
}

async function load(): Promise<void> {
  await run(fetchAll)
}

async function refresh(): Promise<void> {
  try {
    await fetchAll()
  } catch (e: unknown) {
    logger.error('background dependency refresh failed', e)
  }
}

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void refresh()
})

watch(
  () => props.id,
  async () => {
    host.value = null
    usage.value = []
    await load()
  },
)
onMounted(load)

const testing = ref(false)

async function testConnection(): Promise<void> {
  testing.value = true
  try {
    const result = await testDependencyHost(hostId.value)
    if (result.reachable) toastSuccess(testResultText(result))
    else toastWarning(testResultText(result))
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    testing.value = false
    await refresh()
  }
}

function usageSourceText(entry: DependencyUsageResponse): string {
  return entry.source === 'agent_default' ? 'from agent defaults' : 'set on the schedule'
}

const showDelete = ref(false)
const deleting = ref(false)
const deleteError = ref<string | null>(null)

async function confirmDelete(): Promise<void> {
  deleting.value = true
  deleteError.value = null
  try {
    await deleteDependencyHost(hostId.value)
    showDelete.value = false
    await router.push({ path: '/agents', query: { tab: 'dependencies' } })
  } catch (e: unknown) {
    deleteError.value = extractError(e)
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div class="dependency-host-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/agents"
        class="crumb-link"
      >
        Agents
      </RouterLink>
      <span class="crumb-sep">/</span>
      <RouterLink
        :to="{ path: '/agents', query: { tab: 'dependencies' } }"
        class="crumb-link"
      >
        Dependencies
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current mono">{{ host?.name ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !host"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="host">
      <DetailHeader
        :name="host.name"
        mono
        :subtitle="subtitle"
      >
        <template #badges>
          <span
            class="badge"
            :class="badgeClass(reachability.tone)"
          >
            <span class="badge-dot" />
            {{ reachability.label }}
          </span>
          <span
            v-if="host.intermittent"
            class="badge badge--neutral"
          >
            Not always online
          </span>
        </template>
        <template
          v-if="isAdmin"
          #actions
        >
          <button
            type="button"
            class="btn btn-sm"
            :disabled="testing"
            @click="testConnection"
          >
            <RefreshCw
              :size="14"
              :class="{ spinning: testing }"
            />
            {{ testing ? 'Testing...' : 'Test connection' }}
          </button>
        </template>
      </DetailHeader>

      <div class="tab-content fade-in">
        <SettingsRail
          v-slot="{ section: current }"
          :sections="sections"
          :section="section"
          label="Dependency sections"
          @update:section="section = $event"
        >
          <DependencyConnectionCard
            v-if="current === 'connection'"
            :host="host"
            :can-edit="isAdmin"
            @saved="host = $event"
          />

          <template v-else-if="current === 'power'">
            <DependencyPowerCard
              :host="host"
              :is-admin="isAdmin"
              @saved="host = $event"
            />
            <DependencyAvailabilityCard
              :host-id="host.id"
              :port="host.port"
              :can-edit="isAdmin"
              @saved="refresh"
            />
          </template>

          <template v-else-if="current === 'used-by'">
            <p
              v-if="usage.length === 0"
              class="field-hint"
            >
              No schedule needs this dependency yet. Add it under a schedule's Settings, in
              Dependencies, or an agent's Backup defaults.
            </p>
            <div
              v-else
              class="rows"
            >
              <RouterLink
                v-for="entry in usage"
                :key="`${entry.schedule_id}:${entry.agent_id}:${entry.source}`"
                class="agent-row"
                :to="`/schedules/${entry.schedule_id}`"
              >
                <i
                  class="agent-row-stripe"
                  aria-hidden="true"
                />
                <span class="agent-row-name">{{ entry.schedule_name }}</span>
                <span class="meta-pill">{{ entry.hostname }}</span>
                <span class="agent-row-stats">{{ usageSourceText(entry) }}</span>
              </RouterLink>
            </div>
          </template>

          <template v-else-if="current === 'danger'">
            <div class="danger-body">
              <div class="danger-info">
                <span class="danger-heading">Remove dependency</span>
                <span class="danger-desc">
                  Forget this dependency and its settings. Schedules and agent defaults stop needing
                  it, and runs waiting on it stop waiting.
                </span>
              </div>
              <button
                class="btn btn-sm btn-danger"
                type="button"
                @click="showDelete = true"
              >
                Remove dependency
              </button>
            </div>
          </template>
        </SettingsRail>
      </div>

      <ConfirmDeleteDialog
        :show="showDelete"
        title="Remove dependency"
        :submitting="deleting"
        :error="deleteError"
        confirm-label="Remove"
        submitting-label="Removing..."
        @cancel="showDelete = false"
        @confirm="confirmDelete"
      >
        Remove <span class="mono">{{ host.name }}</span
        >? Schedules and agent defaults stop needing it, and runs waiting on it stop waiting.
      </ConfirmDeleteDialog>
    </template>
  </div>
</template>

<style scoped>
.dependency-host-detail {
  max-width: 1200px;
}
</style>

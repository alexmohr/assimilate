<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useWebSocket } from '../composables/useWebSocket'
import { usePushSubscription } from '../composables/usePushSubscription'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import {
  listChannels,
  listRules,
  getVapidPublicKey,
  subscribePush,
  listDeliveries,
} from '../api/notifications'
import { Plus, Bell } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import NotificationHistoryTab from '../components/NotificationHistoryTab.vue'
import NotificationChannelList from '../components/NotificationChannelList.vue'
import {
  channelEventsLabel,
  channelScopeLabel,
  channelTypeLabel,
  eventTypeLabel,
} from '../utils/notificationEvents'
import type {
  NotificationChannelResponse,
  NotificationDeliveryResponse,
  NotificationRuleResponse,
} from '../types/generated'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

/**
 * Every notification channel as one panel of rows, and every channel's
 * delivery history under the History tab. A row opens the channel's own page,
 * where its settings, events, scope and content are edited, a test is sent,
 * and its own history is shown. A new channel is made on its own wizard page.
 */
const TAB_IDS = ['channels', 'history'] as const
type TabId = (typeof TAB_IDS)[number]

function isTabId(value: unknown): value is TabId {
  return TAB_IDS.some((t) => t === value)
}

const route = useRoute()
const router = useRouter()

/** `?tab=history` opens the History tab, so a link can land on it. */
const activeTab = computed<TabId>({
  get() {
    const t = route.query.tab
    return isTabId(t) ? t : 'channels'
  },
  set(val: TabId) {
    void router.replace({ query: { ...route.query, tab: val } })
  },
})
/** The channel the History tab is narrowed to, or null for every channel. */
const historyChannelId = ref<number | null>(null)
const channels = ref<NotificationChannelResponse[]>([])

const tabs = computed((): TabOption<TabId>[] => [
  { id: 'channels', label: 'Channels', count: channels.value.length },
  { id: 'history', label: 'History' },
])

const rules = ref<NotificationRuleResponse[]>([])
const deliveries = ref<NotificationDeliveryResponse[]>([])
const { loading, error, run } = useAsyncAction()
const { isPushSupported, subscription } = usePushSubscription()

function eventsLabel(channelId: number): string {
  return channelEventsLabel(rules.value, channelId)
}

async function loadChannels(): Promise<void> {
  await run(async () => {
    channels.value = await listChannels()
    rules.value = await listRules()
  })
}

async function loadDeliveries(): Promise<void> {
  try {
    deliveries.value = await listDeliveries(20)
  } catch (e: unknown) {
    logger.error('loadDeliveries failed', e)
  }
}

async function loadPushStatus(): Promise<void> {
  try {
    const vapidStatus = await getVapidPublicKey()
    if (isPushSupported() && vapidStatus.configured) {
      const registration = await navigator.serviceWorker.ready
      const existing = await registration.pushManager.getSubscription()
      subscription.value = existing
      if (existing) {
        await subscribePush(existing.toJSON())
      }
    }
  } catch (e: unknown) {
    logger.error('loadPushStatus failed', e)
  }
}

/** A new channel is made on its own wizard page, like a new schedule. */
function openAddChannel(): void {
  router.push('/notifications/new')
}

const { onMessage } = useWebSocket()
onMessage('NotificationDelivery', (data: NotificationDeliveryResponse) => {
  deliveries.value.unshift(data)
  if (deliveries.value.length > 20) {
    deliveries.value.pop()
  }
})

onMounted(() => {
  loadChannels().catch(logger.error)
  loadDeliveries().catch(logger.error)
  loadPushStatus().catch(logger.error)
})
</script>

<template>
  <div class="notifications-view">
    <div class="page-header">
      <h1 class="page-title">Notifications</h1>
      <div class="header-actions">
        <button
          v-if="activeTab === 'channels'"
          class="btn btn-primary"
          @click="openAddChannel"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <BaseTabs
      v-model="activeTab"
      :tabs="tabs"
      label="Notification sections"
    />

    <!-- Channels Tab -->
    <div
      v-if="activeTab === 'channels'"
      class="tab-content fade-in"
    >
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
        v-else-if="channels.length === 0"
        :icon="Bell"
        title="No notification channels"
        description="Create a channel to receive alerts."
        action="New channel"
        @action="openAddChannel"
      />
      <NotificationChannelList
        v-else
        :channels="channels"
        :deliveries="deliveries"
        :events-label="eventsLabel"
        :scope-label="channelScopeLabel"
        :channel-type-label="channelTypeLabel"
      />
    </div>

    <!-- History Tab -->
    <div
      v-if="activeTab === 'history'"
      class="tab-content fade-in"
    >
      <NotificationHistoryTab
        v-model:channel-id="historyChannelId"
        :deliveries="deliveries"
        :channels="channels"
        :event-type-label="eventTypeLabel"
      />
    </div>
  </div>
</template>

<style scoped>
.notifications-view {
  max-width: 1100px;
}
</style>

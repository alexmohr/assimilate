<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { Send } from '@lucide/vue'
import DeliveryLogRow from './DeliveryLogRow.vue'
import EmptyState from './EmptyState.vue'
import LogList from './LogList.vue'
import type {
  EventType,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
} from '../types/generated'

/**
 * The delivery log: one row per attempt, expanding to show the payload and
 * any error.
 */
const props = defineProps<{
  deliveries: NotificationDeliveryResponse[]
  /** Used to resolve a delivery's channel id back to its name. */
  channels: NotificationChannelResponse[]
  /** Shared with the channel list, so both spell an event type the same way. */
  eventTypeLabel: (event: EventType) => string
  /** Narrows the log to one channel; null (or absent) shows every channel. */
  channelId?: number | null
}>()

const emit = defineEmits<{ 'update:channelId': [id: number | null] }>()

const shown = computed((): NotificationDeliveryResponse[] => {
  const id = props.channelId ?? null
  return id === null ? props.deliveries : props.deliveries.filter((d) => d.channel_id === id)
})

/** The picker's option values are strings; "" is every channel. */
const filterValue = computed({
  get: (): string => (props.channelId == null ? '' : String(props.channelId)),
  set: (value: string): void => {
    emit('update:channelId', value === '' ? null : Number(value))
  },
})

const expandedId = ref<number | null>(null)

function toggleExpand(id: number): void {
  expandedId.value = expandedId.value === id ? null : id
}

function channelNameById(id: number): string {
  return props.channels.find((c) => c.id === id)?.name ?? String(id)
}
</script>

<template>
  <EmptyState
    v-if="deliveries.length === 0"
    :icon="Send"
    title="No deliveries yet"
    description="Notifications sent to your channels will be listed here."
  />
  <div
    v-if="deliveries.length > 0 && (channels.length > 1 || channelId != null)"
    class="toolbar"
  >
    <select
      v-model="filterValue"
      class="input select-input select-input--md"
      aria-label="Filter by channel"
    >
      <option value="">All channels</option>
      <option
        v-for="c in channels"
        :key="c.id"
        :value="String(c.id)"
      >
        {{ c.name }}
      </option>
    </select>
  </div>
  <EmptyState
    v-if="deliveries.length > 0 && shown.length === 0"
    :icon="Send"
    title="Nothing sent through this channel yet"
    description="Its deliveries will be listed here once it sends one."
    action="Show all channels"
    @action="emit('update:channelId', null)"
  />
  <LogList
    v-else-if="shown.length > 0"
    data-testid="delivery-history"
  >
    <DeliveryLogRow
      v-for="d in shown"
      :key="d.id"
      :delivery="d"
      :channel-name="channelNameById(d.channel_id)"
      :event-label="eventTypeLabel(d.event_type)"
      :expanded="expandedId === d.id"
      @toggle="toggleExpand(d.id)"
    />
  </LogList>
</template>

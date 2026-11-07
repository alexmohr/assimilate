<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import LogRow from './LogRow.vue'
import { formatDate, formatDateShort } from '../utils/format'
import { deliveryTone, shortError } from '../utils/notificationChannels'
import type {
  DeliveryStatus,
  NotificationDeliveryResponse,
  NotificationEvent,
} from '../types/generated'

/**
 * One notification delivery attempt as a log row: the event and the channel
 * it went to, opening onto the error and the payload that was sent. Shared
 * by the History tab and the channel list's recent deliveries.
 */
defineProps<{
  delivery: NotificationDeliveryResponse
  channelName: string
  eventLabel: string
  expanded: boolean
}>()

const emit = defineEmits<{ toggle: [] }>()

function statusLabel(status: DeliveryStatus): string {
  switch (status) {
    case 'sent':
      return 'Sent'
    case 'failed':
      return 'Failed'
    case 'pending':
      return 'Pending'
  }
}

/** The line under the title: the error of a failed attempt, else its outcome. */
function outcome(d: NotificationDeliveryResponse): string {
  if (d.status === 'failed') return shortError(d.error_message) || 'Failed'
  return statusLabel(d.status)
}

function formatPayload(payload: NotificationEvent): string {
  return JSON.stringify(payload, null, 2)
}
</script>

<template>
  <LogRow
    :tone="deliveryTone(delivery.status)"
    :title="eventLabel"
    :subtitle="channelName"
    :facts="[outcome(delivery)]"
    :time="formatDateShort(delivery.attempted_at)"
    :time-title="formatDate(delivery.attempted_at)"
    :expanded="expanded"
    @toggle="emit('toggle')"
  >
    <template #detail>
      <dl class="info-grid">
        <dt>Time</dt>
        <dd>{{ formatDate(delivery.attempted_at) }}</dd>
        <dt>Channel</dt>
        <dd>{{ channelName }}</dd>
        <dt>Status</dt>
        <dd>{{ statusLabel(delivery.status) }}</dd>
      </dl>
      <pre
        v-if="delivery.error_message"
        class="error-pre"
        >{{ delivery.error_message }}</pre
      >
      <h3 class="group-label">Payload</h3>
      <pre class="detail-pre">{{ formatPayload(delivery.payload) }}</pre>
    </template>
  </LogRow>
</template>

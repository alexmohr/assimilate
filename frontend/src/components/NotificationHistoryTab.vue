<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { ChevronDown, Send } from '@lucide/vue'
import EmptyState from './EmptyState.vue'
import { formatDate } from '../utils/format'
import type {
  NotificationChannel,
  NotificationDelivery,
  NotificationEventType,
} from '../types/notifications'

/**
 * The delivery log: one row per attempt, expanding to show the payload and
 * any error.
 */
const props = defineProps<{
  deliveries: NotificationDelivery[]
  /** Used to resolve a delivery's channel id back to its name. */
  channels: NotificationChannel[]
  /** Shared with the channel list, so both spell an event type the same way. */
  eventTypeLabel: (event: NotificationEventType) => string
}>()

const expandedId = ref<number | null>(null)

function toggleExpand(id: number): void {
  expandedId.value = expandedId.value === id ? null : id
}

function channelNameById(id: number): string {
  return props.channels.find((c) => c.id === id)?.name ?? String(id)
}

function statusClass(status: NotificationDelivery['status']): string {
  if (status === 'sent') return 'status-sent'
  if (status === 'failed') return 'status-failed'
  return 'status-pending'
}

function formatPayload(payload: unknown): string {
  return JSON.stringify(payload, null, 2)
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
    v-else
    class="table-wrap table-wrap--framed"
  >
    <table class="data-table data-table-expandable">
      <thead>
        <tr>
          <th class="col-expand"></th>
          <th>Channel</th>
          <th>Event</th>
          <th>Status</th>
          <th>Error</th>
          <th>Time</th>
        </tr>
      </thead>
      <tbody>
        <template
          v-for="d in deliveries"
          :key="d.id"
        >
          <tr
            class="delivery-row"
            :class="{ expanded: expandedId === d.id }"
            :aria-expanded="expandedId === d.id"
            @click="toggleExpand(d.id)"
          >
            <td class="col-expand">
              <ChevronDown
                :size="14"
                class="expand-chevron"
              />
            </td>
            <td data-label="Channel">{{ channelNameById(d.channel_id) }}</td>
            <td data-label="Event">{{ eventTypeLabel(d.event_type) }}</td>
            <td data-label="Status">
              <span
                class="delivery-status"
                :class="statusClass(d.status)"
              >
                {{ d.status }}
              </span>
            </td>
            <td
              data-label="Error"
              class="mono cell-truncate"
            >
              {{ d.error_message ?? '\u2014' }}
            </td>
            <td data-label="Time">{{ formatDate(d.attempted_at) }}</td>
          </tr>
          <tr
            v-if="expandedId === d.id"
            class="detail-row"
          >
            <td colspan="6">
              <div class="detail-panel">
                <div
                  v-if="d.error_message"
                  class="detail-block"
                >
                  <span class="detail-block-label">Error</span>
                  <pre class="error-pre">{{ d.error_message }}</pre>
                </div>
                <div class="detail-block">
                  <span class="detail-block-label">Payload</span>
                  <pre class="detail-pre">{{ formatPayload(d.payload) }}</pre>
                </div>
              </div>
            </td>
          </tr>
        </template>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.col-expand {
  width: 2rem;
  padding-right: 0 !important;
}

/* Rows expand on click, so they get a hover cue the shared table does not
   give non-interactive rows. The rest of the table now inherits `.data-table`
   rather than restating its padding and colours. */
.delivery-row {
  cursor: pointer;
}

.delivery-row:hover td {
  background: var(--bg-hover);
}

.expand-chevron {
  color: var(--text-muted);
  transition: transform var(--duration-base);
}

.delivery-row.expanded .expand-chevron {
  transform: rotate(-180deg);
}

.detail-row td {
  padding: 0;
  background: var(--bg-hover);
}

.detail-panel {
  padding: var(--space-6) var(--space-7);
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.detail-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.detail-block-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  color: var(--text-muted);
}

.detail-pre.error-pre {
  color: var(--danger);
}

.delivery-status {
  font-size: var(--fs-xs);
  padding: var(--space-1) var(--space-4);
  border-radius: var(--radius-sm);
  font-weight: 500;
}

.status-sent {
  background: color-mix(in srgb, var(--success) 15%, transparent);
  color: var(--success);
}

.status-failed {
  background: var(--danger-subtle);
  color: var(--danger);
}

.status-pending {
  background: color-mix(in srgb, var(--warning) 15%, transparent);
  color: var(--warning);
}

/* Below this breakpoint the table restructures into a stacked card list per
   row instead of scrolling horizontally, so long values (errors, payloads)
   wrap in place rather than forcing the whole table wider than the viewport. */
@media (max-width: 640px) {
  .data-table-expandable thead {
    display: none;
  }

  .data-table-expandable tbody {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .data-table-expandable .delivery-row {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-card);
    padding: var(--space-4) var(--space-5);
  }

  .data-table-expandable .delivery-row td {
    border-bottom: none;
  }

  .data-table-expandable .col-expand {
    display: none;
  }

  .data-table-expandable td {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-5);
    padding: var(--space-3) 0;
    text-align: right;
    white-space: normal;
    word-break: break-word;
  }

  .data-table-expandable td::before {
    content: attr(data-label);
    flex-shrink: 0;
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-muted);
  }

  .data-table-expandable .cell-truncate {
    max-width: none;
    overflow: visible;
    text-overflow: clip;
    white-space: normal;
  }

  .data-table-expandable .detail-row td {
    display: block;
    background: transparent;
    text-align: left;
  }

  .data-table-expandable .detail-row td::before {
    content: none;
  }
}
</style>

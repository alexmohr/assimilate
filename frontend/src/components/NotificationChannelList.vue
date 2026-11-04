<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { AlertTriangle, BellRing, Globe, Mail } from '@lucide/vue'
import OverflowMenu from './OverflowMenu.vue'
import { relativeTime } from '../utils/format'
import {
  channelFailureSummaries,
  channelTarget,
  deliveryTone,
  lastDeliveryLabel,
  latestDeliveryByChannel,
  recentDeliveries,
  shortError,
  sortChannelsByTrouble,
  type ChannelFailureSummary,
  type DeliveryTone,
} from '../utils/notificationChannels'
import type {
  ChannelType,
  EventType,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
} from '../types/generated'

/**
 * The Channels tab: a call-out for any channel that has been failing, the
 * channels as one panel of rows, and the last few deliveries under them.
 *
 * A row carries no buttons of its own. Clicking it opens the channel's edit
 * dialog; everything else a channel can do - its content, events and scope,
 * a test send, pausing it, deleting it - is in the row's overflow menu.
 */
const props = defineProps<{
  channels: NotificationChannelResponse[]
  deliveries: NotificationDeliveryResponse[]
  /** "9 of 14 events" for a channel id. */
  eventsLabel: (channelId: number) => string
  /** "All", or what the channel is narrowed to. */
  scopeLabel: (channel: NotificationChannelResponse) => string
  eventTypeLabel: (event: EventType) => string
  channelTypeLabel: (type: ChannelType) => string
  /** The channel a test send is in flight for, if any. */
  testingChannelId: number | null
}>()

const emit = defineEmits<{
  edit: [channel: NotificationChannelResponse]
  editContent: [channel: NotificationChannelResponse]
  editEvents: [channel: NotificationChannelResponse]
  editScope: [channel: NotificationChannelResponse]
  test: [channel: NotificationChannelResponse]
  toggle: [channel: NotificationChannelResponse]
  delete: [channel: NotificationChannelResponse]
  /** Open the History tab, narrowed to one channel or (null) to all of them. */
  history: [channelId: number | null]
}>()

/** How many deliveries the "Recent deliveries" panel shows. */
const RECENT_COUNT = 5

const failures = computed((): ChannelFailureSummary[] =>
  channelFailureSummaries(props.channels, props.deliveries),
)

const sortedChannels = computed((): NotificationChannelResponse[] =>
  sortChannelsByTrouble(props.channels, new Set(failures.value.map((f) => f.channel.id))),
)

const latest = computed(
  (): Map<number, NotificationDeliveryResponse> => latestDeliveryByChannel(props.deliveries),
)

const recent = computed((): NotificationDeliveryResponse[] =>
  recentDeliveries(props.deliveries, RECENT_COUNT),
)

function typeIcon(type: ChannelType): typeof Mail {
  if (type === 'email') return Mail
  if (type === 'webhook') return Globe
  return BellRing
}

function failureTitle(f: ChannelFailureSummary): string {
  if (f.total === 1) return `${f.channel.name} failed its last delivery`
  return `${f.channel.name} failed ${f.failed} of its last ${f.total} deliveries`
}

function channelName(id: number): string {
  return props.channels.find((c) => c.id === id)?.name ?? `Channel ${id}`
}

function lastDelivery(
  channel: NotificationChannelResponse,
): NotificationDeliveryResponse | undefined {
  return latest.value.get(channel.id)
}

/** The tone of a channel's last delivery, or none when nothing was sent. */
function channelTone(channel: NotificationChannelResponse): DeliveryTone | null {
  const d = lastDelivery(channel)
  return d ? deliveryTone(d.status) : null
}

function dotClass(tone: DeliveryTone | null): string {
  return tone ? `status-dot status-dot--${tone}` : 'status-dot'
}

function deliveryDetail(d: NotificationDeliveryResponse): string {
  if (d.status === 'sent') return 'Delivered'
  if (d.status === 'pending') return 'Pending'
  return shortError(d.error_message) || 'Failed'
}

function channelSub(channel: NotificationChannelResponse): string {
  return `${props.channelTypeLabel(channel.channel_type)} \u00b7 ${channelTarget(channel)}`
}

function narrowSummary(channel: NotificationChannelResponse): string {
  const parts = [props.eventsLabel(channel.id), props.scopeLabel(channel)]
  if (!channel.enabled) parts.push('Paused')
  return parts.join(' · ')
}
</script>

<template>
  <div class="list-stack channel-list">
    <div
      v-for="f in failures"
      :key="f.channel.id"
      class="notice-bar notice-bar--danger"
      data-testid="channel-failure"
    >
      <AlertTriangle :size="16" />
      <div class="notice-bar-text">
        <span class="notice-bar-title">{{ failureTitle(f) }}</span>
        <span
          v-if="f.lastError"
          class="notice-error"
        >
          Last error: {{ f.lastError }}
        </span>
      </div>
      <button
        class="btn btn-sm btn-ghost"
        type="button"
        @click="emit('history', f.channel.id)"
      >
        View history
      </button>
    </div>

    <section class="list-group channel-group">
      <div class="list-group-header">
        <h2 class="list-group-title">Channels</h2>
        <span class="list-group-count">{{ channels.length }}</span>
      </div>
      <div
        class="list-cols"
        aria-hidden="true"
      >
        <span>Channel</span>
        <span>Events</span>
        <span>Scope</span>
        <span>Last delivery</span>
        <span>State</span>
        <span></span>
      </div>
      <div
        v-for="channel in sortedChannels"
        :key="channel.id"
        class="list-row list-row--link channel-row"
        tabindex="0"
        :data-channel="channel.name"
        :aria-label="`Edit ${channel.name}`"
        @click="emit('edit', channel)"
        @keydown.enter.self="emit('edit', channel)"
      >
        <div class="list-row-main">
          <span
            class="list-narrow-only"
            :class="dotClass(channelTone(channel))"
          ></span>
          <span class="channel-tile list-wide-only">
            <component
              :is="typeIcon(channel.channel_type)"
              :size="16"
            />
          </span>
          <div class="list-row-text">
            <span class="list-row-name">{{ channel.name }}</span>
            <span class="list-row-sub">{{ channelSub(channel) }}</span>
          </div>
        </div>
        <span class="list-row-value list-wide-only channel-events">
          {{ eventsLabel(channel.id) }}
        </span>
        <span class="list-row-value list-wide-only channel-scope">
          {{ scopeLabel(channel) }}
        </span>
        <span class="list-narrow-only channel-summary">{{ narrowSummary(channel) }}</span>
        <span
          class="channel-delivery"
          :class="{ 'channel-delivery--danger': channelTone(channel) === 'danger' }"
        >
          <span
            class="list-wide-only"
            :class="dotClass(channelTone(channel))"
          ></span>
          <span class="channel-delivery-text">{{ lastDeliveryLabel(lastDelivery(channel)) }}</span>
        </span>
        <span class="list-wide-only">
          <span
            class="badge"
            :class="channel.enabled ? 'badge--success' : 'badge--neutral'"
          >
            {{ channel.enabled ? 'On' : 'Paused' }}
          </span>
        </span>
        <!-- The menu's own clicks must not reach the row, which opens the editor. -->
        <div
          class="channel-row-menu"
          @click.stop
          @keydown.stop
        >
          <OverflowMenu
            v-slot="{ run }"
            :label="`More actions for ${channel.name}`"
          >
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="run(() => emit('edit', channel))"
            >
              Edit
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="run(() => emit('editContent', channel))"
            >
              Edit content
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="run(() => emit('editEvents', channel))"
            >
              Edit events
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="run(() => emit('editScope', channel))"
            >
              Edit scope
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              :disabled="testingChannelId === channel.id"
              @click="run(() => emit('test', channel))"
            >
              {{ testingChannelId === channel.id ? 'Sending...' : 'Send test' }}
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="run(() => emit('toggle', channel))"
            >
              {{ channel.enabled ? 'Pause' : 'Resume' }}
            </button>
            <button
              class="overflow-menu-item overflow-menu-item--danger"
              role="menuitem"
              type="button"
              @click="run(() => emit('delete', channel))"
            >
              Delete
            </button>
          </OverflowMenu>
        </div>
      </div>
    </section>

    <section
      v-if="recent.length > 0"
      class="list-group recent-group"
    >
      <div class="list-group-header">
        <h2 class="list-group-title">Recent deliveries</h2>
        <div class="list-group-aside">
          <button
            class="btn btn-sm btn-ghost"
            type="button"
            @click="emit('history', null)"
          >
            All history
          </button>
        </div>
      </div>
      <div
        v-for="d in recent"
        :key="d.id"
        class="list-row"
      >
        <div class="list-row-main">
          <span :class="dotClass(deliveryTone(d.status))"></span>
          <span class="recent-event">{{ eventTypeLabel(d.event_type) }}</span>
        </div>
        <span class="recent-channel">{{ channelName(d.channel_id) }}</span>
        <span
          class="recent-detail"
          :class="{ 'channel-delivery--danger': d.status === 'failed' }"
        >
          {{ deliveryDetail(d) }}
        </span>
        <span class="list-row-value recent-when">{{ relativeTime(d.attempted_at) }}</span>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* The overflow menu drops below its row, so the panel must not clip it. */
.channel-list .channel-group {
  --list-cols: minmax(0, 2.4fr) minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1.8fr) 5rem 2.5rem;
  overflow: visible;
}

.channel-list .recent-group {
  --list-cols: minmax(0, 1.4fr) minmax(0, 1fr) minmax(0, 2fr) 5rem;
}

.channel-tile {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  flex: none;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-secondary);
}

.channel-events,
.channel-scope,
.recent-channel {
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.channel-delivery {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-width: 0;
  color: var(--text-secondary);
}

.channel-delivery-text,
.recent-detail,
.notice-error {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.channel-delivery--danger {
  color: var(--danger);
}

.notice-error {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.channel-row-menu {
  justify-self: end;
}

.recent-event {
  font-weight: 600;
}

.recent-detail {
  color: var(--text-secondary);
}

.recent-when {
  color: var(--text-secondary);
  justify-self: end;
}

@media (max-width: 640px) {
  /* The menu sits in the row's top-right corner, where the name line leaves
     room for it. */
  .channel-row {
    padding-right: calc(var(--space-5) + 32px);
  }

  .channel-row-menu {
    position: absolute;
    top: var(--space-3);
    right: var(--space-3);
  }

  .channel-summary {
    flex: 1 1 100%;
    color: var(--text-secondary);
  }

  .channel-delivery {
    flex: 1 1 100%;
  }

  .recent-channel,
  .recent-detail {
    flex: 0 1 auto;
  }

  .recent-when {
    margin-left: auto;
  }
}
</style>

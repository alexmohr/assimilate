<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { AlertTriangle, BellRing, ChevronRight, Globe, Mail } from '@lucide/vue'
import {
  channelFailureSummaries,
  channelTarget,
  deliveryTone,
  lastDeliveryLabel,
  latestDeliveryByChannel,
  sortChannelsByTrouble,
  type ChannelFailureSummary,
  type DeliveryTone,
} from '../utils/notificationChannels'
import type {
  ChannelType,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
} from '../types/generated'

/**
 * The Channels tab: a call-out for any channel that has been failing, and the
 * channels as one panel of rows.
 *
 * A row carries no buttons of its own: it opens the channel's page, where
 * everything a channel can do - its settings, events, scope and content, a
 * test send, pausing it, deleting it, its delivery history - lives. Every
 * channel's history together is the page's History tab.
 */
const props = defineProps<{
  channels: NotificationChannelResponse[]
  deliveries: NotificationDeliveryResponse[]
  /** "9 of 14 events" for a channel id. */
  eventsLabel: (channelId: number) => string
  /** "All", or what the channel is narrowed to. */
  scopeLabel: (channel: NotificationChannelResponse) => string
  channelTypeLabel: (type: ChannelType) => string
}>()

const failures = computed((): ChannelFailureSummary[] =>
  channelFailureSummaries(props.channels, props.deliveries),
)

const sortedChannels = computed((): NotificationChannelResponse[] =>
  sortChannelsByTrouble(props.channels, new Set(failures.value.map((f) => f.channel.id))),
)

const latest = computed(
  (): Map<number, NotificationDeliveryResponse> => latestDeliveryByChannel(props.deliveries),
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

function channelLink(channel: NotificationChannelResponse): string {
  return `/notifications/channels/${channel.id}`
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
      <RouterLink
        class="btn btn-sm btn-ghost"
        :to="{ path: channelLink(f.channel), query: { section: 'history' } }"
      >
        View history
      </RouterLink>
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
      <RouterLink
        v-for="channel in sortedChannels"
        :key="channel.id"
        class="list-row"
        :to="channelLink(channel)"
        :data-channel="channel.name"
      >
        <div class="list-row-main">
          <span
            :class="dotClass(channelTone(channel))"
            role="img"
            :aria-label="lastDeliveryLabel(lastDelivery(channel))"
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
          <span class="channel-delivery-text">{{ lastDeliveryLabel(lastDelivery(channel)) }}</span>
        </span>
        <span class="list-row-value list-wide-only muted">{{
          channel.enabled ? '' : 'Paused'
        }}</span>
        <ChevronRight
          :size="16"
          class="list-row-chevron"
        />
      </RouterLink>
    </section>
  </div>
</template>

<style scoped>
.channel-list .channel-group {
  --list-cols: minmax(0, 2.4fr) minmax(0, 1fr) minmax(0, 1fr) minmax(0, 1.8fr) 5rem 2.5rem;
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
.channel-scope {
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

@media (max-width: 640px) {
  .channel-summary {
    flex: 1 1 100%;
    color: var(--text-secondary);
  }

  .channel-delivery {
    flex: 1 1 100%;
  }
}
</style>

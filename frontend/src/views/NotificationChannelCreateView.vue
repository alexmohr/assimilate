<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import BaseWizard, { type WizardStep } from '../components/BaseWizard.vue'
import ChannelConfigFields from '../components/ChannelConfigFields.vue'
import NotificationEventToggles from '../components/NotificationEventToggles.vue'
import NotificationLinkPicker from '../components/NotificationLinkPicker.vue'
import PaneRow from '../components/PaneRow.vue'
import ScopeSelector from '../components/ScopeSelector.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { createChannel, createRule, getVapidPublicKey } from '../api/notifications'
import { usePushSubscription } from '../composables/usePushSubscription'
import { useScopeOptions } from '../composables/useScopeOptions'
import { configInputFor } from '../utils/channelConfig'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import {
  DEFAULT_NOTIFICATION_LINK,
  notificationLinkLabel,
  notificationLinkProblem,
  type NotificationLink,
} from '../utils/notificationLink'
import {
  CHANNEL_TYPES,
  EVENT_TYPES,
  channelScopeLabel,
  channelTypeLabel,
  eventTypeLabel,
} from '../utils/notificationEvents'
import { validateEmailConfig } from '../utils/smtpValidation'
import {
  duplicateHeaderNames,
  headersForRequest,
  type WebhookHeaderRow,
} from '../utils/webhookHeaders'
import type {
  ChannelScope,
  ChannelType,
  CreateChannelRequest,
  EmailConfigInput,
  EventType,
  NotificationChannelResponse,
  WebhookConfigInput,
} from '../types/generated'

/**
 * A new notification channel, on the same wizard as a new schedule or
 * dependency: how it delivers, which events it is told about, which
 * repositories, hosts and schedules it covers, where its notifications link
 * to, and a review.
 */
type StepId = 'delivery' | 'events' | 'scope' | 'link' | 'review'

const STEPS: readonly WizardStep<StepId>[] = [
  { id: 'delivery', label: 'Delivery', sub: 'Type and where to' },
  { id: 'events', label: 'Events', sub: 'What it is told' },
  { id: 'scope', label: 'Scope', sub: 'What it covers' },
  { id: 'link', label: 'Link', sub: 'Where it leads' },
  { id: 'review', label: 'Review', sub: 'Create channel' },
]

const router = useRouter()
const { scopeRepos, scopeAgents, scopeSchedules, scopeSearch, loadScopeOptions } = useScopeOptions()
const { ensurePushSubscription } = usePushSubscription()

const name = ref('')
const channelType = ref<ChannelType>('email')
const enabled = ref(true)
const emailCfg = ref<EmailConfigInput>({
  smtp_host: '',
  smtp_port: 587,
  smtp_user: '',
  smtp_password: '',
  from_address: '',
  to_addresses: [],
  security: 'starttls',
})
const webhookCfg = ref<WebhookConfigInput>({ url: '' })
const headerRows = ref<WebhookHeaderRow[]>([])
const toAddressesInput = ref('')
const events = ref<EventType[]>([])
const scope = ref<ChannelScope>({})
const link = ref<NotificationLink>({ ...DEFAULT_NOTIFICATION_LINK })
const vapidConfigured = ref(false)
const submitting = ref(false)
const error = ref<string | null>(null)

onMounted(async () => {
  loadScopeOptions().catch(logger.error)
  try {
    vapidConfigured.value = (await getVapidPublicKey()).configured
  } catch (e: unknown) {
    logger.error('loading the push key failed', e)
  }
})

function blockers(step: StepId): string[] {
  const missing: string[] = []
  if (step === 'delivery') {
    if (!name.value.trim()) missing.push('a name')
    if (channelType.value === 'email') {
      if (!emailCfg.value.smtp_host.trim()) missing.push('an SMTP server')
      if (!emailCfg.value.from_address.trim()) missing.push('a sender')
      if (!toAddressesInput.value.trim()) missing.push('a recipient')
    } else if (channelType.value === 'webhook') {
      if (!webhookCfg.value.url.trim()) missing.push('a webhook URL')
      const duplicates = duplicateHeaderNames(headerRows.value)
      if (duplicates.length > 0) missing.push(`each header given once (${duplicates.join(', ')})`)
    } else if (!vapidConfigured.value) {
      missing.push('push keys configured on the server')
    }
  }
  if (step === 'link') {
    const problem = notificationLinkProblem(link.value)
    if (problem) missing.push(problem)
  }
  return missing
}

function toggleEvent(et: EventType): void {
  events.value = events.value.includes(et)
    ? events.value.filter((e) => e !== et)
    : [...events.value, et]
}

function toggleScopeItem(type: keyof ChannelScope, id: number): void {
  const current = scope.value[type] ?? []
  const updated = current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
  scope.value = { ...scope.value, [type]: updated }
}

/** The review's scope line, read off the same labels as the channels list. */
const scopeText = computed(() =>
  channelScopeLabel({ scope: scope.value } as NotificationChannelResponse),
)

async function submit(): Promise<void> {
  error.value = null
  if (channelType.value === 'email') {
    emailCfg.value.to_addresses = toAddressesInput.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  if (channelType.value === 'webhook') {
    // A repeated header blocks the Delivery step, so it never gets this far.
    webhookCfg.value.headers = headersForRequest(headerRows.value)
  }
  submitting.value = true
  try {
    if (channelType.value === 'email') {
      const verdict = await validateEmailConfig(emailCfg.value)
      if (!verdict.success) {
        error.value = verdict.message
        return
      }
    }
    if (channelType.value === 'web_push') await ensurePushSubscription()
    const req: CreateChannelRequest & { link: NotificationLink } = {
      name: name.value,
      enabled: enabled.value,
      scope: scope.value,
      link: link.value,
      ...configInputFor(channelType.value, { email: emailCfg.value, webhook: webhookCfg.value }),
    }
    const created = await createChannel(req)
    await Promise.all(
      events.value.map((et) =>
        createRule({ channel_id: created.id, event_type: et, enabled: true }),
      ),
    )
    router.push(`/notifications/channels/${created.id}`)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="channel-create-view">
    <div class="detail-breadcrumb">
      <RouterLink
        to="/notifications"
        class="crumb-link"
        >Notifications</RouterLink
      >
      <span class="crumb-sep">/</span>
      <span class="crumb-current">New channel</span>
    </div>

    <div class="page-header">
      <h1 class="page-title">New Channel</h1>
    </div>
    <p class="page-description">
      Where Assimilate sends its notifications: an email address, a webhook, or the browsers
      subscribed to web push.
    </p>

    <BaseWizard
      v-slot="{ step, goTo }"
      :steps="STEPS"
      :blockers="blockers"
      label="Channel steps"
      submit-label="Create channel"
      :submitting="submitting"
      @cancel="router.push('/notifications')"
      @submit="submit"
    >
      <template v-if="step === 'delivery'">
        <p class="pane-lede">How this channel delivers, and where to.</p>
        <div class="field">
          <label
            class="field-label"
            for="channel-type"
            >Type <span class="required">*</span></label
          >
          <select
            id="channel-type"
            v-model="channelType"
            class="input"
          >
            <option
              v-for="ct in CHANNEL_TYPES"
              :key="ct"
              :value="ct"
            >
              {{ channelTypeLabel(ct) }}
            </option>
          </select>
        </div>
        <div class="field">
          <label
            class="field-label"
            for="channel-name"
            >Name <span class="required">*</span></label
          >
          <input
            id="channel-name"
            v-model="name"
            class="input"
            placeholder="e.g. Ops Email"
          />
        </div>
        <!-- One-way on purpose: ChannelConfigFields edits the drafts in place. -->
        <ChannelConfigFields
          v-model:to-addresses="toAddressesInput"
          v-model:webhook-headers="headerRows"
          :email-config="emailCfg"
          :webhook-config="webhookCfg"
          :channel-type="channelType"
          show-required
        />
        <p
          v-if="channelType === 'web_push' && !vapidConfigured"
          class="wizard-note wizard-note--warning"
        >
          Web push needs its keys configured on the server before a channel can use it.
        </p>
        <div class="pane-rows">
          <PaneRow title="Enable immediately">
            <ToggleSwitch
              v-model="enabled"
              label="Enable immediately"
            />
          </PaneRow>
        </div>
      </template>

      <template v-else-if="step === 'events'">
        <p class="pane-lede">Which events this channel is told about.</p>
        <NotificationEventToggles
          :event-types="EVENT_TYPES"
          :label-for="eventTypeLabel"
          :is-enabled="(et) => events.includes(et)"
          @toggle="toggleEvent"
        />
      </template>

      <template v-else-if="step === 'scope'">
        <p class="pane-lede">
          Restrict this channel to events from specific repositories, hosts or schedules. Leave
          everything unticked for all of them.
        </p>
        <ScopeSelector
          v-model:search="scopeSearch"
          :selected="scope"
          :repos="scopeRepos"
          :agents="scopeAgents"
          :schedules="scopeSchedules"
          @toggle="toggleScopeItem"
        />
      </template>

      <template v-else-if="step === 'link'">
        <p class="pane-lede">
          Where a notification takes whoever opens it: the click on a push notification, the link in
          an email, the <span class="mono">url</span> of a webhook payload.
        </p>
        <NotificationLinkPicker
          v-model="link"
          id-prefix="new-channel-link"
        />
      </template>

      <template v-else>
        <p class="pane-lede">
          What will be created. Every block links back to the step that owns it.
        </p>
        <div class="review-grid">
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Delivery</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('delivery')"
              >
                Edit
              </button>
            </div>
            <dl class="info-grid">
              <dt>Name</dt>
              <dd>{{ name || '—' }}</dd>
              <dt>Type</dt>
              <dd>{{ channelTypeLabel(channelType) }}</dd>
              <template v-if="channelType === 'email'">
                <dt>To</dt>
                <dd>{{ toAddressesInput || '—' }}</dd>
              </template>
              <template v-else-if="channelType === 'webhook'">
                <dt>URL</dt>
                <dd class="mono">{{ webhookCfg.url || '—' }}</dd>
              </template>
              <dt>State</dt>
              <dd>{{ enabled ? 'On' : 'Paused' }}</dd>
            </dl>
          </div>
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Events</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('events')"
              >
                Edit
              </button>
            </div>
            <p class="muted">
              {{
                events.length === 0 ? 'None yet' : events.map((et) => eventTypeLabel(et)).join(', ')
              }}
            </p>
          </div>
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Scope</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('scope')"
              >
                Edit
              </button>
            </div>
            <p class="muted">{{ scopeText }}</p>
          </div>
          <div class="review-block">
            <div class="review-head">
              <span class="group-label">Link</span>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                @click="goTo('link')"
              >
                Edit
              </button>
            </div>
            <p class="muted">{{ notificationLinkLabel(link) }}</p>
          </div>
        </div>
        <div
          v-if="error"
          class="form-error"
        >
          {{ error }}
        </div>
      </template>
    </BaseWizard>
  </div>
</template>

<style scoped>
.channel-create-view {
  max-width: 1000px;
  min-width: 0;
}
</style>

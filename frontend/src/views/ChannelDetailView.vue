<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  createRule,
  deleteChannel,
  deleteRule,
  listChannels,
  listDeliveries,
  listRules,
  testChannel,
  updateChannel,
} from '../api/notifications'
import { useAsyncAction } from '../composables/useAsyncAction'
import { usePushSubscription } from '../composables/usePushSubscription'
import { useScopeOptions } from '../composables/useScopeOptions'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import { configInputFor } from '../utils/channelConfig'
import {
  CHANNEL_HISTORY_FETCH,
  isChannelSection,
  smtpSecurityLabel,
  type ChannelSection,
} from '../utils/channelSettings'
import { extractError } from '../utils/error'
import {
  DEFAULT_NOTIFICATION_LINK,
  channelLink,
  notificationLinkLabel,
  notificationLinkProblem,
  type NotificationLink,
} from '../utils/notificationLink'
import { channelTarget, lastDeliveryLabel, newestFirst } from '../utils/notificationChannels'
import {
  EVENT_TYPES,
  channelEventsLabel,
  channelScopeLabel,
  channelTypeLabel,
  eventTypeLabel,
} from '../utils/notificationEvents'
import { validateEmailConfig } from '../utils/smtpValidation'
import {
  duplicateHeaderNames,
  headersForRequest,
  rowsFromSaved,
  type WebhookHeaderRow,
} from '../utils/webhookHeaders'
import type {
  ChannelScope,
  EmailConfigInput,
  EventType,
  NotificationChannelResponse,
  NotificationDeliveryResponse,
  NotificationRuleResponse,
  UpdateChannelRequest,
  WebhookConfigInput,
} from '../types/generated'
import BaseSpinner from '../components/BaseSpinner.vue'
import ChannelConfigFields from '../components/ChannelConfigFields.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import DetailHeader from '../components/DetailHeader.vue'
import EditableSection from '../components/EditableSection.vue'
import NotificationContentEditor from '../components/NotificationContentEditor.vue'
import NotificationEventToggles from '../components/NotificationEventToggles.vue'
import NotificationHistoryTab from '../components/NotificationHistoryTab.vue'
import OverflowMenu from '../components/OverflowMenu.vue'
import NotificationLinkPicker from '../components/NotificationLinkPicker.vue'
import PaneRow from '../components/PaneRow.vue'
import ScopeSelector from '../components/ScopeSelector.vue'
import SettingsRail, { type SettingsSections } from '../components/SettingsRail.vue'

/**
 * One notification channel: where it delivers, which events it is told
 * about, which repositories, hosts and schedules those events may come from,
 * what its messages say, and what it has sent. Admin only, like the
 * Notifications page it is opened from. Sending a test, pausing and deleting
 * it are in the header's overflow menu.
 */
const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const toast = useToast()
const { ensurePushSubscription } = usePushSubscription()

const channelId = computed(() => Number(props.id))
const channel = ref<NotificationChannelResponse | null>(null)
const rules = ref<NotificationRuleResponse[]>([])
const deliveries = ref<NotificationDeliveryResponse[]>([])
const { loading, error, run } = useAsyncAction()

const { scopeRepos, scopeAgents, scopeSchedules, scopeSearch, loadScopeOptions } = useScopeOptions()

const section = computed<ChannelSection>({
  get() {
    const s = route.query.section
    return isChannelSection(s) ? s : 'settings'
  },
  set(val: ChannelSection) {
    void router.replace({ query: { ...route.query, section: val } })
  },
})

const sections: SettingsSections<ChannelSection> = [
  { id: 'settings', label: 'Delivery' },
  { id: 'events', label: 'Events' },
  { id: 'scope', label: 'Scope' },
  { id: 'link', label: 'Link' },
  { id: 'content', label: 'Content' },
  { id: 'history', label: 'Delivery history' },
]

/** This channel's deliveries, newest first. */
const ownDeliveries = computed((): NotificationDeliveryResponse[] =>
  newestFirst(deliveries.value.filter((d) => d.channel_id === channelId.value)),
)

/** The history list resolves a delivery's channel by id; this page has one. */
const historyChannels = computed((): NotificationChannelResponse[] =>
  channel.value ? [channel.value] : [],
)

const subtitle = computed((): string | null =>
  channel.value
    ? `${channelTypeLabel(channel.value.channel_type)} · ${channelTarget(channel.value)}`
    : null,
)

// Loading.

async function fetchAll(): Promise<void> {
  const id = channelId.value
  const [channelList, ruleList, deliveryList] = await Promise.all([
    listChannels(),
    listRules(),
    listDeliveries(CHANNEL_HISTORY_FETCH),
  ])
  if (channelId.value !== id) return
  const found = channelList.find((c) => c.id === id)
  if (!found) throw new Error('Channel not found')
  channel.value = found
  rules.value = ruleList
  deliveries.value = deliveryList
}

async function load(): Promise<void> {
  await run(fetchAll)
}

const { onMessage } = useWebSocket()
onMessage('NotificationDelivery', (data: NotificationDeliveryResponse) => {
  if (data.channel_id === channelId.value) deliveries.value = [data, ...deliveries.value]
})

watch(
  () => props.id,
  async () => {
    channel.value = null
    await load()
  },
)
onMounted(() => {
  void load()
  void loadScopeOptions()
})

// Delivery settings.

function blankEmail(): EmailConfigInput {
  return {
    smtp_host: '',
    smtp_port: 587,
    smtp_user: '',
    smtp_password: '',
    from_address: '',
    to_addresses: [],
    security: 'starttls',
  }
}

const editing = ref(false)
const saving = ref(false)
const editError = ref<string | null>(null)
const nameDraft = ref('')
const emailDraft = ref<EmailConfigInput>(blankEmail())
const webhookDraft = ref<WebhookConfigInput>({ url: '' })
const toAddressesDraft = ref('')
const headerRowsDraft = ref<WebhookHeaderRow[]>([])
const savedWebhookUrl = ref<string | undefined>(undefined)

function startEdit(): void {
  const ch = channel.value
  if (!ch) return
  nameDraft.value = ch.name
  if (ch.channel_type === 'email') {
    // The server never returns the stored password; blank here means "keep it".
    emailDraft.value = { ...ch.config, smtp_password: '' }
    toAddressesDraft.value = ch.config.to_addresses.join(', ')
  } else if (ch.channel_type === 'webhook') {
    webhookDraft.value = { ...ch.config }
  }
  // Header values are never returned either: the rows carry names only, and a
  // blank value keeps the saved one.
  headerRowsDraft.value = rowsFromSaved(ch.webhook_headers)
  savedWebhookUrl.value = ch.channel_type === 'webhook' ? ch.config.url : undefined
  editError.value = null
  editing.value = true
}

async function saveSettings(): Promise<void> {
  const ch = channel.value
  if (!ch) return
  if (!nameDraft.value.trim()) {
    editError.value = 'Name is required'
    return
  }
  if (ch.channel_type === 'email') {
    emailDraft.value.to_addresses = toAddressesDraft.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  if (ch.channel_type === 'webhook') {
    const duplicates = duplicateHeaderNames(headerRowsDraft.value)
    if (duplicates.length > 0) {
      editError.value = `Each header can only be given once: ${duplicates.join(', ')}`
      return
    }
    webhookDraft.value.headers = headersForRequest(headerRowsDraft.value)
  }
  saving.value = true
  editError.value = null
  try {
    if (ch.channel_type === 'email') {
      const verdict = await validateEmailConfig(emailDraft.value, ch.id)
      if (!verdict.success) {
        editError.value = verdict.message
        return
      }
    }
    // A web push channel has no settings of its own beyond its name.
    const req: UpdateChannelRequest =
      ch.channel_type === 'web_push'
        ? { name: nameDraft.value }
        : {
            name: nameDraft.value,
            ...configInputFor(ch.channel_type, {
              email: emailDraft.value,
              webhook: webhookDraft.value,
            }),
          }
    channel.value = await updateChannel(ch.id, req)
    editing.value = false
    toast.success('Channel saved')
  } catch (e: unknown) {
    editError.value = extractError(e)
  } finally {
    saving.value = false
  }
}

// Link: where this channel's notifications take whoever opens them.

const editingLink = ref(false)
const savingLink = ref(false)
const linkError = ref<string | null>(null)
const linkDraft = ref<NotificationLink>({ ...DEFAULT_NOTIFICATION_LINK })

function startLinkEdit(): void {
  if (!channel.value) return
  linkDraft.value = { ...channelLink(channel.value) }
  linkError.value = null
  editingLink.value = true
}

async function saveLink(): Promise<void> {
  const ch = channel.value
  if (!ch) return
  const problem = notificationLinkProblem(linkDraft.value)
  if (problem) {
    linkError.value = `Needs ${problem}.`
    return
  }
  savingLink.value = true
  linkError.value = null
  try {
    const req: UpdateChannelRequest & { link: NotificationLink } = {
      name: ch.name,
      link: linkDraft.value,
    }
    channel.value = await updateChannel(ch.id, req)
    editingLink.value = false
    toast.success('Link saved')
  } catch (e: unknown) {
    linkError.value = extractError(e)
  } finally {
    savingLink.value = false
  }
}

// Events.

const ruleToggling = ref<EventType | null>(null)

function isEventEnabled(et: EventType): boolean {
  return rules.value.some((r) => r.channel_id === channelId.value && r.event_type === et)
}

function isRuleToggling(et: EventType): boolean {
  return ruleToggling.value === et
}

async function toggleRule(et: EventType): Promise<void> {
  const id = channelId.value
  ruleToggling.value = et
  const existing = rules.value.find((r) => r.channel_id === id && r.event_type === et)
  try {
    if (existing) {
      await deleteRule(existing.id)
      rules.value = rules.value.filter((r) => r.id !== existing.id)
    } else {
      const created = await createRule({ channel_id: id, event_type: et, enabled: true })
      rules.value = [...rules.value, created]
    }
  } catch (e: unknown) {
    toast.error(`Could not change ${eventTypeLabel(et).toLowerCase()}: ${extractError(e)}`)
  } finally {
    ruleToggling.value = null
  }
}

// Scope.

async function toggleScopeItem(type: keyof ChannelScope, id: number): Promise<void> {
  const ch = channel.value
  if (!ch) return
  const current = ch.scope?.[type] ?? []
  const updated = current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
  try {
    channel.value = await updateChannel(ch.id, { scope: { ...ch.scope, [type]: updated } })
  } catch (e: unknown) {
    toast.error(`Could not change the scope: ${extractError(e)}`)
  }
}

// Header actions.

const testing = ref(false)

/** Sends a test through the channel and says how it went in a toast. */
async function sendTest(): Promise<void> {
  const ch = channel.value
  if (!ch) return
  testing.value = true
  try {
    if (ch.channel_type === 'web_push') await ensurePushSubscription()
    await testChannel(ch.id)
    toast.success(`Test sent to ${ch.name}`)
  } catch (e: unknown) {
    toast.error(`Test to ${ch.name} failed: ${extractError(e)}`)
  } finally {
    testing.value = false
  }
}

async function toggleEnabled(): Promise<void> {
  const ch = channel.value
  if (!ch) return
  try {
    channel.value = await updateChannel(ch.id, { enabled: !ch.enabled })
    toast.success(ch.enabled ? `Paused ${ch.name}` : `Resumed ${ch.name}`)
  } catch (e: unknown) {
    toast.error(extractError(e))
  }
}

const showDelete = ref(false)
const deleting = ref(false)
const deleteError = ref<string | null>(null)

async function confirmDelete(): Promise<void> {
  const ch = channel.value
  if (!ch) return
  deleting.value = true
  deleteError.value = null
  try {
    await deleteChannel(ch.id)
    toast.success(`Deleted ${ch.name}`)
    showDelete.value = false
    await router.push('/notifications')
  } catch (e: unknown) {
    deleteError.value = extractError(e)
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div class="channel-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/notifications"
        class="crumb-link"
      >
        Notifications
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current">{{ channel?.name ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !channel"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="channel">
      <DetailHeader
        :name="channel.name"
        :subtitle="subtitle"
      >
        <template #badges>
          <span class="badge badge--neutral">{{ channelTypeLabel(channel.channel_type) }}</span>
          <span
            class="badge"
            :class="channel.enabled ? 'badge--success' : 'badge--neutral'"
          >
            <span
              v-if="channel.enabled"
              class="badge-dot"
            />
            {{ channel.enabled ? 'On' : 'Paused' }}
          </span>
        </template>
        <template #meta>
          <span>{{ channelEventsLabel(rules, channel.id) }}</span>
          <span>Scope: {{ channelScopeLabel(channel) }}</span>
          <span>{{ lastDeliveryLabel(ownDeliveries[0]) }}</span>
        </template>
        <template #actions>
          <OverflowMenu
            v-slot="{ run: runAction }"
            label="More channel actions"
          >
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              :disabled="testing"
              @click="runAction(() => void sendTest())"
            >
              {{ testing ? 'Sending...' : 'Send test' }}
            </button>
            <button
              class="overflow-menu-item"
              role="menuitem"
              type="button"
              @click="runAction(() => void toggleEnabled())"
            >
              {{ channel.enabled ? 'Pause channel' : 'Resume channel' }}
            </button>
            <button
              class="overflow-menu-item overflow-menu-item--danger"
              role="menuitem"
              type="button"
              @click="runAction(() => (showDelete = true))"
            >
              Delete channel
            </button>
          </OverflowMenu>
        </template>
      </DetailHeader>

      <div class="tab-content fade-in">
        <SettingsRail
          v-slot="{ section: current }"
          :sections="sections"
          :section="section"
          label="Channel sections"
          @update:section="section = $event"
        >
          <EditableSection
            v-if="current === 'settings'"
            lede="Where this channel sends its notifications, and how it signs in there."
            :editing="editing"
            can-edit
            :saving="saving"
            :error="editError"
            @edit="startEdit"
            @cancel="editing = false"
            @save="saveSettings"
          >
            <template #view>
              <dl class="info-grid">
                <dt>Name</dt>
                <dd>{{ channel.name }}</dd>
                <dt>Type</dt>
                <dd>{{ channelTypeLabel(channel.channel_type) }}</dd>
                <template v-if="channel.channel_type === 'email'">
                  <dt>SMTP server</dt>
                  <dd class="mono">
                    {{ channel.config.smtp_host }}:{{ channel.config.smtp_port }}
                  </dd>
                  <dt>Security</dt>
                  <dd>{{ smtpSecurityLabel(channel.config.security) }}</dd>
                  <dt>SMTP user</dt>
                  <dd :class="{ muted: !channel.config.smtp_user }">
                    {{ channel.config.smtp_user || 'None' }}
                  </dd>
                  <dt>SMTP password</dt>
                  <dd :class="{ muted: !channel.has_password }">
                    {{ channel.has_password ? 'Saved' : 'None' }}
                  </dd>
                  <dt>From</dt>
                  <dd>{{ channel.config.from_address }}</dd>
                  <dt>To</dt>
                  <dd>{{ channel.config.to_addresses.join(', ') }}</dd>
                </template>
                <template v-else-if="channel.channel_type === 'webhook'">
                  <dt>URL</dt>
                  <dd class="mono channel-url">{{ channel.config.url }}</dd>
                  <dt>Headers</dt>
                  <dd :class="{ muted: channel.webhook_headers.length === 0 }">
                    {{
                      channel.webhook_headers.length > 0
                        ? channel.webhook_headers.map((h) => h.name).join(', ')
                        : 'None'
                    }}
                  </dd>
                </template>
                <template v-else>
                  <dt>Delivers to</dt>
                  <dd>Every browser subscribed to web push</dd>
                </template>
              </dl>
            </template>
            <template #edit>
              <div class="pane-rows">
                <PaneRow
                  title="Name"
                  label-for="channel-edit-name"
                >
                  <input
                    id="channel-edit-name"
                    v-model="nameDraft"
                    class="input"
                  />
                </PaneRow>
                <PaneRow
                  v-if="channel.channel_type !== 'web_push'"
                  :title="channel.channel_type === 'email' ? 'SMTP' : 'Webhook'"
                  stack
                >
                  <!-- One-way on purpose: ChannelConfigFields edits the drafts in
                       place and never replaces them. -->
                  <div class="form-stack">
                    <ChannelConfigFields
                      v-model:to-addresses="toAddressesDraft"
                      v-model:webhook-headers="headerRowsDraft"
                      :email-config="emailDraft"
                      :webhook-config="webhookDraft"
                      :channel-type="channel.channel_type"
                      :channel-id="channel.id"
                      :has-stored-password="channel.has_password"
                      :saved-webhook-url="savedWebhookUrl"
                    />
                  </div>
                </PaneRow>
              </div>
            </template>
          </EditableSection>

          <template v-else-if="current === 'events'">
            <div class="pane-head">
              <p class="pane-lede">
                Which events this channel is told about. Each switch applies at once.
              </p>
            </div>
            <NotificationEventToggles
              :event-types="EVENT_TYPES"
              :label-for="eventTypeLabel"
              :is-enabled="isEventEnabled"
              :is-disabled="isRuleToggling"
              @toggle="toggleRule"
            />
          </template>

          <template v-else-if="current === 'scope'">
            <div class="pane-head">
              <p class="pane-lede">
                Restrict this channel to events from specific repositories, hosts or schedules.
                Leave everything unticked for all of them. Each change applies at once.
              </p>
            </div>
            <ScopeSelector
              v-model:search="scopeSearch"
              :selected="channel.scope ?? {}"
              :repos="scopeRepos"
              :agents="scopeAgents"
              :schedules="scopeSchedules"
              @toggle="toggleScopeItem"
            />
          </template>

          <EditableSection
            v-else-if="current === 'link'"
            lede="Where a notification takes whoever opens it: the click on a push notification, the link in an email, the url of a webhook payload."
            :editing="editingLink"
            can-edit
            :saving="savingLink"
            :error="linkError"
            @edit="startLinkEdit"
            @cancel="editingLink = false"
            @save="saveLink"
          >
            <template #view>
              <dl class="info-grid">
                <dt>Links to</dt>
                <dd>{{ notificationLinkLabel(channelLink(channel)) }}</dd>
              </dl>
            </template>
            <template #edit>
              <NotificationLinkPicker
                v-model="linkDraft"
                id-prefix="channel-link"
              />
            </template>
          </EditableSection>
          <NotificationContentEditor
            v-else-if="current === 'content'"
            :key="channel.id"
            :channel="channel"
            standalone
            @updated="channel = $event"
          />

          <NotificationHistoryTab
            v-else-if="current === 'history'"
            :deliveries="ownDeliveries"
            :channels="historyChannels"
            :event-type-label="eventTypeLabel"
          />
        </SettingsRail>
      </div>

      <ConfirmDeleteDialog
        :show="showDelete"
        title="Delete channel"
        :submitting="deleting"
        :error="deleteError"
        confirm-label="Delete"
        submitting-label="Deleting..."
        @cancel="showDelete = false"
        @confirm="confirmDelete"
      >
        Delete channel <strong>{{ channel.name }}</strong
        >? All associated rules will also be removed.
      </ConfirmDeleteDialog>
    </template>
  </div>
</template>

<style scoped>
.channel-detail {
  max-width: 1200px;
}

.channel-url {
  overflow-wrap: anywhere;
}
</style>

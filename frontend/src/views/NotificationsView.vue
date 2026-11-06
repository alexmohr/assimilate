<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useWebSocket } from '../composables/useWebSocket'
import { useEscapeKey } from '../composables/useEscapeKey'
import { usePushSubscription } from '../composables/usePushSubscription'
import { extractError } from '../utils/error'
import { validateEmailConfig } from '../utils/smtpValidation'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import {
  listChannels,
  createChannel,
  listRules,
  createRule,
  getVapidPublicKey,
  subscribePush,
  listDeliveries,
} from '../api/notifications'
import { listRepos } from '../api/repos'
import { listAgents } from '../api/agents'
import { listSchedules } from '../api/schedules'
import { Plus, Bell } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import ChannelConfigFields from '../components/ChannelConfigFields.vue'
import NotificationEventToggles from '../components/NotificationEventToggles.vue'
import ScopeSelector, { type ScopeOption } from '../components/ScopeSelector.vue'
import NotificationHistoryTab from '../components/NotificationHistoryTab.vue'
import NotificationChannelList from '../components/NotificationChannelList.vue'
import { configInputFor } from '../utils/channelConfig'
import {
  CHANNEL_TYPES,
  EVENT_TYPES,
  channelEventsLabel,
  channelScopeLabel,
  channelTypeLabel,
  eventTypeLabel,
} from '../utils/notificationEvents'
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
  NotificationDeliveryResponse,
  NotificationRuleResponse,
  WebhookConfigInput,
} from '../types/generated'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

/**
 * Every notification channel as one panel of rows, and every channel's
 * delivery history under the History tab. A row opens the channel's own page,
 * where its settings, events, scope and content are edited, a test is sent,
 * and its own history is shown. Only creating a channel stays a dialog: the
 * three-step wizard.
 */
const TAB_IDS = ['channels', 'history'] as const
type TabId = (typeof TAB_IDS)[number]

function isTabId(value: unknown): value is TabId {
  return TAB_IDS.some((t) => t === value)
}

/** The add wizard's transport-independent fields; each transport keeps its own draft. */
interface AddChannelForm {
  name: string
  channel_type: ChannelType
  enabled: boolean
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
const scopeRepos = ref<ScopeOption[]>([])
const scopeAgents = ref<ScopeOption[]>([])
const scopeSchedules = ref<ScopeOption[]>([])
const scopeSearch = ref('')

const { isPushSupported, subscription, ensurePushSubscription } = usePushSubscription()

// Add channel wizard state
const addConfigFields = ref<InstanceType<typeof ChannelConfigFields> | null>(null)
const showAddChannelDialog = ref(false)
const wizardStep = ref(1)
const addChannelForm = ref<AddChannelForm>({
  name: '',
  channel_type: 'email',
  enabled: true,
})
const wizardEvents = ref<EventType[]>([])
const wizardScope = ref<ChannelScope>({})
const addChannelError = ref('')
const addChannelLoading = ref(false)
const toAddressesInput = ref('')

const vapidConfigured = ref(false)

function createEmailConfig(): EmailConfigInput {
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

function createWebhookConfig(): WebhookConfigInput {
  return { url: '' }
}

const addChannelEmailCfg = ref<EmailConfigInput>(createEmailConfig())
const addChannelWebhookCfg = ref<WebhookConfigInput>(createWebhookConfig())
const addHeaderRows = ref<WebhookHeaderRow[]>([])

const addChannelFormValid = computed((): boolean => {
  const form = addChannelForm.value
  if (!form.name.trim()) return false
  if (form.channel_type === 'web_push' && !vapidConfigured.value) return false
  if (form.channel_type === 'email') {
    if (
      !addChannelEmailCfg.value.smtp_host.trim() ||
      !addChannelEmailCfg.value.from_address.trim() ||
      !toAddressesInput.value.trim()
    )
      return false
  } else if (form.channel_type === 'webhook') {
    if (!addChannelWebhookCfg.value.url.trim()) return false
  }
  return true
})

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
    vapidConfigured.value = vapidStatus.configured
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

async function loadScopeOptions(): Promise<void> {
  try {
    const [repoRows, agentRows, scheduleRows] = await Promise.all([
      listRepos(),
      listAgents(),
      listSchedules(),
    ])
    scopeRepos.value = repoRows.map((r) => ({ id: r.id, label: r.name }))
    scopeAgents.value = agentRows.map((c) => ({
      id: c.id,
      label: c.display_name ?? c.hostname,
    }))
    scopeSchedules.value = scheduleRows.map((s) => ({
      id: s.id,
      label: `Schedule #${s.id}`,
    }))
  } catch (e: unknown) {
    logger.error('loadScopeOptions failed', e)
  }
}

function openAddChannel(): void {
  addChannelForm.value = {
    name: '',
    channel_type: 'email',
    enabled: true,
  }
  addChannelEmailCfg.value = createEmailConfig()
  addChannelWebhookCfg.value = createWebhookConfig()
  addHeaderRows.value = []
  toAddressesInput.value = ''
  addChannelError.value = ''
  wizardStep.value = 1
  wizardEvents.value = []
  wizardScope.value = {}
  scopeSearch.value = ''
  addConfigFields.value?.reset()
  showAddChannelDialog.value = true
}

async function submitAddChannel(): Promise<void> {
  if (!addChannelForm.value.name.trim()) {
    addChannelError.value = 'Name is required'
    return
  }
  if (addChannelForm.value.channel_type === 'email') {
    addChannelEmailCfg.value.to_addresses = toAddressesInput.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  if (addChannelForm.value.channel_type === 'webhook') {
    const duplicates = duplicateHeaderNames(addHeaderRows.value)
    if (duplicates.length > 0) {
      addChannelError.value = `Each header can only be given once: ${duplicates.join(', ')}`
      return
    }
    addChannelWebhookCfg.value.headers = headersForRequest(addHeaderRows.value)
  }
  addChannelLoading.value = true
  addChannelError.value = ''
  try {
    if (addChannelForm.value.channel_type === 'email') {
      // Checked directly rather than through addConfigFields: the SMTP fields
      // live on step 1, so by the time Create is pressed on step 3 that ref is
      // null and the gate would reject every email channel.
      const verdict = await validateEmailConfig(addChannelEmailCfg.value)
      if (!verdict.success) {
        addChannelError.value = verdict.message
        return
      }
    }
    if (addChannelForm.value.channel_type === 'web_push') {
      await ensurePushSubscription()
    }
    const req: CreateChannelRequest = {
      name: addChannelForm.value.name,
      enabled: addChannelForm.value.enabled,
      scope: wizardScope.value,
      ...configInputFor(addChannelForm.value.channel_type, {
        email: addChannelEmailCfg.value,
        webhook: addChannelWebhookCfg.value,
      }),
    }
    const created = await createChannel(req)
    channels.value.push(created)
    // Create rules for selected events
    const rulePromises = wizardEvents.value.map((et) =>
      createRule({ channel_id: created.id, event_type: et, enabled: true }),
    )
    const createdRules = await Promise.all(rulePromises)
    rules.value.push(...createdRules)
    showAddChannelDialog.value = false
  } catch (e: unknown) {
    addChannelError.value = extractError(e)
  } finally {
    addChannelLoading.value = false
  }
}

function wizardNextStep(): void {
  if (wizardStep.value < 3) wizardStep.value++
}

function wizardPrevStep(): void {
  if (wizardStep.value > 1) wizardStep.value--
}

function toggleWizardEvent(et: EventType): void {
  const idx = wizardEvents.value.indexOf(et)
  if (idx >= 0) {
    wizardEvents.value.splice(idx, 1)
  } else {
    wizardEvents.value.push(et)
  }
}

function toggleWizardScopeItem(type: keyof ChannelScope, id: number): void {
  const current = wizardScope.value[type] ?? []
  const updated = current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
  wizardScope.value = { ...wizardScope.value, [type]: updated }
}

useEscapeKey(showAddChannelDialog, () => {
  showAddChannelDialog.value = false
})

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
  loadScopeOptions().catch(logger.error)
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

    <!-- Add Channel Wizard -->
    <BaseModal
      :open="showAddChannelDialog"
      size="lg"
      title="New channel"
      @close="showAddChannelDialog = false"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          New channel
        </h2>
        <span class="wizard-step-indicator">Step {{ wizardStep }} of 3</span>
      </template>
      <!-- Step 1: Type & Config -->
      <template v-if="wizardStep === 1">
        <div class="field">
          <label class="field-label">Type <span class="required">*</span></label>
          <select
            v-model="addChannelForm.channel_type"
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
          <label class="field-label">Name <span class="required">*</span></label>
          <input
            v-model="addChannelForm.name"
            class="input"
            placeholder="e.g. Ops Email"
          />
        </div>

        <!--
          The config drafts are bound one-way on purpose: ChannelConfigFields
          edits their fields in place and never replaces the object, so the
          update handler a v-model would install could never fire.
        -->
        <ChannelConfigFields
          ref="addConfigFields"
          v-model:to-addresses="toAddressesInput"
          v-model:webhook-headers="addHeaderRows"
          :email-config="addChannelEmailCfg"
          :webhook-config="addChannelWebhookCfg"
          :channel-type="addChannelForm.channel_type"
          show-required
        />

        <!-- Web Push hint -->
        <div
          v-if="addChannelForm.channel_type === 'web_push' && !vapidConfigured"
          class="form-hint-warning"
        >
          VAPID keys must be configured before creating a Web Push channel.
        </div>

        <div class="field">
          <ToggleSwitch
            :model-value="addChannelForm.enabled"
            @update:model-value="addChannelForm.enabled = $event"
          >
            Enable immediately
          </ToggleSwitch>
        </div>
      </template>

      <!-- Step 2: Events -->
      <template v-if="wizardStep === 2">
        <p class="step-description">Select which events should trigger this channel.</p>
        <NotificationEventToggles
          :event-types="EVENT_TYPES"
          :label-for="eventTypeLabel"
          :is-enabled="(et) => wizardEvents.includes(et)"
          @toggle="toggleWizardEvent"
        />
      </template>

      <!-- Step 3: Scope -->
      <template v-if="wizardStep === 3">
        <p class="step-description">
          Optionally restrict this channel to specific resources. Leave empty for all.
        </p>
        <ScopeSelector
          v-model:search="scopeSearch"
          :selected="wizardScope"
          :repos="scopeRepos"
          :agents="scopeAgents"
          :schedules="scopeSchedules"
          @toggle="toggleWizardScopeItem"
        />
      </template>

      <div
        v-if="addChannelError"
        class="form-error"
      >
        {{ addChannelError }}
      </div>

      <template #footer>
        <button
          v-if="wizardStep > 1"
          class="btn btn-ghost"
          @click="wizardPrevStep"
        >
          Back
        </button>
        <button
          v-else
          class="btn btn-ghost"
          @click="showAddChannelDialog = false"
        >
          Cancel
        </button>
        <button
          v-if="wizardStep < 3"
          class="btn btn-primary"
          :disabled="wizardStep === 1 && !addChannelFormValid"
          @click="wizardNextStep"
        >
          Next
        </button>
        <button
          v-else
          class="btn btn-primary"
          :disabled="addChannelLoading"
          @click="submitAddChannel"
        >
          {{ addChannelLoading ? 'Creating...' : 'Create' }}
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.notifications-view {
  max-width: 1100px;
}

/* Wizard */

.wizard-step-indicator {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-left: auto;
  margin-right: var(--space-5);
}

.step-description {
  font-size: var(--fs-base);
  color: var(--text-secondary);
  margin-bottom: var(--space-6);
}

/* Form */

.form-hint-warning {
  background: color-mix(in srgb, var(--warning) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
  border-radius: var(--radius-sm);
  padding: var(--space-4) var(--space-5);
  font-size: var(--fs-base);
  color: var(--warning);
  margin-bottom: var(--space-6);
}
</style>

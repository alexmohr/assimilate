<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useWebSocket } from '../composables/useWebSocket'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
import { validateEmailConfig } from '../utils/smtpValidation'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import {
  listChannels,
  createChannel,
  updateChannel,
  deleteChannel,
  testChannel,
  listRules,
  createRule,
  deleteRule,
  getVapidPublicKey,
  subscribePush,
  listDeliveries,
} from '../api/notifications'
import { Plus, Trash2, Bell, Send, Mail, Globe, BellRing } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import ChannelConfigFields from '../components/ChannelConfigFields.vue'
import NotificationHistoryTab from '../components/NotificationHistoryTab.vue'
import type {
  NotificationChannel,
  CreateChannelRequest,
  UpdateChannelRequest,
  NotificationRule,
  NotificationEventType,
  ChannelType,
  ChannelConfig,
  EmailConfig,
  WebhookConfig,
  NotificationDelivery,
  ChannelScope,
} from '../types/notifications'
import type { Repo } from '../types/repo'
import { apiClient } from '../api/client'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

type TabId = 'channels' | 'history'

interface ScopeOption {
  id: number
  label: string
}

const tabs: TabOption<TabId>[] = [
  { id: 'channels', label: 'Channels', icon: Bell },
  { id: 'history', label: 'History', icon: Send },
]

const activeTab = ref<TabId>('channels')
const channels = ref<NotificationChannel[]>([])
const rules = ref<NotificationRule[]>([])
const deliveries = ref<NotificationDelivery[]>([])
const { loading, error, run } = useAsyncAction()
const scopeRepos = ref<ScopeOption[]>([])
const scopeAgents = ref<ScopeOption[]>([])
const scopeSchedules = ref<ScopeOption[]>([])

// Add channel wizard state
const addConfigFields = ref<InstanceType<typeof ChannelConfigFields> | null>(null)
const editConfigFields = ref<InstanceType<typeof ChannelConfigFields> | null>(null)
const showAddChannelDialog = ref(false)
const wizardStep = ref(1)
const addChannelForm = ref<CreateChannelRequest>({
  name: '',
  channel_type: 'email',
  config: createEmailConfig(),
  enabled: true,
})
const wizardEvents = ref<NotificationEventType[]>([])
const wizardScope = ref<ChannelScope>({})
const addChannelError = ref('')
const addChannelLoading = ref(false)
const toAddressesInput = ref('')

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

// Edit channel dialog state
const showEditChannelDialog = ref(false)
const editChannelId = ref<number | null>(null)
const editChannelForm = ref<UpdateChannelRequest>({})
const editChannelError = ref('')
const editChannelLoading = ref(false)
const editToAddressesInput = ref('')

// Delete channel dialog state
const showDeleteChannelDialog = ref(false)
const deleteChannelId = ref<number | null>(null)
const deleteChannelName = ref('')
const deleteChannelLoading = ref(false)
const deleteChannelError = ref('')

// Events edit modal state
const showEventsModal = ref(false)
const eventsModalChannelId = ref<number | null>(null)

// Scope edit modal state
const showScopeModal = ref(false)
const scopeModalChannelId = ref<number | null>(null)
const scopeSearch = ref('')

const ruleTogglingKey = ref<string | null>(null)

const testingChannelId = ref<number | null>(null)
const testResult = ref<{ id: number; success: boolean; message: string } | null>(null)

const currentPushSubscription = ref<PushSubscription | null>(null)
const vapidConfigured = ref(false)

const EVENT_TYPES: NotificationEventType[] = [
  'backup_success',
  'backup_warning',
  'backup_failed',
  'check_success',
  'check_failed',
  'agent_connected',
  'agent_disconnected',
]

const CHANNEL_TYPES: ChannelType[] = ['email', 'webhook', 'web_push']

function createEmailConfig(): EmailConfig {
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

function createWebhookConfig(): WebhookConfig {
  return { url: '', headers: {} }
}

function isEmailConfig(config: ChannelConfig): config is EmailConfig {
  return 'smtp_host' in config && 'smtp_port' in config
}
function isWebhookConfig(config: ChannelConfig): config is WebhookConfig {
  return 'url' in config
}

const addChannelEmailCfg = computed((): EmailConfig => {
  if (isEmailConfig(addChannelForm.value.config)) return addChannelForm.value.config
  return createEmailConfig()
})
const addChannelWebhookCfg = computed((): WebhookConfig => {
  if (isWebhookConfig(addChannelForm.value.config)) return addChannelForm.value.config
  return createWebhookConfig()
})
const editChannelEmailCfg = computed((): EmailConfig => {
  const config = editChannelForm.value.config
  if (config != null && isEmailConfig(config)) return config
  return createEmailConfig()
})
const editChannelWebhookCfg = computed((): WebhookConfig => {
  const config = editChannelForm.value.config
  if (config != null && isWebhookConfig(config)) return config
  return createWebhookConfig()
})

const activeEventsChannel = computed((): NotificationChannel | undefined => {
  if (eventsModalChannelId.value == null) return undefined
  return channels.value.find((c) => c.id === eventsModalChannelId.value)
})

const activeScopeChannel = computed((): NotificationChannel | undefined => {
  if (scopeModalChannelId.value == null) return undefined
  return channels.value.find((c) => c.id === scopeModalChannelId.value)
})

function eventTypeLabel(et: NotificationEventType): string {
  const words = et.split('_')
  return [words[0].charAt(0).toUpperCase() + words[0].slice(1), ...words.slice(1)].join(' ')
}

function channelTypeLabel(ct: ChannelType): string {
  if (ct === 'email') return 'Email'
  if (ct === 'webhook') return 'Webhook'
  return 'Web push'
}

function channelTypeIcon(ct: ChannelType): typeof Mail {
  if (ct === 'email') return Mail
  if (ct === 'webhook') return Globe
  return BellRing
}

function channelEventsLabel(channelId: number): string {
  const count = rules.value.filter((r) => r.channel_id === channelId).length
  if (count === 0) return 'None'
  return `${count} of ${EVENT_TYPES.length} enabled`
}

function channelScopeLabel(channel: NotificationChannel): string {
  const s = channel.scope
  if (!s) return 'All'
  const parts: string[] = []
  if (s.repo_ids && s.repo_ids.length > 0) {
    parts.push(`${s.repo_ids.length} repo${s.repo_ids.length > 1 ? 's' : ''}`)
  }
  if (s.agent_ids && s.agent_ids.length > 0) {
    parts.push(`${s.agent_ids.length} host${s.agent_ids.length > 1 ? 's' : ''}`)
  }
  if (s.schedule_ids && s.schedule_ids.length > 0) {
    parts.push(`${s.schedule_ids.length} schedule${s.schedule_ids.length > 1 ? 's' : ''}`)
  }
  return parts.length > 0 ? parts.join(', ') : 'All'
}

function filteredScopeOptions(options: ScopeOption[]): ScopeOption[] {
  const q = scopeSearch.value.toLowerCase().trim()
  if (!q) return options
  return options.filter((o) => o.label.toLowerCase().includes(q))
}

function isScopeSelected(
  channel: NotificationChannel,
  type: keyof ChannelScope,
  id: number,
): boolean {
  const arr = channel.scope?.[type]
  return Array.isArray(arr) && arr.includes(id)
}

async function toggleScopeItem(
  channel: NotificationChannel,
  type: keyof ChannelScope,
  id: number,
): Promise<void> {
  const current = channel.scope?.[type] ?? []
  const updated = current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
  const newScope: ChannelScope = { ...channel.scope, [type]: updated }
  try {
    const result = await updateChannel(channel.id, { scope: newScope })
    const idx = channels.value.findIndex((c) => c.id === channel.id)
    if (idx >= 0) channels.value[idx] = result
  } catch (e: unknown) {
    logger.error('toggleScopeItem failed', e)
  }
}

const isPushSupported = computed((): boolean => {
  return 'serviceWorker' in navigator && 'PushManager' in window
})

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
    if (isPushSupported.value && vapidStatus.configured) {
      const registration = await navigator.serviceWorker.ready
      const existing = await registration.pushManager.getSubscription()
      currentPushSubscription.value = existing
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
    const [reposRes, agentsRes, schedulesRes] = await Promise.all([
      apiClient.get<Repo[]>('/repos'),
      apiClient.get<{ id: number; hostname: string; display_name: string | null }[]>('/agents'),
      apiClient.get<{ id: number; agent_id: number; repo_id: number | null }[]>('/schedules'),
    ])
    scopeRepos.value = reposRes.data.map((r) => ({ id: r.id, label: r.name }))
    scopeAgents.value = agentsRes.data.map((c) => ({
      id: c.id,
      label: c.display_name ?? c.hostname,
    }))
    scopeSchedules.value = schedulesRes.data.map((s) => ({
      id: s.id,
      label: `Schedule #${s.id}`,
    }))
  } catch (e: unknown) {
    logger.error('loadScopeOptions failed', e)
  }
}

function resetAddChannelConfig(): void {
  const ct = addChannelForm.value.channel_type
  if (ct === 'email') {
    addChannelForm.value.config = createEmailConfig()
    toAddressesInput.value = ''
  } else if (ct === 'webhook') {
    addChannelForm.value.config = createWebhookConfig()
  } else {
    addChannelForm.value.config = {}
  }
}

function openAddChannel(): void {
  addChannelForm.value = {
    name: '',
    channel_type: 'email',
    config: createEmailConfig(),
    enabled: true,
  }
  toAddressesInput.value = ''
  addChannelError.value = ''
  wizardStep.value = 1
  wizardEvents.value = []
  wizardScope.value = {}
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
      ...addChannelForm.value,
      scope: wizardScope.value,
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

function toggleWizardEvent(et: NotificationEventType): void {
  const idx = wizardEvents.value.indexOf(et)
  if (idx >= 0) {
    wizardEvents.value.splice(idx, 1)
  } else {
    wizardEvents.value.push(et)
  }
}

function isWizardScopeSelected(type: keyof ChannelScope, id: number): boolean {
  const arr = wizardScope.value[type]
  return Array.isArray(arr) && arr.includes(id)
}

function toggleWizardScopeItem(type: keyof ChannelScope, id: number): void {
  const current = wizardScope.value[type] ?? []
  const updated = current.includes(id) ? current.filter((x) => x !== id) : [...current, id]
  wizardScope.value = { ...wizardScope.value, [type]: updated }
}

function openEditChannel(channel: NotificationChannel): void {
  editChannelId.value = channel.id
  editChannelForm.value = {
    name: channel.name,
    config: { ...channel.config },
    enabled: channel.enabled,
  }
  if (channel.channel_type === 'email' && 'smtp_host' in channel.config) {
    editToAddressesInput.value = channel.config.to_addresses.join(', ')
  }
  editChannelError.value = ''
  editConfigFields.value?.reset()
  showEditChannelDialog.value = true
}

function editChannelType(): ChannelType {
  const ch = channels.value.find((c) => c.id === editChannelId.value)
  return ch?.channel_type ?? 'email'
}

async function submitEditChannel(): Promise<void> {
  if (editChannelId.value === null) return
  if (editChannelType() === 'email' && editChannelForm.value.config) {
    editChannelEmailCfg.value.to_addresses = editToAddressesInput.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  editChannelLoading.value = true
  editChannelError.value = ''
  try {
    if (editChannelType() === 'email' && editChannelForm.value.config) {
      const verdict = await validateEmailConfig(editChannelEmailCfg.value)
      if (!verdict.success) {
        editChannelError.value = verdict.message
        return
      }
    }
    const updated = await updateChannel(editChannelId.value, editChannelForm.value)
    const idx = channels.value.findIndex((c) => c.id === editChannelId.value)
    if (idx !== -1) {
      channels.value[idx] = updated
    }
    showEditChannelDialog.value = false
  } catch (e: unknown) {
    editChannelError.value = extractError(e)
  } finally {
    editChannelLoading.value = false
  }
}

function openDeleteChannel(channel: NotificationChannel): void {
  deleteChannelId.value = channel.id
  deleteChannelName.value = channel.name
  deleteChannelError.value = ''
  showDeleteChannelDialog.value = true
}

async function confirmDeleteChannel(): Promise<void> {
  if (deleteChannelId.value === null) return
  deleteChannelLoading.value = true
  deleteChannelError.value = ''
  try {
    await deleteChannel(deleteChannelId.value)
    channels.value = channels.value.filter((c) => c.id !== deleteChannelId.value)
    rules.value = rules.value.filter((r) => r.channel_id !== deleteChannelId.value)
    showDeleteChannelDialog.value = false
  } catch (e: unknown) {
    deleteChannelError.value = extractError(e)
  } finally {
    deleteChannelLoading.value = false
  }
}

async function toggleChannel(channel: NotificationChannel): Promise<void> {
  try {
    const updated = await updateChannel(channel.id, { enabled: !channel.enabled })
    const idx = channels.value.findIndex((c) => c.id === channel.id)
    if (idx !== -1) {
      channels.value[idx] = updated
    }
  } catch (e: unknown) {
    logger.error('toggleChannel failed', e)
  }
}

async function handleTestChannel(id: number): Promise<void> {
  testingChannelId.value = id
  testResult.value = null
  try {
    const channel = channels.value.find((c) => c.id === id)
    if (channel?.channel_type === 'web_push') {
      await ensurePushSubscription()
    }
    await testChannel(id)
    testResult.value = { id, success: true, message: 'Test sent' }
  } catch (e: unknown) {
    testResult.value = { id, success: false, message: extractError(e) }
  } finally {
    testingChannelId.value = null
  }
}

function isEventEnabled(channelId: number, et: NotificationEventType): boolean {
  return rules.value.some((r) => r.channel_id === channelId && r.event_type === et)
}

function isRuleToggling(channelId: number, et: NotificationEventType): boolean {
  return ruleTogglingKey.value === `${channelId}:${et}`
}

async function toggleRule(channelId: number, et: NotificationEventType): Promise<void> {
  const key = `${channelId}:${et}`
  ruleTogglingKey.value = key
  const existing = rules.value.find((r) => r.channel_id === channelId && r.event_type === et)
  try {
    if (existing) {
      await deleteRule(existing.id)
      rules.value = rules.value.filter((r) => r.id !== existing.id)
    } else {
      const created = await createRule({
        channel_id: channelId,
        event_type: et,
        enabled: true,
      })
      rules.value.push(created)
    }
  } catch (e: unknown) {
    logger.error('toggleRule failed', e)
  } finally {
    ruleTogglingKey.value = null
  }
}

function openEventsModal(channelId: number): void {
  eventsModalChannelId.value = channelId
  showEventsModal.value = true
}

function openScopeModal(channelId: number): void {
  scopeModalChannelId.value = channelId
  scopeSearch.value = ''
  showScopeModal.value = true
}

function urlBase64ToUint8Array(base64String: string): Uint8Array<ArrayBuffer> {
  const padding = '='.repeat((4 - (base64String.length % 4)) % 4)
  const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/')
  const rawData = window.atob(base64)
  const outputArray = new Uint8Array(rawData.length)
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i)
  }
  return outputArray
}

async function ensurePushSubscription(): Promise<void> {
  if (!isPushSupported.value) {
    throw new Error('Push notifications are not supported in this browser')
  }

  if (Notification.permission === 'denied') {
    throw new Error('Notification permission was denied. Please enable it in browser settings.')
  }

  if (Notification.permission !== 'granted') {
    let result = await Notification.requestPermission()
    if (result === 'default') {
      result = await Notification.requestPermission()
    }
    if (result !== 'granted') {
      throw new Error('Notification permission is required for web push')
    }
  }

  const vapidStatus = await getVapidPublicKey()
  if (!vapidStatus.configured) {
    throw new Error('VAPID keys not configured on the server')
  }

  const registration = await navigator.serviceWorker.ready

  const existing = await registration.pushManager.getSubscription()
  if (existing) {
    await existing.unsubscribe()
  }

  const subscription = await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: urlBase64ToUint8Array(vapidStatus.key),
  })
  await subscribePush(subscription.toJSON())
  currentPushSubscription.value = subscription
}

useEscapeKey(showAddChannelDialog, () => {
  showAddChannelDialog.value = false
})
useEscapeKey(showEditChannelDialog, () => {
  showEditChannelDialog.value = false
})
useEscapeKey(showDeleteChannelDialog, () => {
  showDeleteChannelDialog.value = false
})
useEscapeKey(showEventsModal, () => {
  showEventsModal.value = false
})
useEscapeKey(showScopeModal, () => {
  showScopeModal.value = false
})

const { onMessage } = useWebSocket()
onMessage('NotificationDelivery', (data: NotificationDelivery) => {
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
      <div
        v-else
        class="channels-list"
      >
        <div
          v-for="channel in channels"
          :key="channel.id"
          class="channel-card"
        >
          <div class="channel-header">
            <div class="channel-info">
              <component
                :is="channelTypeIcon(channel.channel_type)"
                :size="16"
                class="channel-icon"
              />
              <span class="channel-name">{{ channel.name }}</span>
              <span class="badge badge--neutral">{{ channelTypeLabel(channel.channel_type) }}</span>
            </div>
            <div class="channel-actions">
              <button
                class="btn btn-sm btn-ghost"
                :disabled="testingChannelId === channel.id"
                @click="handleTestChannel(channel.id)"
              >
                {{ testingChannelId === channel.id ? 'Testing...' : 'Test' }}
              </button>
              <ToggleSwitch
                :model-value="channel.enabled"
                @update:model-value="toggleChannel(channel)"
              >
                {{ channel.enabled ? 'On' : 'Off' }}
              </ToggleSwitch>
              <button
                class="btn btn-sm btn-ghost"
                @click="openEditChannel(channel)"
              >
                Edit
              </button>
              <button
                class="btn btn-sm btn-ghost btn-danger-text"
                @click="openDeleteChannel(channel)"
              >
                <Trash2 :size="14" />
              </button>
            </div>
          </div>
          <div
            v-if="testResult && testResult.id === channel.id"
            class="test-result"
            :class="testResult.success ? 'test-success' : 'test-failure'"
          >
            {{ testResult.message }}
          </div>
          <div class="channel-meta">
            <div class="meta-row">
              <span class="meta-label">Events:</span>
              <span class="meta-value">{{ channelEventsLabel(channel.id) }}</span>
              <button
                class="meta-edit-btn"
                title="Edit events"
                @click="openEventsModal(channel.id)"
              >
                ✎
              </button>
            </div>
            <div class="meta-row">
              <span class="meta-label">Scope:</span>
              <span class="meta-value">{{ channelScopeLabel(channel) }}</span>
              <button
                class="meta-edit-btn"
                title="Edit scope"
                @click="openScopeModal(channel.id)"
              >
                ✎
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- History Tab -->
    <div
      v-if="activeTab === 'history'"
      class="tab-content fade-in"
    >
      <NotificationHistoryTab
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
            @change="resetAddChannelConfig"
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
          The config objects are bound one-way on purpose: ChannelConfigFields
          edits their fields in place and never replaces the object, so these
          are the form's own config, not a copy. Binding them with v-model
          would install an update handler that writes back to a read-only
          computed - dead in practice, and a silent no-op if it ever fired.
        -->
        <ChannelConfigFields
          ref="addConfigFields"
          v-model:to-addresses="toAddressesInput"
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
        <div class="events-list">
          <div
            v-for="et in EVENT_TYPES"
            :key="et"
            class="event-item"
          >
            <ToggleSwitch
              :model-value="wizardEvents.includes(et)"
              @update:model-value="toggleWizardEvent(et)"
            />
            <span class="event-label">{{ eventTypeLabel(et) }}</span>
          </div>
        </div>
      </template>

      <!-- Step 3: Scope -->
      <template v-if="wizardStep === 3">
        <p class="step-description">
          Optionally restrict this channel to specific resources. Leave empty for all.
        </p>
        <input
          v-model="scopeSearch"
          class="input scope-search"
          type="text"
          placeholder="Search..."
        />
        <div class="scope-sections">
          <div
            v-if="scopeRepos.length > 0"
            class="scope-section"
          >
            <span class="group-label group-label--lg scope-section-title">Repositories</span>
            <label
              v-for="opt in filteredScopeOptions(scopeRepos)"
              :key="'r' + opt.id"
              class="scope-item"
            >
              <input
                type="checkbox"
                :checked="isWizardScopeSelected('repo_ids', opt.id)"
                @change="toggleWizardScopeItem('repo_ids', opt.id)"
              />
              <span>{{ opt.label }}</span>
            </label>
          </div>
          <div
            v-if="scopeAgents.length > 0"
            class="scope-section"
          >
            <span class="group-label group-label--lg scope-section-title">Hosts</span>
            <label
              v-for="opt in filteredScopeOptions(scopeAgents)"
              :key="'c' + opt.id"
              class="scope-item"
            >
              <input
                type="checkbox"
                :checked="isWizardScopeSelected('agent_ids', opt.id)"
                @change="toggleWizardScopeItem('agent_ids', opt.id)"
              />
              <span>{{ opt.label }}</span>
            </label>
          </div>
          <div
            v-if="scopeSchedules.length > 0"
            class="scope-section"
          >
            <span class="group-label group-label--lg scope-section-title">Schedules</span>
            <label
              v-for="opt in filteredScopeOptions(scopeSchedules)"
              :key="'s' + opt.id"
              class="scope-item"
            >
              <input
                type="checkbox"
                :checked="isWizardScopeSelected('schedule_ids', opt.id)"
                @change="toggleWizardScopeItem('schedule_ids', opt.id)"
              />
              <span>{{ opt.label }}</span>
            </label>
          </div>
        </div>
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

    <!-- Edit Channel Dialog -->
    <BaseModal
      :open="showEditChannelDialog"
      title="Edit channel"
      @close="showEditChannelDialog = false"
    >
      <div class="field">
        <label class="field-label">Name</label>
        <input
          v-model="editChannelForm.name"
          class="input"
        />
      </div>

      <!-- One-way for the same reason as the add dialog above. -->
      <ChannelConfigFields
        v-if="editChannelForm.config"
        ref="editConfigFields"
        v-model:to-addresses="editToAddressesInput"
        :email-config="editChannelEmailCfg"
        :webhook-config="editChannelWebhookCfg"
        :channel-type="editChannelType()"
      />

      <div class="field">
        <ToggleSwitch
          :model-value="editChannelForm.enabled ?? false"
          @update:model-value="editChannelForm.enabled = $event"
        >
          Enabled
        </ToggleSwitch>
      </div>
      <div
        v-if="editChannelError"
        class="form-error"
      >
        {{ editChannelError }}
      </div>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showEditChannelDialog = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="editChannelLoading"
          @click="submitEditChannel"
        >
          {{ editChannelLoading ? 'Saving...' : 'Save' }}
        </button>
      </template>
    </BaseModal>

    <!-- Delete Channel Dialog -->
    <BaseModal
      :open="showDeleteChannelDialog"
      title="Delete channel"
      size="sm"
      @close="showDeleteChannelDialog = false"
    >
      <p class="confirm-text">
        Delete channel <strong>{{ deleteChannelName }}</strong
        >? All associated rules will also be removed.
      </p>
      <div
        v-if="deleteChannelError"
        class="form-error"
      >
        {{ deleteChannelError }}
      </div>

      <template #footer>
        <button
          class="btn btn-ghost"
          @click="showDeleteChannelDialog = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-danger"
          :disabled="deleteChannelLoading"
          @click="confirmDeleteChannel"
        >
          {{ deleteChannelLoading ? 'Deleting...' : 'Delete' }}
        </button>
      </template>
    </BaseModal>

    <!-- Events Edit Modal -->
    <BaseModal
      v-if="activeEventsChannel"
      :open="showEventsModal"
      @close="showEventsModal = false"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Events — {{ activeEventsChannel.name }}
        </h2>
      </template>
      <p class="step-description">Toggle which events trigger notifications for this channel.</p>
      <div class="events-list">
        <div
          v-for="et in EVENT_TYPES"
          :key="et"
          class="event-item"
        >
          <ToggleSwitch
            :model-value="isEventEnabled(activeEventsChannel.id, et)"
            :disabled="isRuleToggling(activeEventsChannel.id, et)"
            @update:model-value="toggleRule(activeEventsChannel.id, et)"
          />
          <span class="event-label">{{ eventTypeLabel(et) }}</span>
        </div>
      </div>

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showEventsModal = false"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <!-- Scope Edit Modal -->
    <BaseModal
      v-if="activeScopeChannel"
      :open="showScopeModal"
      @close="showScopeModal = false"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Scope — {{ activeScopeChannel.name }}
        </h2>
      </template>
      <p class="step-description">
        Restrict this channel to specific resources. Leave empty for all.
      </p>
      <input
        v-model="scopeSearch"
        class="input scope-search"
        type="text"
        placeholder="Search..."
      />
      <div class="scope-sections">
        <div
          v-if="scopeRepos.length > 0"
          class="scope-section"
        >
          <span class="group-label group-label--lg scope-section-title">Repositories</span>
          <label
            v-for="opt in filteredScopeOptions(scopeRepos)"
            :key="'r' + opt.id"
            class="scope-item"
          >
            <input
              type="checkbox"
              :checked="isScopeSelected(activeScopeChannel, 'repo_ids', opt.id)"
              @change="toggleScopeItem(activeScopeChannel, 'repo_ids', opt.id)"
            />
            <span>{{ opt.label }}</span>
          </label>
        </div>
        <div
          v-if="scopeAgents.length > 0"
          class="scope-section"
        >
          <span class="group-label group-label--lg scope-section-title">Hosts</span>
          <label
            v-for="opt in filteredScopeOptions(scopeAgents)"
            :key="'c' + opt.id"
            class="scope-item"
          >
            <input
              type="checkbox"
              :checked="isScopeSelected(activeScopeChannel, 'agent_ids', opt.id)"
              @change="toggleScopeItem(activeScopeChannel, 'agent_ids', opt.id)"
            />
            <span>{{ opt.label }}</span>
          </label>
        </div>
        <div
          v-if="scopeSchedules.length > 0"
          class="scope-section"
        >
          <span class="group-label group-label--lg scope-section-title">Schedules</span>
          <label
            v-for="opt in filteredScopeOptions(scopeSchedules)"
            :key="'s' + opt.id"
            class="scope-item"
          >
            <input
              type="checkbox"
              :checked="isScopeSelected(activeScopeChannel, 'schedule_ids', opt.id)"
              @change="toggleScopeItem(activeScopeChannel, 'schedule_ids', opt.id)"
            />
            <span>{{ opt.label }}</span>
          </label>
        </div>
      </div>

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showScopeModal = false"
        >
          Done
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.notifications-view {
  max-width: 1100px;
}

.channels-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.channel-card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-6) var(--space-7);
  background: var(--bg-card);
}

.channel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-6);
}

.channel-info {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.channel-icon {
  color: var(--text-muted);
}

.channel-name {
  font-weight: 600;
  font-size: var(--fs-md);
}

.channel-actions {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.test-result {
  margin-top: var(--space-4);
  padding: var(--space-3) var(--space-5);
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
}

.channel-meta {
  margin-top: var(--space-5);
  padding-top: var(--space-5);
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.meta-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--fs-sm);
}

.meta-label {
  color: var(--text-muted);
  font-weight: 500;
}

.meta-value {
  color: var(--text-secondary);
}

.meta-edit-btn {
  background: none;
  border: none;
  padding: var(--space-1) var(--space-2);
  cursor: pointer;
  font-size: var(--fs-sm);
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  line-height: 1;
}

.meta-edit-btn:hover {
  background: var(--bg-elevated);
  color: var(--accent);
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

/* Events list (wizard + modal) */
.events-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.event-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-sm);
}

.event-item:hover {
  background: var(--bg-hover);
}

.event-label {
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

/* Scope sections (wizard + modal) */
.scope-search {
  margin-bottom: var(--space-5);
}

.scope-sections {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
  max-height: 320px;
  overflow-y: auto;
}

.scope-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

/* The shared group label plus the space this list wants under it. */
.scope-section-title {
  margin-bottom: var(--space-2);
}

.scope-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-2) var(--space-3);
  font-size: var(--fs-sm);
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.scope-item:hover {
  background: var(--bg-hover);
}

.scope-item input[type='checkbox'] {
  accent-color: var(--accent);
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

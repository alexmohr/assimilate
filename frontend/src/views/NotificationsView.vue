<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useWebSocket } from '../composables/useWebSocket'
import { useEscapeKey } from '../composables/useEscapeKey'
import { extractError } from '../utils/error'
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
  validateSmtp,
} from '../api/notifications'
import { Plus, Trash2, Bell, Send, Mail, Globe, BellRing } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type {
  NotificationChannel,
  CreateChannelRequest,
  UpdateChannelRequest,
  NotificationRule,
  NotificationEventType,
  ChannelType,
  EmailConfig,
  WebhookConfig,
  NotificationDelivery,
  ChannelScope,
} from '../types/notifications'
import { apiClient } from '../api/client'

type TabId = 'channels' | 'history'

interface ScopeOption {
  id: number
  label: string
}

const activeTab = ref<TabId>('channels')
const channels = ref<NotificationChannel[]>([])
const rules = ref<NotificationRule[]>([])
const deliveries = ref<NotificationDelivery[]>([])
const loading = ref(false)
const error = ref('')
const scopeRepos = ref<ScopeOption[]>([])
const scopeClients = ref<ScopeOption[]>([])
const scopeSchedules = ref<ScopeOption[]>([])

// Add channel wizard state
const showAddChannelDialog = ref(false)
const wizardStep = ref(1)
const addChannelForm = ref<CreateChannelRequest>({
  name: '',
  channel_type: 'email',
  config: {
    smtp_host: '',
    smtp_port: 587,
    smtp_user: '',
    smtp_password: '',
    from_address: '',
    to_addresses: [],
    security: 'starttls',
  } as EmailConfig,
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
    const cfg = form.config as EmailConfig
    if (!cfg.smtp_host.trim() || !cfg.from_address.trim() || !toAddressesInput.value.trim())
      return false
  } else if (form.channel_type === 'webhook') {
    const cfg = form.config as WebhookConfig
    if (!cfg.url.trim()) return false
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

const smtpValidating = ref(false)
const smtpValidationResult = ref<{ success: boolean; message: string } | null>(null)

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

function eventTypeLabel(et: NotificationEventType): string {
  return et
    .split('_')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ')
}

function channelTypeLabel(ct: ChannelType): string {
  if (ct === 'email') return 'Email'
  if (ct === 'webhook') return 'Webhook'
  return 'Web Push'
}

function channelTypeIcon(ct: ChannelType): typeof Mail {
  if (ct === 'email') return Mail
  if (ct === 'webhook') return Globe
  return BellRing
}

function deliveryStatusClass(status: string): string {
  if (status === 'sent') return 'status-sent'
  if (status === 'failed') return 'status-failed'
  return 'status-pending'
}

function channelNameById(id: number): string {
  return channels.value.find((c) => c.id === id)?.name ?? String(id)
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
  if (s.client_ids && s.client_ids.length > 0) {
    parts.push(`${s.client_ids.length} host${s.client_ids.length > 1 ? 's' : ''}`)
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
  loading.value = true
  error.value = ''
  try {
    channels.value = await listChannels()
    rules.value = await listRules()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loading.value = false
  }
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
    const [reposRes, clientsRes, schedulesRes] = await Promise.all([
      apiClient.get<{ id: number; name: string }[]>('/repos'),
      apiClient.get<{ id: number; hostname: string; display_name: string | null }[]>('/agents'),
      apiClient.get<{ id: number; agent_id: number; repo_id: number | null }[]>('/schedules'),
    ])
    scopeRepos.value = reposRes.data.map((r) => ({ id: r.id, label: r.name }))
    scopeClients.value = clientsRes.data.map((c) => ({
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
    addChannelForm.value.config = {
      smtp_host: '',
      smtp_port: 587,
      smtp_user: '',
      smtp_password: '',
      from_address: '',
      to_addresses: [],
      security: 'starttls',
    } as EmailConfig
    toAddressesInput.value = ''
  } else if (ct === 'webhook') {
    addChannelForm.value.config = { url: '', headers: {} } as WebhookConfig
  } else {
    addChannelForm.value.config = {}
  }
}

function openAddChannel(): void {
  addChannelForm.value = {
    name: '',
    channel_type: 'email',
    config: {
      smtp_host: '',
      smtp_port: 587,
      smtp_user: '',
      smtp_password: '',
      from_address: '',
      to_addresses: [],
      security: 'starttls',
    } as EmailConfig,
    enabled: true,
  }
  toAddressesInput.value = ''
  addChannelError.value = ''
  wizardStep.value = 1
  wizardEvents.value = []
  wizardScope.value = {}
  smtpValidationResult.value = null
  showAddChannelDialog.value = true
}

async function submitAddChannel(): Promise<void> {
  if (!addChannelForm.value.name.trim()) {
    addChannelError.value = 'Name is required'
    return
  }
  if (addChannelForm.value.channel_type === 'email') {
    const cfg = addChannelForm.value.config as EmailConfig
    cfg.to_addresses = toAddressesInput.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  addChannelLoading.value = true
  addChannelError.value = ''
  try {
    if (addChannelForm.value.channel_type === 'email') {
      const cfg = addChannelForm.value.config as EmailConfig
      const valid = await validateSmtpCredentials(cfg)
      if (!valid) {
        addChannelError.value = smtpValidationResult.value?.message ?? 'SMTP validation failed'
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
  if (channel.channel_type === 'email') {
    editToAddressesInput.value = (channel.config as EmailConfig).to_addresses.join(', ')
  }
  editChannelError.value = ''
  smtpValidationResult.value = null
  showEditChannelDialog.value = true
}

function editChannelType(): ChannelType {
  const ch = channels.value.find((c) => c.id === editChannelId.value)
  return ch?.channel_type ?? 'email'
}

async function submitEditChannel(): Promise<void> {
  if (editChannelId.value === null) return
  if (editChannelType() === 'email' && editChannelForm.value.config) {
    const cfg = editChannelForm.value.config as EmailConfig
    cfg.to_addresses = editToAddressesInput.value
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
  }
  editChannelLoading.value = true
  editChannelError.value = ''
  try {
    if (editChannelType() === 'email' && editChannelForm.value.config) {
      const cfg = editChannelForm.value.config as EmailConfig
      const valid = await validateSmtpCredentials(cfg)
      if (!valid) {
        editChannelError.value = smtpValidationResult.value?.message ?? 'SMTP validation failed'
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

async function validateSmtpCredentials(cfg: EmailConfig): Promise<boolean> {
  smtpValidating.value = true
  smtpValidationResult.value = null
  try {
    await validateSmtp({
      smtp_host: cfg.smtp_host,
      smtp_port: cfg.smtp_port,
      smtp_user: cfg.smtp_user,
      smtp_password: cfg.smtp_password,
      security: cfg.security ?? 'starttls',
    })
    smtpValidationResult.value = { success: true, message: 'SMTP login successful' }
    return true
  } catch (e: unknown) {
    smtpValidationResult.value = { success: false, message: extractError(e) }
    return false
  } finally {
    smtpValidating.value = false
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

function eventsModalChannel(): NotificationChannel | undefined {
  return channels.value.find((c) => c.id === eventsModalChannelId.value)
}

function scopeModalChannel(): NotificationChannel | undefined {
  return channels.value.find((c) => c.id === scopeModalChannelId.value)
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

function formatDate(iso: string): string {
  return new Date(iso).toLocaleString()
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

    <div class="tabs">
      <button
        class="tab"
        :class="{ active: activeTab === 'channels' }"
        @click="activeTab = 'channels'"
      >
        <Bell :size="14" />
        Channels
      </button>

      <button
        class="tab"
        :class="{ active: activeTab === 'history' }"
        @click="activeTab = 'history'"
      >
        <Send :size="14" />
        History
      </button>
    </div>

    <!-- Channels Tab -->
    <div v-if="activeTab === 'channels'">
      <BaseSpinner
        v-if="loading"
        size="lg"
      />
      <div
        v-else-if="error"
        class="state-msg state-error"
      >
        {{ error }}
      </div>
      <EmptyState
        v-else-if="channels.length === 0"
        :icon="Bell"
        title="No notification channels"
        description="Create a channel to receive alerts."
        action="Add Channel"
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
              <span class="channel-type-badge">{{ channelTypeLabel(channel.channel_type) }}</span>
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
    <div v-if="activeTab === 'history'">
      <div
        v-if="deliveries.length === 0"
        class="state-msg"
      >
        No delivery history yet.
      </div>
      <div
        v-else
        class="table-wrapper"
      >
        <table class="data-table">
          <thead>
            <tr>
              <th>Channel</th>
              <th>Event</th>
              <th>Status</th>
              <th>Error</th>
              <th>Time</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="d in deliveries"
              :key="d.id"
            >
              <td>{{ channelNameById(d.channel_id) }}</td>
              <td>{{ eventTypeLabel(d.event_type as NotificationEventType) }}</td>
              <td>
                <span
                  class="delivery-status"
                  :class="deliveryStatusClass(d.status)"
                >
                  {{ d.status }}
                </span>
              </td>
              <td class="mono">{{ d.error_message ?? '—' }}</td>
              <td>{{ formatDate(d.attempted_at) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Add Channel Wizard -->
    <Teleport to="body">
      <div
        v-if="showAddChannelDialog"
        class="overlay"
        @click.self="showAddChannelDialog = false"
      >
        <div class="dialog dialog-wizard">
          <div class="dialog-header">
            <h2 class="dialog-title">New Channel</h2>
            <span class="wizard-step-indicator">Step {{ wizardStep }} of 3</span>
            <button
              class="close-btn"
              @click="showAddChannelDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
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

              <!-- Email Config -->
              <template v-if="addChannelForm.channel_type === 'email'">
                <div class="field">
                  <label class="field-label">SMTP Host <span class="required">*</span></label>
                  <input
                    v-model="(addChannelForm.config as EmailConfig).smtp_host"
                    class="input mono"
                    placeholder="smtp.example.com"
                  />
                </div>
                <div class="field-row">
                  <div class="field">
                    <label class="field-label">SMTP User</label>
                    <input
                      v-model="(addChannelForm.config as EmailConfig).smtp_user"
                      class="input"
                    />
                  </div>
                  <div class="field field-narrow">
                    <label class="field-label">Port</label>
                    <input
                      v-model.number="(addChannelForm.config as EmailConfig).smtp_port"
                      class="input"
                      type="number"
                    />
                  </div>
                </div>
                <div class="field">
                  <label class="field-label">SMTP Password</label>
                  <input
                    v-model="(addChannelForm.config as EmailConfig).smtp_password"
                    class="input"
                    type="password"
                  />
                </div>
                <div class="field">
                  <label class="field-label">From Address <span class="required">*</span></label>
                  <input
                    v-model="(addChannelForm.config as EmailConfig).from_address"
                    class="input"
                    placeholder="noreply@example.com"
                  />
                </div>
                <div class="field">
                  <label class="field-label">To Addresses <span class="required">*</span></label>
                  <input
                    v-model="toAddressesInput"
                    class="input"
                    placeholder="admin@example.com, ops@example.com"
                  />
                  <span class="field-hint">Comma-separated email addresses</span>
                </div>
                <div class="field">
                  <label class="field-label">Security</label>
                  <select
                    v-model="(addChannelForm.config as EmailConfig).security"
                    class="input"
                  >
                    <option value="starttls">STARTTLS (port 587)</option>
                    <option value="tls">SSL/TLS (port 465)</option>
                    <option value="none">None (insecure)</option>
                  </select>
                </div>
                <div class="field">
                  <button
                    class="btn btn-sm btn-ghost"
                    :disabled="smtpValidating"
                    @click="validateSmtpCredentials(addChannelForm.config as EmailConfig)"
                  >
                    {{ smtpValidating ? 'Testing...' : 'Test Connection' }}
                  </button>
                  <span
                    v-if="smtpValidationResult"
                    class="smtp-validation-result"
                    :class="smtpValidationResult.success ? 'test-success' : 'test-failure'"
                  >
                    {{ smtpValidationResult.message }}
                  </span>
                </div>
              </template>

              <!-- Webhook Config -->
              <template v-if="addChannelForm.channel_type === 'webhook'">
                <div class="field">
                  <label class="field-label">URL <span class="required">*</span></label>
                  <input
                    v-model="(addChannelForm.config as WebhookConfig).url"
                    class="input mono"
                    placeholder="https://hooks.example.com/notify"
                  />
                </div>
              </template>

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
                  <span class="scope-section-title">Repositories</span>
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
                  v-if="scopeClients.length > 0"
                  class="scope-section"
                >
                  <span class="scope-section-title">Hosts</span>
                  <label
                    v-for="opt in filteredScopeOptions(scopeClients)"
                    :key="'c' + opt.id"
                    class="scope-item"
                  >
                    <input
                      type="checkbox"
                      :checked="isWizardScopeSelected('client_ids', opt.id)"
                      @change="toggleWizardScopeItem('client_ids', opt.id)"
                    />
                    <span>{{ opt.label }}</span>
                  </label>
                </div>
                <div
                  v-if="scopeSchedules.length > 0"
                  class="scope-section"
                >
                  <span class="scope-section-title">Schedules</span>
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
          </div>
          <div class="dialog-footer">
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
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Edit Channel Dialog -->
    <Teleport to="body">
      <div
        v-if="showEditChannelDialog"
        class="overlay"
        @click.self="showEditChannelDialog = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Edit Channel</h2>
            <button
              class="close-btn"
              @click="showEditChannelDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <div class="field">
              <label class="field-label">Name</label>
              <input
                v-model="editChannelForm.name"
                class="input"
              />
            </div>

            <!-- Email Config Edit -->
            <template v-if="editChannelType() === 'email' && editChannelForm.config">
              <div class="field">
                <label class="field-label">SMTP Host</label>
                <input
                  v-model="(editChannelForm.config as EmailConfig).smtp_host"
                  class="input mono"
                />
              </div>
              <div class="field-row">
                <div class="field">
                  <label class="field-label">SMTP User</label>
                  <input
                    v-model="(editChannelForm.config as EmailConfig).smtp_user"
                    class="input"
                  />
                </div>
                <div class="field field-narrow">
                  <label class="field-label">Port</label>
                  <input
                    v-model.number="(editChannelForm.config as EmailConfig).smtp_port"
                    class="input"
                    type="number"
                  />
                </div>
              </div>
              <div class="field">
                <label class="field-label">SMTP Password</label>
                <input
                  v-model="(editChannelForm.config as EmailConfig).smtp_password"
                  class="input"
                  type="password"
                />
              </div>
              <div class="field">
                <label class="field-label">From Address</label>
                <input
                  v-model="(editChannelForm.config as EmailConfig).from_address"
                  class="input"
                />
              </div>
              <div class="field">
                <label class="field-label">To Addresses</label>
                <input
                  v-model="editToAddressesInput"
                  class="input"
                  placeholder="admin@example.com, ops@example.com"
                />
                <span class="field-hint">Comma-separated email addresses</span>
              </div>
              <div class="field">
                <label class="field-label">Security</label>
                <select
                  v-model="(editChannelForm.config as EmailConfig).security"
                  class="input"
                >
                  <option value="starttls">STARTTLS (port 587)</option>
                  <option value="tls">SSL/TLS (port 465)</option>
                  <option value="none">None (insecure)</option>
                </select>
              </div>
              <div class="field">
                <button
                  class="btn btn-sm btn-ghost"
                  :disabled="smtpValidating"
                  @click="validateSmtpCredentials(editChannelForm.config as EmailConfig)"
                >
                  {{ smtpValidating ? 'Testing...' : 'Test Connection' }}
                </button>
                <span
                  v-if="smtpValidationResult"
                  class="smtp-validation-result"
                  :class="smtpValidationResult.success ? 'test-success' : 'test-failure'"
                >
                  {{ smtpValidationResult.message }}
                </span>
              </div>
            </template>

            <!-- Webhook Config Edit -->
            <template v-if="editChannelType() === 'webhook' && editChannelForm.config">
              <div class="field">
                <label class="field-label">URL</label>
                <input
                  v-model="(editChannelForm.config as WebhookConfig).url"
                  class="input mono"
                />
              </div>
            </template>

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
          </div>
          <div class="dialog-footer">
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
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Delete Channel Dialog -->
    <Teleport to="body">
      <div
        v-if="showDeleteChannelDialog"
        class="overlay"
        @click.self="showDeleteChannelDialog = false"
      >
        <div class="dialog dialog-sm">
          <div class="dialog-header">
            <h2 class="dialog-title">Delete Channel</h2>
            <button
              class="close-btn"
              @click="showDeleteChannelDialog = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
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
          </div>
          <div class="dialog-footer">
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
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Events Edit Modal -->
    <Teleport to="body">
      <div
        v-if="showEventsModal && eventsModalChannel()"
        class="overlay"
        @click.self="showEventsModal = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Events — {{ eventsModalChannel()!.name }}</h2>
            <button
              class="close-btn"
              @click="showEventsModal = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
            <p class="step-description">
              Toggle which events trigger notifications for this channel.
            </p>
            <div class="events-list">
              <div
                v-for="et in EVENT_TYPES"
                :key="et"
                class="event-item"
              >
                <ToggleSwitch
                  :model-value="isEventEnabled(eventsModalChannelId!, et)"
                  :disabled="isRuleToggling(eventsModalChannelId!, et)"
                  @update:model-value="toggleRule(eventsModalChannelId!, et)"
                />
                <span class="event-label">{{ eventTypeLabel(et) }}</span>
              </div>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showEventsModal = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Scope Edit Modal -->
    <Teleport to="body">
      <div
        v-if="showScopeModal && scopeModalChannel()"
        class="overlay"
        @click.self="showScopeModal = false"
      >
        <div class="dialog">
          <div class="dialog-header">
            <h2 class="dialog-title">Scope — {{ scopeModalChannel()!.name }}</h2>
            <button
              class="close-btn"
              @click="showScopeModal = false"
            >
              &times;
            </button>
          </div>
          <div class="dialog-body">
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
                <span class="scope-section-title">Repositories</span>
                <label
                  v-for="opt in filteredScopeOptions(scopeRepos)"
                  :key="'r' + opt.id"
                  class="scope-item"
                >
                  <input
                    type="checkbox"
                    :checked="isScopeSelected(scopeModalChannel()!, 'repo_ids', opt.id)"
                    @change="toggleScopeItem(scopeModalChannel()!, 'repo_ids', opt.id)"
                  />
                  <span>{{ opt.label }}</span>
                </label>
              </div>
              <div
                v-if="scopeClients.length > 0"
                class="scope-section"
              >
                <span class="scope-section-title">Hosts</span>
                <label
                  v-for="opt in filteredScopeOptions(scopeClients)"
                  :key="'c' + opt.id"
                  class="scope-item"
                >
                  <input
                    type="checkbox"
                    :checked="isScopeSelected(scopeModalChannel()!, 'client_ids', opt.id)"
                    @change="toggleScopeItem(scopeModalChannel()!, 'client_ids', opt.id)"
                  />
                  <span>{{ opt.label }}</span>
                </label>
              </div>
              <div
                v-if="scopeSchedules.length > 0"
                class="scope-section"
              >
                <span class="scope-section-title">Schedules</span>
                <label
                  v-for="opt in filteredScopeOptions(scopeSchedules)"
                  :key="'s' + opt.id"
                  class="scope-item"
                >
                  <input
                    type="checkbox"
                    :checked="isScopeSelected(scopeModalChannel()!, 'schedule_ids', opt.id)"
                    @change="toggleScopeItem(scopeModalChannel()!, 'schedule_ids', opt.id)"
                  />
                  <span>{{ opt.label }}</span>
                </label>
              </div>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="showScopeModal = false"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.notifications-view {
  max-width: 1100px;
}

.tabs {
  display: flex;
  gap: 0;
  border-bottom: 1px solid var(--border);
  margin-bottom: 1.5rem;
}

.tab {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.75rem 1.25rem;
  border: none;
  background: none;
  font-size: 0.875rem;
  color: var(--text-muted);
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition:
    color 0.15s,
    border-color 0.15s;
}

.tab:hover {
  color: var(--text-secondary);
}

.tab.active {
  color: var(--text-secondary);
  border-bottom-color: currentColor;
  font-weight: 600;
}

.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
}

.channels-list {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.channel-card {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
  background: var(--bg-card);
}

.channel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
}

.channel-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.channel-icon {
  color: var(--text-muted);
}

.channel-name {
  font-weight: 600;
  font-size: 0.9rem;
}

.channel-type-badge {
  font-size: 0.72rem;
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.channel-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.test-result {
  margin-top: 0.5rem;
  padding: 0.4rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: 0.82rem;
}

.test-success {
  background: color-mix(in srgb, var(--success) 15%, transparent);
  color: var(--success);
}

.test-failure {
  background: var(--danger-subtle);
  color: var(--danger);
}

.smtp-validation-result {
  margin-left: 0.5rem;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  font-size: 0.8rem;
}

.channel-meta {
  margin-top: 0.75rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.meta-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.82rem;
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
  padding: 0.1rem 0.3rem;
  cursor: pointer;
  font-size: 0.82rem;
  color: var(--text-muted);
  border-radius: var(--radius-sm);
  line-height: 1;
}

.meta-edit-btn:hover {
  background: var(--surface-raised);
  color: var(--primary);
}

.table-wrapper {
  overflow-x: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.data-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
}

.data-table th {
  padding: 0.75rem 1rem;
  text-align: left;
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  background: var(--bg-card);
  border-bottom: 1px solid var(--border);
  white-space: nowrap;
}

.data-table td {
  padding: 0.75rem 1rem;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
  vertical-align: middle;
}

.data-table tbody tr:last-child td {
  border-bottom: none;
}

.data-table tbody tr:hover td {
  background: var(--bg-hover);
}

.delivery-status {
  font-size: 0.78rem;
  padding: 0.15rem 0.5rem;
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

/* Wizard */
.dialog-wizard {
  max-width: 520px;
}

.wizard-step-indicator {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin-left: auto;
  margin-right: 0.75rem;
}

.step-description {
  font-size: 0.85rem;
  color: var(--text-secondary);
  margin-bottom: 1rem;
}

/* Events list (wizard + modal) */
.events-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.event-item {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.4rem 0.5rem;
  border-radius: var(--radius-sm);
}

.event-item:hover {
  background: var(--bg-hover);
}

.event-label {
  font-size: 0.85rem;
  color: var(--text-secondary);
}

/* Scope sections (wizard + modal) */
.scope-search {
  margin-bottom: 0.75rem;
}

.scope-sections {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  max-height: 320px;
  overflow-y: auto;
}

.scope-section {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.scope-section-title {
  font-size: 0.72rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  color: var(--text-muted);
  margin-bottom: 0.2rem;
}

.scope-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.3rem 0.4rem;
  font-size: 0.83rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.scope-item:hover {
  background: var(--bg-hover);
}

.scope-item input[type='checkbox'] {
  accent-color: var(--primary);
}

/* Form */
.field-row {
  display: flex;
  gap: 1rem;
  margin-bottom: 1rem;
}

.field-row .field {
  flex: 1;
  margin-bottom: 0;
}

.field-narrow {
  max-width: 120px;
  flex: 0 0 120px !important;
}

.form-hint-warning {
  background: color-mix(in srgb, var(--warning) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
  border-radius: var(--radius-sm);
  padding: 0.6rem 0.8rem;
  font-size: 0.85rem;
  color: var(--warning);
  margin-bottom: 1rem;
}
</style>

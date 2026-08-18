<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { validateEmailConfig } from '../utils/smtpValidation'
import type { ChannelType, EmailConfig, WebhookConfig } from '../types/notifications'

/**
 * The transport-specific fields of a notification channel. The add wizard and
 * the edit dialog carried two near-identical copies of this markup, differing
 * only in whether the labels are marked required.
 */
defineProps<{
  channelType: ChannelType
  /** Marks the mandatory labels, which the create flow wants and edit does not. */
  showRequired?: boolean
}>()

/**
 * Bound two-way because the fields below edit the caller's config object in
 * place - the parent holds the request payload these become.
 */
const emailConfig = defineModel<EmailConfig>('emailConfig', { required: true })
const webhookConfig = defineModel<WebhookConfig>('webhookConfig', { required: true })

/** Comma-separated recipients, parsed back into `to_addresses` on submit. */
const toAddresses = defineModel<string>('toAddresses', { required: true })

const validating = ref(false)
const result = ref<{ success: boolean; message: string } | null>(null)

/**
 * Checks the SMTP credentials on demand, surfacing the verdict inline.
 *
 * The owning dialog does NOT gate its save on this - it calls
 * validateEmailConfig directly, because this component is unmounted on the
 * later steps of the add wizard and a ref to it would be null exactly when
 * the save happens.
 */
async function validate(): Promise<boolean> {
  validating.value = true
  result.value = null
  try {
    result.value = await validateEmailConfig(emailConfig.value)
    return result.value.success
  } finally {
    validating.value = false
  }
}

/** Cleared when the dialog reopens, so a stale verdict is never shown. */
function reset(): void {
  result.value = null
}

defineExpose({ validate, reset, result })
</script>

<template>
  <template v-if="channelType === 'email'">
    <div class="field">
      <label class="field-label">
        SMTP Host
        <span
          v-if="showRequired"
          class="required"
          >*</span
        >
      </label>
      <input
        v-model="emailConfig.smtp_host"
        class="input mono"
        placeholder="smtp.example.com"
      />
    </div>
    <div class="field-row">
      <div class="field">
        <label class="field-label">SMTP User</label>
        <input
          v-model="emailConfig.smtp_user"
          class="input"
        />
      </div>
      <div class="field field-narrow">
        <label class="field-label">Port</label>
        <input
          v-model.number="emailConfig.smtp_port"
          class="input"
          type="number"
        />
      </div>
    </div>
    <div class="field">
      <label class="field-label">SMTP Password</label>
      <input
        v-model="emailConfig.smtp_password"
        class="input"
        type="password"
      />
    </div>
    <div class="field">
      <label class="field-label">
        From Address
        <span
          v-if="showRequired"
          class="required"
          >*</span
        >
      </label>
      <input
        v-model="emailConfig.from_address"
        class="input"
        placeholder="noreply@example.com"
      />
    </div>
    <div class="field">
      <label class="field-label">
        To Addresses
        <span
          v-if="showRequired"
          class="required"
          >*</span
        >
      </label>
      <input
        v-model="toAddresses"
        class="input"
        placeholder="admin@example.com, ops@example.com"
      />
      <span class="field-hint">Comma-separated email addresses</span>
    </div>
    <div class="field">
      <label class="field-label">Security</label>
      <select
        v-model="emailConfig.security"
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
        :disabled="validating"
        @click="validate"
      >
        {{ validating ? 'Testing...' : 'Test Connection' }}
      </button>
      <span
        v-if="result"
        class="smtp-validation-result"
        :class="result.success ? 'test-success' : 'test-failure'"
      >
        {{ result.message }}
      </span>
    </div>
  </template>

  <template v-if="channelType === 'webhook'">
    <div class="field">
      <label class="field-label">
        URL
        <span
          v-if="showRequired"
          class="required"
          >*</span
        >
      </label>
      <input
        v-model="webhookConfig.url"
        class="input mono"
        placeholder="https://hooks.example.com/notify"
      />
    </div>
  </template>
</template>

<style scoped>
.smtp-validation-result {
  margin-left: 0.5rem;
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-sm);
  font-size: var(--fs-sm);
}
</style>

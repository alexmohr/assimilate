<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The System page's Prometheus panel: whether `/metrics` is served, and which
 * networks may scrape it without a monitoring token. Loads and saves its own
 * settings, so a failure here leaves the rest of the page alone.
 */
import { computed, onMounted, reactive, ref } from 'vue'
import { getMetricsSettings, updateMetricsSettings } from '../api/system'
import { useTimeout } from '../composables/useTimeout'
import { extractError } from '../utils/error'
import { formatNetworkList, parseNetworkList } from '../utils/apiToken'
import BaseSpinner from './BaseSpinner.vue'
import type { MetricsSettingsResponse } from '../types/generated'

const loading = ref(true)
const error = ref('')
const saving = ref(false)
const saved = ref(false)
const savedTimeout = useTimeout()
const form = reactive({ enabled: false, networks: '' })

const scrapeUrl = computed(() => `${window.location.origin}/metrics`)

function fill(settings: MetricsSettingsResponse): void {
  form.enabled = settings.enabled
  form.networks = formatNetworkList(settings.allowed_networks)
}

onMounted(async () => {
  try {
    fill(await getMetricsSettings())
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load metrics settings')
  } finally {
    loading.value = false
  }
})

async function save(): Promise<void> {
  saving.value = true
  saved.value = false
  error.value = ''
  try {
    fill(
      await updateMetricsSettings({
        enabled: form.enabled,
        allowed_networks: parseNetworkList(form.networks),
      }),
    )
    saved.value = true
    savedTimeout.start(() => {
      saved.value = false
    }, 2000)
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to save metrics settings')
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Prometheus metrics</h2>
    </div>
    <p class="pane-lede">
      Serve backup, repository and agent health at <code>/metrics</code> for Prometheus. Scrapers
      authenticate with a monitoring token from <RouterLink to="/tokens">API tokens</RouterLink>,
      which can read nothing else.
    </p>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <template v-else>
      <div
        v-if="error"
        class="state-msg state-msg--inline state-error"
      >
        {{ error }}
      </div>

      <form
        class="form-stack"
        @submit.prevent="save"
      >
        <label class="field toggle-row">
          <input
            id="metrics-enabled"
            v-model="form.enabled"
            type="checkbox"
          />
          <span class="toggle-row-label">Serve <code>/metrics</code></span>
        </label>

        <div class="field">
          <label
            class="field-label"
            for="metrics-networks"
          >
            Networks allowed without a token
          </label>
          <textarea
            id="metrics-networks"
            v-model="form.networks"
            class="input area-input area-input-sm"
            placeholder="10.0.0.0/8"
            spellcheck="false"
          />
          <span class="field-hint"
            >One IP address or CIDR range per line. Leave empty to require a token from every
            scraper.</span
          >
        </div>

        <dl class="info-grid">
          <dt>Scrape URL</dt>
          <dd class="mono">{{ scrapeUrl }}</dd>
        </dl>

        <div class="info-actions">
          <button
            class="btn btn-primary"
            type="submit"
            :disabled="saving"
          >
            {{ saving ? 'Saving...' : 'Save metrics settings' }}
          </button>
          <span
            v-if="saved"
            class="save-success"
          >
            Metrics settings saved
          </span>
        </div>
      </form>
    </template>
  </div>
</template>

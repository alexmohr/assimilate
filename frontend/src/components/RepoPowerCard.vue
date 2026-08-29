<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import { updateRepoPower } from '../api/repos'
import { extractError } from '../utils/error'
import EditableSection from './EditableSection.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { RepoWithStats } from '../types/repo'

/**
 * Waking the machine hosting this repository before a backup writes to it,
 * and powering it back down when the run is done.
 *
 * No agent-process section here, unlike `AgentPowerCard` - a repository host
 * isn't running Assimilate, it's just an SSH destination borg writes to, so
 * there's nothing to start or stop beyond the machine itself.
 */
const props = defineProps<{
  repo: RepoWithStats
  isAdmin: boolean
}>()

const emit = defineEmits<{ saved: [] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const wakeEnabled = ref(false)
const wakeMac = ref('')
const wakeBroadcast = ref('')
const wakeTimeout = ref(180)
const shutdownAfterBackup = ref(false)

// See AgentPowerCard.vue: the field is only hidden by the parent's v-if, not
// reset, so left stale it would resubmit a value the server rejects once
// wake is off, with no way back to the field that fixes it.
watch(wakeEnabled, (enabled) => {
  if (!enabled) shutdownAfterBackup.value = false
})

function startEdit(): void {
  const power = props.repo.power
  wakeEnabled.value = power.wake_enabled
  wakeMac.value = power.wake_mac_address ?? ''
  wakeBroadcast.value = power.wake_broadcast_address ?? ''
  wakeTimeout.value = power.wake_timeout_seconds
  shutdownAfterBackup.value = power.shutdown_after_backup
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    await updateRepoPower(props.repo.id, {
      wake_enabled: wakeEnabled.value,
      wake_mac_address: wakeMac.value.trim() || null,
      wake_broadcast_address: wakeBroadcast.value.trim() || null,
      wake_timeout_seconds: wakeTimeout.value,
      shutdown_after_backup: shutdownAfterBackup.value,
    })
    emit('saved')
    editing.value = false
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <EditableSection
    lede="Wake the machine hosting this repository before a backup writes to it, and power it back
      down when the run is done."
    :editing="editing"
    :can-edit="isAdmin"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <dl class="info-grid">
        <dt>Wake host before backup</dt>
        <dd>{{ repo.power.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
        <template v-if="repo.power.wake_enabled">
          <dt>MAC address</dt>
          <dd class="mono">{{ repo.power.wake_mac_address ?? 'Not set' }}</dd>
          <dt>Broadcast address</dt>
          <dd class="mono">{{ repo.power.wake_broadcast_address ?? 'Default' }}</dd>
          <dt>Wait for host</dt>
          <dd>{{ repo.power.wake_timeout_seconds }} seconds</dd>
          <dt>Shut down host after backup</dt>
          <dd>{{ repo.power.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
        </template>
      </dl>
    </template>

    <template #edit>
      <div class="field field-inline">
        <div class="field-body">
          <p class="field-title">Wake host before backup</p>
          <p class="field-hint">
            Checked before every backup, reusing the same connection check as
            <em>Test connection</em> on the Repository section - the Wake-on-LAN packet below is
            only sent if the host doesn't respond.
          </p>
        </div>
        <ToggleSwitch v-model="wakeEnabled" />
      </div>

      <template v-if="wakeEnabled">
        <div class="field">
          <label
            class="field-label"
            for="repo-power-wake-mac"
            >MAC address</label
          >
          <input
            id="repo-power-wake-mac"
            v-model="wakeMac"
            class="input mono"
            placeholder="9C:B6:D0:1A:44:7F"
          />
        </div>

        <div class="field">
          <label
            class="field-label"
            for="repo-power-wake-broadcast"
            >Broadcast address</label
          >
          <input
            id="repo-power-wake-broadcast"
            v-model="wakeBroadcast"
            class="input mono"
            placeholder="192.168.1.255"
          />
          <span class="field-hint"
            >Optional - defaults to the global broadcast address when unset.</span
          >
        </div>

        <div class="field">
          <label
            class="field-label"
            for="repo-power-wake-timeout"
            >Wait for host (seconds)</label
          >
          <input
            id="repo-power-wake-timeout"
            v-model.number="wakeTimeout"
            type="number"
            min="1"
            class="input"
          />
          <span class="field-hint"
            >How long to wait for SSH before the backup is marked failed.</span
          >
        </div>

        <div class="field field-inline">
          <div class="field-body">
            <p class="field-title">Shut down host after backup</p>
            <p class="field-hint">
              Only if this run woke it - a repository host that was already on is left running,
              since other schedules may still be writing to it.
            </p>
          </div>
          <ToggleSwitch v-model="shutdownAfterBackup" />
        </div>
      </template>
    </template>
  </EditableSection>
</template>

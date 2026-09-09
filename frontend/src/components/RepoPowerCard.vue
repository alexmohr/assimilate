<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { updateRepoPower } from '../api/repos'
import { listRepoSchedules } from '../api/schedules'
import { extractError } from '../utils/error'
import EditableSection from './EditableSection.vue'
import HelpHint from './HelpHint.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { RepoWithStats } from '../types/repo'
import type { ScheduleRow } from '../types/schedule'

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

/**
 * See `AgentPowerCard`: an address missing while waking or shutting down is
 * on has been redacted for this viewer, not left unconfigured -
 * `repos_wake_requires_mac` and `repos_shutdown_requires_mac` guarantee one
 * exists.
 */
const wakeSecretsHidden = computed(
  () =>
    props.repo.power.wake_mac_address === null &&
    (props.repo.power.wake_enabled || props.repo.power.shutdown_after_backup),
)

const wakeEnabled = ref(false)
const wakeMac = ref('')
const wakeBroadcast = ref('')
const wakeTimeout = ref(180)
const shutdownAfterBackup = ref(false)

// See AgentPowerCard.vue: shutting down keys off having a MAC address rather
// than off this host's own wake toggle, since a schedule can wake a host the
// toggle leaves alone. Left stale, this would resubmit a value the server
// rejects with the field that fixes it out of reach.
watch(wakeMac, (mac) => {
  if (mac.trim() === '') shutdownAfterBackup.value = false
})

/**
 * Schedules that wake this host whatever the toggle below says - see
 * `AgentPowerCard`, including why this is scoped to what the viewer may see
 * and so under-reports rather than reporting past the visibility rule.
 * Best-effort: a failed load leaves the note out.
 */
const overridingSchedules = ref<ScheduleRow[]>([])

onMounted(async () => {
  try {
    const rows = await listRepoSchedules(props.repo.id)
    overridingSchedules.value = rows.filter((s) => s.wake_override === 'enabled')
  } catch {
    overridingSchedules.value = []
  }
})

/** See `AgentPowerCard`: the wake details outlive the host's own toggle. */
const showWakeDetails = computed(
  () =>
    props.repo.power.wake_enabled ||
    props.repo.power.wake_mac_address !== null ||
    props.repo.power.shutdown_after_backup,
)

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
    lede-label="repository power"
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
        <dt>
          Wake host before backup
          <HelpHint label="wake host before backup">
            This is the default for jobs that do not set their own. A schedule can override it under
            its Settings, in Power.
          </HelpHint>
        </dt>
        <dd>{{ repo.power.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
        <template v-if="showWakeDetails">
          <dt>MAC address</dt>
          <!-- See AgentPowerCard: a null MAC inside this block is redacted. -->
          <dd class="mono">{{ repo.power.wake_mac_address ?? 'Hidden' }}</dd>
          <dt>Broadcast address</dt>
          <dd class="mono">
            {{ repo.power.wake_broadcast_address ?? (wakeSecretsHidden ? 'Hidden' : 'Default') }}
          </dd>
          <dt>Wait for host</dt>
          <dd>{{ repo.power.wake_timeout_seconds }} seconds</dd>
          <dt>Shut down host after backup</dt>
          <dd>{{ repo.power.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
        </template>
      </dl>
      <p
        v-if="overridingSchedules.length > 0"
        class="field-hint"
      >
        {{ overridingSchedules.length }}
        {{ overridingSchedules.length === 1 ? 'schedule wakes' : 'schedules wake' }} this host
        whatever the setting above says:
        <span class="override-links">
          <RouterLink
            v-for="s in overridingSchedules"
            :key="s.id"
            class="override-link"
            :to="`/schedules/${s.id}?tab=settings&section=power`"
            >{{ s.name || `Schedule #${s.id}` }}</RouterLink
          >
        </span>
      </p>
    </template>

    <template #edit>
      <div class="field field-inline">
        <div class="field-body">
          <p class="field-title">
            Wake host before backup
            <HelpHint label="wake host before backup">
              Checked before every backup, reusing the same connection check as
              <em>Test connection</em> on the Repository section - the Wake-on-LAN packet below is
              only sent if the host doesn't respond. This is the default for jobs that do not set
              their own; a schedule can override it either way.
            </HelpHint>
          </p>
        </div>
        <ToggleSwitch v-model="wakeEnabled" />
      </div>

      <div class="field">
        <div class="field-label-row field-label-row--tight">
          <label
            class="field-label"
            for="repo-power-wake-mac"
            >MAC address</label
          >
          <HelpHint label="MAC address">
            Used whenever this host is woken - by the setting above, or by a schedule that asks for
            it under its own Power settings.
          </HelpHint>
        </div>
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
        <div class="field-label-row field-label-row--tight">
          <label
            class="field-label"
            for="repo-power-wake-timeout"
            >Wait for host (seconds)</label
          >
          <HelpHint label="wait for host">
            How long to wait for SSH before the backup is marked failed.
          </HelpHint>
        </div>
        <input
          id="repo-power-wake-timeout"
          v-model.number="wakeTimeout"
          type="number"
          min="1"
          class="input"
        />
      </div>

      <div class="field field-inline">
        <div class="field-body">
          <p class="field-title">
            Shut down host after backup
            <HelpHint label="shut down host after backup">
              Only if this run woke it - a repository host that was already on is left running,
              since other schedules may still be writing to it.
            </HelpHint>
          </p>
        </div>
        <ToggleSwitch v-model="shutdownAfterBackup" />
      </div>
    </template>
  </EditableSection>
</template>

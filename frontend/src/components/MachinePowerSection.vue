<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ChevronRight, CornerDownRight } from '@lucide/vue'
import { updateMachinePower, type Machine } from '../api/machines'
import { extractError } from '../utils/error'
import { scheduleChoiceText, scheduleLabel } from '../utils/machine'
import EditableSection from './EditableSection.vue'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'

/**
 * Waking a machine before a backup that needs it, and powering it back down
 * when the run is done. Set once here for every role the machine plays - the
 * agent on it, the repository host it is, the dependencies on it - so they
 * cannot disagree about which MAC address wakes it.
 *
 * A machine that is only a dependency is never shut down, so the shutdown row
 * is absent there rather than offered and refused.
 */
const props = defineProps<{
  machine: Machine
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [machine: Machine] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const power = computed(() => props.machine.power)

/**
 * An address missing while waking or shutting down is on has been redacted
 * for this viewer, not left unconfigured: `machines_wake_requires_mac` and
 * `machines_shutdown_requires_mac` guarantee one exists. Saying "Not set"
 * there would read as a broken machine.
 */
const wakeSecretsHidden = computed(
  () =>
    power.value.wake_mac_address === null &&
    (power.value.wake_enabled || power.value.shutdown_after_backup),
)

/** The wake details outlive the machine's own toggle: a schedule can still wake it. */
const showWakeDetails = computed(
  () =>
    power.value.wake_enabled ||
    power.value.wake_mac_address !== null ||
    power.value.shutdown_after_backup,
)

const wakeEnabled = ref(false)
const wakeMac = ref('')
const wakeBroadcast = ref('')
const wakeTimeout = ref(180)
const shutdownAfterBackup = ref(false)

// Shutting down keys off having a MAC address rather than off the machine's
// own wake toggle, since a schedule can wake a machine the toggle leaves
// alone. Left stale, this would resubmit a value the server rejects with the
// field that fixes it out of reach.
watch(wakeMac, (mac) => {
  if (mac.trim() === '') shutdownAfterBackup.value = false
})

// The pane is reused when the page moves to another machine; a half-edited
// form belongs to the previous one.
watch(
  () => props.machine.id,
  () => {
    editing.value = false
  },
)

function startEdit(): void {
  wakeEnabled.value = power.value.wake_enabled
  wakeMac.value = power.value.wake_mac_address ?? ''
  wakeBroadcast.value = power.value.wake_broadcast_address ?? ''
  wakeTimeout.value = power.value.wake_timeout_seconds
  shutdownAfterBackup.value = power.value.shutdown_after_backup
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const updated = await updateMachinePower(props.machine.id, {
      wake_enabled: wakeEnabled.value,
      wake_mac_address: wakeMac.value.trim() || null,
      wake_broadcast_address: wakeBroadcast.value.trim() || null,
      wake_timeout_seconds: wakeTimeout.value,
      shutdown_after_backup: props.machine.can_shut_down && shutdownAfterBackup.value,
    })
    emit('saved', updated)
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
    label="Wake and shut down"
    lede="Wake this machine before a backup that needs it, and power it back down when the run is
      done."
    lede-label="waking this machine"
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <dl class="info-grid">
        <dt>
          Wake before backup
          <HelpHint label="wake before backup">
            The default for schedules that do not make their own choice. A schedule can override it
            in its Power &amp; dependencies settings.
          </HelpHint>
        </dt>
        <dd>{{ power.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
        <template v-if="showWakeDetails">
          <dt>MAC address</dt>
          <!-- Only rendered when the machine wakes, shuts down or has an
               address on file, so a null MAC here is always a redacted one. -->
          <dd class="mono">{{ power.wake_mac_address ?? 'Hidden' }}</dd>
          <dt>Broadcast address</dt>
          <dd class="mono">
            {{ power.wake_broadcast_address ?? (wakeSecretsHidden ? 'Hidden' : 'Default') }}
          </dd>
          <dt>Wait after waking</dt>
          <dd>{{ power.wake_timeout_seconds }} seconds</dd>
        </template>
        <template v-if="machine.can_shut_down">
          <dt>Shut down after backup</dt>
          <dd>{{ power.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
        </template>
      </dl>
      <p
        v-if="!machine.can_shut_down"
        class="field-hint"
      >
        Assimilate never shuts down a machine that is only a dependency.
      </p>

      <div
        v-if="machine.schedule_overrides.length > 0"
        class="override-note"
      >
        <p class="override-lead">
          <CornerDownRight :size="14" />
          <span>
            {{ machine.schedule_overrides.length }}
            {{ machine.schedule_overrides.length === 1 ? 'schedule does' : 'schedules do' }} not
            leave this machine to the setting above
          </span>
        </p>
        <div class="override-links">
          <RouterLink
            v-for="entry in machine.schedule_overrides"
            :key="entry.schedule_id"
            class="override-link"
            :to="{
              path: `/schedules/${entry.schedule_id}`,
              query: { tab: 'settings', section: 'power' },
            }"
          >
            <span>{{ scheduleLabel(entry) }} · {{ scheduleChoiceText(entry) }}</span>
            <ChevronRight :size="14" />
          </RouterLink>
        </div>
      </div>
    </template>

    <template #edit>
      <div class="pane-rows">
        <PaneRow
          title="Wake before backup"
          help="wake before backup"
        >
          <template #help>
            Checked before every backup that needs this machine. The Wake-on-LAN packet is only sent
            if it does not already answer, and only once per run, however many of its roles the run
            needs. The default for schedules that do not make their own choice.
          </template>
          <ToggleSwitch
            v-model="wakeEnabled"
            label="Wake before backup"
          />
        </PaneRow>

        <div class="pane-nest">
          <PaneRow
            title="MAC address"
            label-for="machine-wake-mac"
            help="where the wake packet is sent"
            stack
          >
            <template #help>
              Used whenever this machine is woken - by the setting above, or by a schedule that asks
              for it.
            </template>
            <input
              id="machine-wake-mac"
              v-model="wakeMac"
              class="input mono"
              placeholder="9C:B6:D0:1A:44:7F"
            />
          </PaneRow>

          <PaneRow
            title="Broadcast address"
            label-for="machine-wake-broadcast"
            hint="Optional. Defaults to the global broadcast address when unset."
            stack
          >
            <input
              id="machine-wake-broadcast"
              v-model="wakeBroadcast"
              class="input mono"
              placeholder="192.168.1.255"
            />
          </PaneRow>

          <PaneRow
            title="Wait after waking (seconds)"
            label-for="machine-wake-timeout"
            help="the wake deadline"
          >
            <template #help>
              How long to wait for the machine to answer after the wake packet before the backup is
              marked failed - or skipped, when the run waits for it.
            </template>
            <input
              id="machine-wake-timeout"
              v-model.number="wakeTimeout"
              type="number"
              min="1"
              class="input"
            />
          </PaneRow>
        </div>

        <PaneRow
          v-if="machine.can_shut_down"
          title="Shut down after backup"
          help="shut down after backup"
        >
          <template #help>
            Only if this run woke it, and only once nothing in the run still needs it. A machine
            that was already on is left running.
          </template>
          <ToggleSwitch
            v-model="shutdownAfterBackup"
            label="Shut down after backup"
          />
        </PaneRow>
      </div>
    </template>
  </EditableSection>
</template>

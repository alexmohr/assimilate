<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { updateRepoHostPower, type RepoHost } from '../api/repoHosts'
import { listRepoSchedules } from '../api/schedules'
import { extractError } from '../utils/error'
import EditableSection from './EditableSection.vue'
import HelpHint from './HelpHint.vue'
import OverrideNote from './OverrideNote.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { ScheduleRow } from '../types/schedule'

/**
 * Waking a repository host before a backup writes to any repository on it,
 * and powering it back down when the run is done. Set once here for every
 * repository on the machine: they cannot disagree about which MAC address
 * wakes it, or whether it is shut down afterwards.
 *
 * No agent-process section here, unlike `AgentPowerCard` - a repository host
 * isn't running Assimilate, it's just an SSH destination borg writes to, so
 * there's nothing to start or stop beyond the machine itself.
 */
const props = defineProps<{
  host: RepoHost
  isAdmin: boolean
}>()

const emit = defineEmits<{ saved: [] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

/**
 * See `AgentPowerCard`: an address missing while waking or shutting down is
 * on has been redacted for this viewer, not left unconfigured -
 * `repo_hosts_wake_requires_mac` and `repo_hosts_shutdown_requires_mac`
 * guarantee one exists.
 */
const wakeSecretsHidden = computed(
  () =>
    props.host.power.wake_mac_address === null &&
    (props.host.power.wake_enabled || props.host.power.shutdown_after_backup),
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
    // Every schedule writing to any repository on this host can wake it, and
    // one writing to two of them is still one schedule.
    const perRepo = await Promise.all(props.host.repositories.map((r) => listRepoSchedules(r.id)))
    const byId = new Map(perRepo.flat().map((s) => [s.id, s]))
    overridingSchedules.value = [...byId.values()].filter((s) => s.wake_override === 'enabled')
  } catch {
    overridingSchedules.value = []
  }
})

/** See `AgentPowerCard`: the wake details outlive the host's own toggle. */
const showWakeDetails = computed(
  () =>
    props.host.power.wake_enabled ||
    props.host.power.wake_mac_address !== null ||
    props.host.power.shutdown_after_backup,
)

function startEdit(): void {
  const power = props.host.power
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
    await updateRepoHostPower(props.host.id, {
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
    lede="Wake this host before a backup writes to any repository on it, and power it back down
      when the run is done."
    lede-label="waking its host"
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
        <dd>{{ host.power.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
        <template v-if="showWakeDetails">
          <dt>MAC address</dt>
          <!-- See AgentPowerCard: a null MAC inside this block is redacted. -->
          <dd class="mono">{{ host.power.wake_mac_address ?? 'Hidden' }}</dd>
          <dt>Broadcast address</dt>
          <dd class="mono">
            {{ host.power.wake_broadcast_address ?? (wakeSecretsHidden ? 'Hidden' : 'Default') }}
          </dd>
          <dt>Wait for host</dt>
          <dd>{{ host.power.wake_timeout_seconds }} seconds</dd>
          <dt>Shut down host after backup</dt>
          <dd>{{ host.power.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
        </template>
      </dl>
      <OverrideNote :schedules="overridingSchedules" />
    </template>

    <template #edit>
      <div class="pane-rows">
        <PaneRow
          title="Wake host before backup"
          help="wake host before backup"
        >
          <template #help>
            Checked before every backup, reusing the same connection check as
            <em>Test connection</em> on a repository - the Wake-on-LAN packet below is only sent if
            the host doesn't respond. Woken once per run, however many of its repositories the run
            writes to. This is the default for jobs that do not set their own; a schedule can
            override it either way.
          </template>
          <ToggleSwitch
            v-model="wakeEnabled"
            label="Wake host before backup"
          />
        </PaneRow>

        <div class="pane-nest">
          <PaneRow
            title="MAC address"
            label-for="repo-host-power-wake-mac"
            help="where the wake packet is sent"
            stack
          >
            <template #help>
              Used whenever this host is woken - by the setting above, or by a schedule that asks
              for it under its own Power settings.
            </template>
            <input
              id="repo-host-power-wake-mac"
              v-model="wakeMac"
              class="input mono"
              placeholder="9C:B6:D0:1A:44:7F"
            />
          </PaneRow>

          <PaneRow
            title="Broadcast address"
            label-for="repo-host-power-wake-broadcast"
            hint="Optional - defaults to the global broadcast address when unset."
            stack
          >
            <input
              id="repo-host-power-wake-broadcast"
              v-model="wakeBroadcast"
              class="input mono"
              placeholder="192.168.1.255"
            />
          </PaneRow>

          <PaneRow
            title="Wait for host (seconds)"
            label-for="repo-host-power-wake-timeout"
            help="the reconnect deadline"
          >
            <template #help>
              How long to wait for SSH before the backup is marked failed.
            </template>
            <input
              id="repo-host-power-wake-timeout"
              v-model.number="wakeTimeout"
              type="number"
              min="1"
              class="input"
            />
          </PaneRow>
        </div>

        <PaneRow
          title="Shut down host after backup"
          help="shut down host after backup"
        >
          <template #help>
            Only if this run woke it, and only once the last of its repositories this run writes to
            is done - a repository host that was already on is left running, since other schedules
            may still be writing to it.
          </template>
          <ToggleSwitch
            v-model="shutdownAfterBackup"
            label="Shut down host after backup"
          />
        </PaneRow>
      </div>
    </template>
  </EditableSection>
</template>

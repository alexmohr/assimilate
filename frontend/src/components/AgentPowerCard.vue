<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { updateAgentPower } from '../api/agents'
import { extractError } from '../utils/error'
import EditableSection from './EditableSection.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'

/**
 * Waking an agent's host before a backup, and starting the agent process
 * itself over SSH if it still isn't connecting once the host is up - undoing
 * only what this run turned on, once the backup is done.
 *
 * Both dependent groups (wake details, agent-process details) stay in the
 * form once their toggle is off rather than disappearing, since a value
 * entered and then hidden by an accidental click would otherwise be lost.
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to push settings to. */
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [agent: AgentRow] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const wakeEnabled = ref(false)
const wakeMac = ref('')
const wakeBroadcast = ref('')
const wakeTimeout = ref(180)
const shutdownAfterBackup = ref(false)
const startAgentEnabled = ref(false)
const stopAgentAfterBackup = ref(false)
const sshHost = ref('')
const sshPort = ref(22)
const serviceName = ref('assimilate-agent')

// A dependent toggle's field is only hidden by the parent's v-if, not reset -
// left stale it would silently resubmit a value the server rejects once the
// parent that justified it is off, with no way back to the field that fixes it.
watch(wakeEnabled, (enabled) => {
  if (!enabled) shutdownAfterBackup.value = false
})
watch(startAgentEnabled, (enabled) => {
  if (!enabled) stopAgentAfterBackup.value = false
})

// Shutting down needs an SSH destination just as much as starting the agent
// does - a wake-only host (wake + shutdown enabled, agent already running as
// a persistent service) never sets startAgentEnabled, but still needs
// somewhere to send `shutdown -h now`. The field lives in the "Agent
// process" section for layout reasons, but its visibility follows both
// toggles that can need it, not just start-agent.
const needsSshHost = computed(
  () => startAgentEnabled.value || (wakeEnabled.value && shutdownAfterBackup.value),
)

function startEdit(): void {
  const power = props.agent.power
  wakeEnabled.value = power.wake.wake_enabled
  wakeMac.value = power.wake.wake_mac_address ?? ''
  wakeBroadcast.value = power.wake.wake_broadcast_address ?? ''
  wakeTimeout.value = power.wake.wake_timeout_seconds
  shutdownAfterBackup.value = power.wake.shutdown_after_backup
  startAgentEnabled.value = power.start_agent_enabled
  stopAgentAfterBackup.value = power.stop_agent_after_backup
  sshHost.value = power.ssh_host ?? ''
  sshPort.value = power.ssh_port
  serviceName.value = power.agent_service_name
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const res = await updateAgentPower(
      props.agent.hostname,
      {
        wake: {
          wake_enabled: wakeEnabled.value,
          wake_mac_address: wakeMac.value.trim() || null,
          wake_broadcast_address: wakeBroadcast.value.trim() || null,
          wake_timeout_seconds: wakeTimeout.value,
          shutdown_after_backup: shutdownAfterBackup.value,
        },
        start_agent_enabled: startAgentEnabled.value,
        stop_agent_after_backup: stopAgentAfterBackup.value,
        ssh_host: sshHost.value.trim() || null,
        ssh_port: sshPort.value,
        agent_service_name: serviceName.value.trim() || 'assimilate-agent',
      },
      props.agent.domain,
    )
    emit('saved', res)
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
    lede="Wake this host before a backup runs, and let Assimilate power it back down when it's
      done."
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <section
        class="pane-section"
        style="border-top: none; padding-top: 0"
      >
        <div class="pane-section-head">
          <span class="group-label group-label--lg">Host power</span>
        </div>
        <dl class="info-grid">
          <dt>Wake host before backup</dt>
          <dd>{{ agent.power.wake.wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
          <template v-if="agent.power.wake.wake_enabled">
            <dt>MAC address</dt>
            <dd class="mono">{{ agent.power.wake.wake_mac_address ?? 'Not set' }}</dd>
            <dt>Broadcast address</dt>
            <dd class="mono">{{ agent.power.wake.wake_broadcast_address ?? 'Default' }}</dd>
            <dt>Wait for host</dt>
            <dd>{{ agent.power.wake.wake_timeout_seconds }} seconds</dd>
            <dt>Shut down host after backup</dt>
            <dd>{{ agent.power.wake.shutdown_after_backup ? 'Enabled' : 'Disabled' }}</dd>
          </template>
        </dl>
      </section>

      <section class="pane-section">
        <div class="pane-section-head">
          <span class="group-label group-label--lg">Agent process</span>
        </div>
        <dl class="info-grid">
          <dt>Start agent before backup</dt>
          <dd>{{ agent.power.start_agent_enabled ? 'Enabled' : 'Disabled' }}</dd>
          <template
            v-if="
              agent.power.start_agent_enabled ||
              (agent.power.wake.wake_enabled && agent.power.wake.shutdown_after_backup)
            "
          >
            <dt>SSH host</dt>
            <dd class="mono">{{ agent.power.ssh_host ?? 'Not set' }}:{{ agent.power.ssh_port }}</dd>
          </template>
          <template v-if="agent.power.start_agent_enabled">
            <dt>Service name</dt>
            <dd class="mono">{{ agent.power.agent_service_name }}</dd>
            <dt>Stop agent after backup</dt>
            <dd>{{ agent.power.stop_agent_after_backup ? 'Enabled' : 'Disabled' }}</dd>
          </template>
        </dl>
      </section>
    </template>

    <template #edit>
      <section
        class="pane-section"
        style="border-top: none; padding-top: 0"
      >
        <div class="pane-section-head">
          <span class="group-label group-label--lg">Host power</span>
        </div>

        <div class="field field-inline">
          <div class="field-body">
            <p class="field-title">Wake host before backup</p>
            <p class="field-hint">
              Checked before every backup - the Wake-on-LAN packet below is only sent if the agent
              doesn't already respond.
            </p>
          </div>
          <ToggleSwitch v-model="wakeEnabled" />
        </div>

        <template v-if="wakeEnabled">
          <div class="field">
            <label
              class="field-label"
              for="power-wake-mac"
              >MAC address</label
            >
            <input
              id="power-wake-mac"
              v-model="wakeMac"
              class="input mono"
              placeholder="3C:97:0E:2B:9A:44"
            />
          </div>

          <div class="field">
            <label
              class="field-label"
              for="power-wake-broadcast"
              >Broadcast address</label
            >
            <input
              id="power-wake-broadcast"
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
              for="power-wake-timeout"
              >Wait for host (seconds)</label
            >
            <input
              id="power-wake-timeout"
              v-model.number="wakeTimeout"
              type="number"
              min="1"
              class="input"
            />
            <span class="field-hint"
              >How long to wait for the agent to reconnect before the backup is marked failed.</span
            >
          </div>

          <div class="field field-inline">
            <div class="field-body">
              <p class="field-title">Shut down host after backup</p>
              <p class="field-hint">
                Only if this run woke it - a host that was already on when the backup started is
                left running.
              </p>
            </div>
            <ToggleSwitch v-model="shutdownAfterBackup" />
          </div>
        </template>
      </section>

      <section class="pane-section">
        <div class="pane-section-head">
          <span class="group-label group-label--lg">Agent process</span>
        </div>

        <div class="field field-inline">
          <div class="field-body">
            <p class="field-title">Start agent before backup</p>
            <p class="field-hint">
              Checked first, same as above - only started if the agent isn't already connected. For
              hosts where it runs on demand instead of as a background service.
            </p>
          </div>
          <ToggleSwitch v-model="startAgentEnabled" />
        </div>

        <template v-if="needsSshHost">
          <div class="field-row">
            <div class="field">
              <label
                class="field-label"
                for="power-ssh-host"
                >SSH host</label
              >
              <input
                id="power-ssh-host"
                v-model="sshHost"
                class="input mono"
                placeholder="web-01.lan"
              />
            </div>
            <div class="field field-narrow">
              <label
                class="field-label"
                for="power-ssh-port"
                >SSH port</label
              >
              <input
                id="power-ssh-port"
                v-model.number="sshPort"
                type="number"
                min="1"
                max="65535"
                class="input"
              />
            </div>
          </div>
          <span
            v-if="!startAgentEnabled"
            class="field-hint"
            >Needed to shut this host down after backup - the agent itself already runs as a
            persistent service.</span
          >
        </template>

        <template v-if="startAgentEnabled">
          <div class="field">
            <label
              class="field-label"
              for="power-service-name"
              >Service name</label
            >
            <input
              id="power-service-name"
              v-model="serviceName"
              class="input mono"
            />
          </div>

          <div class="field field-inline">
            <div class="field-body">
              <p class="field-title">Stop agent after backup</p>
              <p class="field-hint">Only if this run started it.</p>
            </div>
            <ToggleSwitch v-model="stopAgentAfterBackup" />
          </div>
        </template>
      </section>
    </template>
  </EditableSection>
</template>

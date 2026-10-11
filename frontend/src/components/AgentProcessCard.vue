<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { updateAgentProcess } from '../api/agents'
import { extractError } from '../utils/error'
import EditableSection from './EditableSection.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'

/**
 * Starting the agent process over SSH if it still is not connecting once its
 * machine is up, stopping it again afterwards, and the SSH destination that is
 * also how the machine is shut down. Waking and shutting the machine down are
 * the machine's, in the sections above.
 *
 * The details stay in the form once their toggle is off rather than
 * disappearing, since a value entered and then hidden by an accidental click
 * would otherwise be lost.
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to start. */
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [agent: AgentRow] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const startAgentEnabled = ref(false)
const stopAgentAfterBackup = ref(false)
const sshHost = ref('')
const sshPort = ref(22)
const serviceName = ref('assimilate-agent')

watch(startAgentEnabled, (enabled) => {
  if (!enabled) stopAgentAfterBackup.value = false
})

/** Shutting the machine down goes over the same SSH destination as starting the agent. */
const shutsDown = computed(() => props.agent.power.wake.shutdown_after_backup)

// Mirrors `update_agent_process`: start-agent OR the machine's shutdown needs
// an SSH host. Gating the field on start-agent alone would leave the server
// rejecting the save with the field that fixes it unrendered.
const needsSshHost = computed(() => startAgentEnabled.value || shutsDown.value)

function startEdit(): void {
  const power = props.agent.power
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
    const res = await updateAgentProcess(
      props.agent.hostname,
      {
        start_agent_enabled: startAgentEnabled.value,
        stop_agent_after_backup: stopAgentAfterBackup.value,
        ssh_host: sshHost.value.trim() || null,
        ssh_port: sshPort.value,
        agent_service_name: serviceName.value.trim(),
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
  <section class="pane-section">
    <EditableSection
      label="Agent process"
      lede="Start the agent over SSH before a backup when it runs on demand rather than as a
        background service."
      lede-label="starting the agent"
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
          <dt>Start agent before backup</dt>
          <dd>{{ agent.power.start_agent_enabled ? 'Enabled' : 'Disabled' }}</dd>
          <template v-if="agent.power.start_agent_enabled || shutsDown">
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
      </template>

      <template #edit>
        <div class="pane-rows">
          <PaneRow
            title="Start agent before backup"
            help="start agent before backup"
          >
            <template #help>
              Checked once the machine is up - only started if the agent is not already connected.
              For hosts where it runs on demand instead of as a background service.
            </template>
            <ToggleSwitch
              v-model="startAgentEnabled"
              label="Start agent before backup"
            />
          </PaneRow>

          <!-- Gated as a whole: with neither the agent started nor the machine
               shut down, both children are absent. -->
          <div
            v-if="needsSshHost"
            class="pane-nest"
          >
            <PaneRow
              title="SSH host"
              label-for="power-ssh-host"
              :help="startAgentEnabled ? undefined : 'needed for shutdown only'"
              stack
            >
              <template #help>
                Needed to shut this machine down after backup - the agent itself already runs as a
                persistent service.
              </template>
              <div class="field-row">
                <input
                  id="power-ssh-host"
                  v-model="sshHost"
                  class="input mono"
                  placeholder="web-01.lan"
                />
                <input
                  id="power-ssh-port"
                  v-model.number="sshPort"
                  type="number"
                  min="1"
                  max="65535"
                  class="input field-narrow"
                  aria-label="SSH port"
                />
              </div>
            </PaneRow>

            <template v-if="startAgentEnabled">
              <PaneRow
                title="Service name"
                label-for="power-service-name"
                stack
              >
                <input
                  id="power-service-name"
                  v-model="serviceName"
                  class="input mono"
                />
              </PaneRow>

              <PaneRow
                title="Stop agent after backup"
                help="stop agent after backup"
              >
                <template #help> Only if this run started it. </template>
                <ToggleSwitch
                  v-model="stopAgentAfterBackup"
                  label="Stop agent after backup"
                />
              </PaneRow>
            </template>
          </div>
        </div>
      </template>
    </EditableSection>
  </section>
</template>

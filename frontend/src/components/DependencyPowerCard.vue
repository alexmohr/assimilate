<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import BaseSegmented from './BaseSegmented.vue'
import EditableSection from './EditableSection.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import { updateDependencyHostPower, type DependencyHost } from '../api/dependencyHosts'
import { listRepoHosts, type RepoHost } from '../api/repoHosts'
import { extractError } from '../utils/error'

/**
 * Waking a dependency before a backup that needs it. Either its own
 * Wake-on-LAN settings, or - for a machine that is also a repository host -
 * that host's, so one machine has one MAC address and one wait.
 *
 * There is deliberately no shutdown here: Assimilate never powers a
 * dependency down. A shared repository host still is, by its own setting, but
 * only once no run that needs this dependency is still going.
 */
const props = defineProps<{
  host: DependencyHost
  isAdmin: boolean
}>()

const emit = defineEmits<{ saved: [host: DependencyHost] }>()

type PowerSource = 'shared' | 'own'

const SOURCE_OPTIONS: readonly { value: PowerSource; label: string }[] = [
  { value: 'shared', label: 'Same as a repository host' },
  { value: 'own', label: 'Own wake settings' },
]

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const source = ref<PowerSource>('own')
const repoHostId = ref<number | null>(null)
const repoHosts = ref<RepoHost[]>([])
const wakeEnabled = ref(false)
const wakeMac = ref('')
const wakeBroadcast = ref('')
const wakeTimeout = ref(180)

const power = computed(() => props.host.power)

/**
 * A missing MAC address while waking is on has been withheld from this
 * viewer, not left unset - a wake needs one to be sent anywhere.
 */
const secretsHidden = computed(
  () => power.value.effective_wake_mac_address === null && power.value.effective_wake_enabled,
)

const showWakeDetails = computed(
  () => power.value.effective_wake_enabled || power.value.effective_wake_mac_address !== null,
)

const broadcastText = computed(
  () =>
    power.value.effective_wake_broadcast_address ?? (secretsHidden.value ? 'Hidden' : 'Default'),
)

/** Best-effort: without the list, the current host is still offered. */
async function loadRepoHosts(): Promise<void> {
  try {
    repoHosts.value = await listRepoHosts()
  } catch {
    repoHosts.value = []
  }
}

/** The hosts the select offers, always including the one already chosen. */
const repoHostOptions = computed<{ id: number; ssh_host: string }[]>(() => {
  const shared = power.value.repo_host
  const listed = repoHosts.value.map((h) => ({ id: h.id, ssh_host: h.ssh_host }))
  if (shared && !listed.some((h) => h.id === shared.id)) listed.unshift(shared)
  return listed
})

function startEdit(): void {
  // The own values are kept while the repository host's apply, so they are
  // what the form starts from either way - and what is sent back unchanged
  // when only the source moves.
  source.value = power.value.repo_host ? 'shared' : 'own'
  repoHostId.value = power.value.repo_host?.id ?? null
  wakeEnabled.value = power.value.wake_enabled
  wakeMac.value = power.value.wake_mac_address ?? ''
  wakeBroadcast.value = power.value.wake_broadcast_address ?? ''
  wakeTimeout.value = power.value.wake_timeout_seconds
  error.value = null
  editing.value = true
  void loadRepoHosts()
}

async function save(): Promise<void> {
  const shared = source.value === 'shared'
  if (shared && repoHostId.value === null) {
    error.value = 'Pick the repository host this dependency is the same machine as.'
    return
  }
  saving.value = true
  error.value = null
  try {
    const updated = await updateDependencyHostPower(props.host.id, {
      repo_host_id: shared ? repoHostId.value : null,
      wake_enabled: wakeEnabled.value,
      wake_mac_address: wakeMac.value.trim() || null,
      wake_broadcast_address: wakeBroadcast.value.trim() || null,
      wake_timeout_seconds: wakeTimeout.value,
    })
    editing.value = false
    emit('saved', updated)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <EditableSection
    lede="Wake this machine before a backup that needs it. Assimilate never shuts a dependency down."
    lede-label="waking a dependency"
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
        <dt>Power settings</dt>
        <dd v-if="power.repo_host">
          Shared with repository host
          <RouterLink
            class="mono"
            :to="{ path: `/repo-hosts/${power.repo_host.id}`, query: { section: 'power' } }"
          >
            {{ power.repo_host.ssh_host }}
          </RouterLink>
        </dd>
        <dd v-else>Own wake settings</dd>
        <dt>Wake host before backup</dt>
        <dd>{{ power.effective_wake_enabled ? 'Enabled' : 'Disabled' }}</dd>
        <template v-if="showWakeDetails">
          <dt>MAC address</dt>
          <dd class="mono">{{ power.effective_wake_mac_address ?? 'Hidden' }}</dd>
          <dt>Broadcast address</dt>
          <dd class="mono">{{ broadcastText }}</dd>
          <dt>Wait for host</dt>
          <dd>{{ power.effective_wake_timeout_seconds }} seconds</dd>
        </template>
      </dl>
      <p
        v-if="power.repo_host"
        class="field-hint"
      >
        Edit these on {{ power.repo_host.ssh_host }}'s Power section.
        {{ power.repo_host.ssh_host }} is still shut down after a backup if its own setting says so,
        but only once no run that needs this dependency is still going.
      </p>
      <p class="field-hint">Assimilate never shuts a dependency down.</p>
    </template>

    <template #edit>
      <div class="pane-rows">
        <PaneRow
          title="Power settings"
          help="sharing a repository host's wake settings"
          stack
        >
          <template #help>
            When this dependency is the same machine as a repository host, share one set of wake
            settings, so the machine has one MAC address and is never shut down while a backup still
            needs it.
          </template>
          <BaseSegmented
            v-model="source"
            :options="SOURCE_OPTIONS"
            label="Power settings"
          />
        </PaneRow>

        <div
          v-if="source === 'shared'"
          class="pane-nest"
        >
          <PaneRow
            title="Repository host"
            label-for="dependency-power-repo-host"
            hint="Wake-on-LAN, MAC address and wait time come from this host."
            stack
          >
            <select
              id="dependency-power-repo-host"
              v-model="repoHostId"
              class="input"
            >
              <option
                :value="null"
                disabled
              >
                Choose a repository host
              </option>
              <option
                v-for="option in repoHostOptions"
                :key="option.id"
                :value="option.id"
              >
                {{ option.ssh_host }}
              </option>
            </select>
          </PaneRow>
        </div>

        <template v-else>
          <PaneRow
            title="Wake host before backup"
            help="waking a dependency before a backup"
          >
            <template #help>
              Checked before every backup that needs this machine. The Wake-on-LAN packet is only
              sent if the check fails. Woken once per run, however many hosts in the run need it.
            </template>
            <ToggleSwitch
              v-model="wakeEnabled"
              label="Wake host before backup"
            />
          </PaneRow>

          <div
            v-if="wakeEnabled"
            class="pane-nest"
          >
            <PaneRow
              title="MAC address"
              label-for="dependency-power-mac"
              stack
            >
              <input
                id="dependency-power-mac"
                v-model="wakeMac"
                class="input mono"
                placeholder="9C:B6:D0:1A:44:7F"
              />
            </PaneRow>
            <PaneRow
              title="Broadcast address"
              label-for="dependency-power-broadcast"
              hint="Optional. Defaults to the global broadcast address when unset."
              stack
            >
              <input
                id="dependency-power-broadcast"
                v-model="wakeBroadcast"
                class="input mono"
                placeholder="192.168.1.255"
              />
            </PaneRow>
            <PaneRow
              title="Wait for host (seconds)"
              label-for="dependency-power-wait"
              :hint="`How long to wait for port ${host.port} to answer after the wake packet.`"
            >
              <input
                id="dependency-power-wait"
                v-model.number="wakeTimeout"
                class="input"
                type="number"
                min="1"
              />
            </PaneRow>
          </div>
        </template>
      </div>
    </template>
  </EditableSection>
</template>

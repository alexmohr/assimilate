<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import BaseSegmented from './BaseSegmented.vue'
import EditableSection from './EditableSection.vue'
import PaneRow from './PaneRow.vue'
import { updateDependencyHostMachine, type DependencyHost } from '../api/dependencyHosts'
import { listRepoHosts, type RepoHost } from '../api/repoHosts'
import { extractError } from '../utils/error'

/**
 * Which machine a dependency is on: one of its own, or the machine of a
 * repository host - a NAS that is both where the backups go and the share a
 * pre-backup command mounts. Sharing it shares everything in the sections
 * below, so the machine has one MAC address and one answer to "is it always
 * online", and is never shut down while a backup still needs it.
 */
const props = defineProps<{
  host: DependencyHost
  isAdmin: boolean
}>()

const emit = defineEmits<{ saved: [host: DependencyHost] }>()

type MachineSource = 'shared' | 'own'

const SOURCE_OPTIONS: readonly { value: MachineSource; label: string }[] = [
  { value: 'own', label: 'Its own machine' },
  { value: 'shared', label: 'Same as a repository host' },
]

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const source = ref<MachineSource>('own')
const repoHostId = ref<number | null>(null)
const repoHosts = ref<RepoHost[]>([])

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
  const shared = props.host.repo_host
  const listed = repoHosts.value.map((h) => ({ id: h.id, ssh_host: h.ssh_host }))
  if (shared && !listed.some((h) => h.id === shared.id)) listed.unshift(shared)
  return listed
})

function startEdit(): void {
  source.value = props.host.repo_host ? 'shared' : 'own'
  repoHostId.value = props.host.repo_host?.id ?? null
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
    const updated = await updateDependencyHostMachine(
      props.host.id,
      shared ? repoHostId.value : null,
    )
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
    label="Machine"
    lede="When this dependency is the same machine as a repository host, they share one set of
      power settings."
    lede-label="sharing a machine"
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
        <dt>Machine</dt>
        <dd v-if="host.repo_host">
          Same as repository host
          <RouterLink
            class="mono"
            :to="{ path: `/repo-hosts/${host.repo_host.id}`, query: { section: 'power' } }"
          >
            {{ host.repo_host.ssh_host }}
          </RouterLink>
        </dd>
        <dd v-else>Its own machine</dd>
      </dl>
    </template>

    <template #edit>
      <div class="pane-rows">
        <PaneRow
          title="Machine"
          help="sharing a repository host's machine"
          stack
        >
          <template #help>
            Moving onto a repository host's machine takes that machine's settings. Moving back onto
            a machine of its own starts with default settings: never woken, always online.
          </template>
          <BaseSegmented
            v-model="source"
            :options="SOURCE_OPTIONS"
            label="Machine"
          />
        </PaneRow>

        <div
          v-if="source === 'shared'"
          class="pane-nest"
        >
          <PaneRow
            title="Repository host"
            label-for="dependency-machine-repo-host"
            stack
          >
            <select
              id="dependency-machine-repo-host"
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
      </div>
    </template>
  </EditableSection>
</template>

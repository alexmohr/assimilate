<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import EditableSection from './EditableSection.vue'
import DependencyPortField from './DependencyPortField.vue'
import PaneRow from './PaneRow.vue'
import { updateDependencyHost, type DependencyHost } from '../api/dependencyHosts'
import { checkDescription } from '../utils/dependencyHost'
import { extractError } from '../utils/error'

/**
 * Where a dependency is reached and which port says it is up. Checked from
 * the Assimilate server, not from the agent whose pre-backup command mounts
 * its share - the server is what decides whether a run starts.
 */
const props = defineProps<{
  host: DependencyHost
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [host: DependencyHost] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const name = ref('')
const address = ref('')
const port = ref(445)
const description = ref('')

function startEdit(): void {
  name.value = props.host.name
  address.value = props.host.address
  port.value = props.host.port
  description.value = props.host.description
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const updated = await updateDependencyHost(props.host.id, {
      name: name.value.trim(),
      address: address.value.trim(),
      port: port.value,
      description: description.value.trim(),
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
    lede="How Assimilate checks that this machine is up before a backup that needs it runs."
    lede-label="checking a dependency"
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
        <dt>Name</dt>
        <dd>{{ host.name }}</dd>
        <dt>Address</dt>
        <dd class="mono">{{ host.address }}</dd>
        <dt>Check</dt>
        <dd>{{ checkDescription(host.port) }}</dd>
        <dt>Checked from</dt>
        <dd>The Assimilate server</dd>
        <template v-if="host.power.repo_host">
          <dt>Same machine as</dt>
          <dd>
            Repository host
            <RouterLink
              class="mono"
              :to="`/repo-hosts/${host.power.repo_host.id}`"
            >
              {{ host.power.repo_host.ssh_host }}
            </RouterLink>
          </dd>
        </template>
        <dt>Description</dt>
        <dd :class="{ muted: !host.description }">{{ host.description || 'None' }}</dd>
      </dl>
      <p class="field-hint">
        An open port means the machine is up, not that the share is mounted. Keep the mount point
        check in the pre-backup command.
      </p>
    </template>

    <template #edit>
      <div class="pane-rows">
        <PaneRow
          title="Name"
          label-for="dependency-edit-name"
          stack
        >
          <input
            id="dependency-edit-name"
            v-model="name"
            class="input"
          />
        </PaneRow>
        <PaneRow
          title="Address"
          label-for="dependency-edit-address"
          hint="Hostname or IP address, as the Assimilate server reaches it."
          stack
        >
          <input
            id="dependency-edit-address"
            v-model="address"
            class="input mono"
          />
        </PaneRow>
        <PaneRow
          title="Check"
          label-for="dependency-edit-port"
          stack
        >
          <DependencyPortField
            v-model="port"
            input-id="dependency-edit-port"
          />
        </PaneRow>
        <PaneRow
          title="Description"
          label-for="dependency-edit-description"
          hint="Optional."
          stack
        >
          <input
            id="dependency-edit-description"
            v-model="description"
            class="input"
          />
        </PaneRow>
      </div>
    </template>
  </EditableSection>
</template>

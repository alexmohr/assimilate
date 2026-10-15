<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Database, RefreshCw } from '@lucide/vue'
import {
  getAgentDatabases,
  scanAgentDatabases,
  updateAgentDatabaseBackup,
  updateAgentDatabaseSource,
} from '../api/databases'
import { useAsyncAction } from '../composables/useAsyncAction'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import EmptyState from './EmptyState.vue'
import PaneRow from './PaneRow.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'
import type {
  AgentDatabaseSourceResponse,
  AgentDatabasesResponse,
  DatabaseSelectionMode,
  DatabaseSourceKind,
} from '../types/generated'

/**
 * Dumping the databases in this host's Kubernetes cluster into its backups:
 * where the agent finds the cluster, where it streams the dumps from, and which
 * database workloads are dumped.
 *
 * Like virtual-machine staging, the settings sit on the host and a schedule
 * only opts in. The source table is what the agent last found, so it is empty
 * until the cluster has been scanned once.
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to scan or push to. */
  canEdit: boolean
}>()

const loading = ref(true)
const loadError = ref<string | null>(null)
const data = ref<AgentDatabasesResponse | null>(null)

const editing = ref(false)
const { loading: saving, error: saveError, run: runSave } = useAsyncAction()
const { loading: scanning, error: scanError, run: runScan } = useAsyncAction()
const { error: rowError, run: runRow } = useAsyncAction()
/** The source whose switch is being saved, so only its row is disabled. */
const rowSaving = ref<string | null>(null)

const enabled = ref(false)
const selection = ref<DatabaseSelectionMode>('all')
const dumpDir = ref('')
const kubeconfigPath = ref('')

const SELECTION_OPTIONS: readonly SegmentedOption<DatabaseSelectionMode>[] = [
  { value: 'all', label: 'All except excluded' },
  { value: 'selected', label: 'Only selected' },
]

const SELECTION_LABELS: Record<DatabaseSelectionMode, string> = {
  all: 'Every source except the ones excluded below',
  selected: 'Only the sources selected below',
}

const KIND_LABELS: Record<DatabaseSourceKind, string> = {
  cnpg_cluster: 'CloudNativePG',
  annotated: 'Annotated pod',
}

const sources = computed<AgentDatabaseSourceResponse[]>(() => data.value?.sources ?? [])

const includedHint = computed<string>(() =>
  selection.value === 'all'
    ? 'Turn a source off to leave its databases out of the backup. A database deployed after the last scan is included automatically.'
    : 'Turn a source on to dump its databases. A database deployed after the last scan is left out until you select it.',
)

function sourceKey(source: AgentDatabaseSourceResponse): string {
  return `${source.namespace}/${source.name}`
}

function kindLabel(source: AgentDatabaseSourceResponse): string {
  return source.kind === null ? 'Not scanned' : KIND_LABELS[source.kind]
}

function databaseList(source: AgentDatabaseSourceResponse): string {
  return source.databases.length === 0 ? 'None found' : source.databases.join(', ')
}

function applySettings(response: AgentDatabasesResponse): void {
  data.value = response
  enabled.value = response.settings.enabled
  selection.value = response.settings.selection
  dumpDir.value = response.settings.dump_dir
  kubeconfigPath.value = response.settings.kubeconfig_path ?? ''
}

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    applySettings(await getAgentDatabases(props.agent.hostname, props.agent.domain))
  } catch (e: unknown) {
    loadError.value = extractError(e)
  } finally {
    loading.value = false
  }
}

/** Opening or leaving the form starts from what is saved, never a half-edit. */
function setEditing(open: boolean): void {
  if (data.value) applySettings(data.value)
  saveError.value = null
  editing.value = open
}

function kubeconfigOrNull(): string | null {
  const trimmed = kubeconfigPath.value.trim()
  return trimmed === '' ? null : trimmed
}

async function save(): Promise<void> {
  const response = await runSave(() =>
    updateAgentDatabaseBackup(
      props.agent.hostname,
      {
        enabled: enabled.value,
        selection: selection.value,
        dump_dir: dumpDir.value.trim(),
        kubeconfig_path: kubeconfigOrNull(),
      },
      props.agent.domain,
    ),
  )
  if (!response) return
  applySettings(response)
  editing.value = false
}

/**
 * Scans the cluster. While the settings are being edited, the kubeconfig in
 * the form is tried rather than the saved one, so a new path can be tested
 * before it is saved; the edited settings stay as they are.
 */
async function scan(): Promise<void> {
  const response = await runScan(() =>
    scanAgentDatabases(
      props.agent.hostname,
      editing.value ? kubeconfigOrNull() : null,
      props.agent.domain,
    ),
  )
  if (!response) return
  // The response carries the saved settings, so an open form keeps its edits.
  if (editing.value) data.value = response
  else applySettings(response)
}

async function setIncluded(source: AgentDatabaseSourceResponse, included: boolean): Promise<void> {
  rowSaving.value = sourceKey(source)
  const response = await runRow(() =>
    updateAgentDatabaseSource(
      props.agent.hostname,
      source.namespace,
      source.name,
      included,
      props.agent.domain,
    ),
  )
  rowSaving.value = null
  if (response) data.value = response
}

onMounted(load)
</script>

<template>
  <BaseSpinner
    v-if="loading && !data"
    label="Loading databases"
  />
  <p
    v-else-if="loadError && !data"
    class="form-error"
  >
    {{ loadError }}
  </p>

  <template v-else>
    <EditableSection
      lede="Dump the databases running in this host's Kubernetes cluster into its backups, streamed
          straight into the archive without staging them on disk. Schedules opt in one by one."
      lede-label="Database dumps"
      :editing="editing"
      :can-edit="canEdit"
      :saving="saving"
      :error="saveError"
      @edit="setEditing(true)"
      @cancel="setEditing(false)"
      @save="save"
    >
      <template #view>
        <dl class="info-grid">
          <dt>Allow schedules to dump databases</dt>
          <dd>{{ enabled ? 'Allowed' : 'Blocked' }}</dd>
          <dt>Which sources</dt>
          <dd>{{ SELECTION_LABELS[selection] }}</dd>
          <dt>Kubeconfig</dt>
          <dd :class="{ mono: kubeconfigPath !== '', muted: kubeconfigPath === '' }">
            {{ kubeconfigPath === '' ? 'Agent default' : kubeconfigPath }}
          </dd>
          <dt>Dump directory</dt>
          <dd class="mono">{{ dumpDir }}</dd>
        </dl>
      </template>

      <template #edit>
        <div class="pane-rows">
          <PaneRow
            title="Allow schedules to dump databases"
            help="allow database dumps"
          >
            <template #help>
              Each schedule decides whether its own runs include them. Blocked here means no
              schedule dumps this host's databases, whatever the schedule asks for.
            </template>
            <ToggleSwitch
              v-model="enabled"
              label="Allow schedules to dump databases"
            />
          </PaneRow>

          <div class="pane-nest">
            <PaneRow
              title="Which sources"
              stack
            >
              <BaseSegmented
                v-model="selection"
                :options="SELECTION_OPTIONS"
                label="Which database sources to dump"
              />
            </PaneRow>

            <PaneRow
              title="Kubeconfig"
              label-for="db-kubeconfig"
              help="how the agent reaches the cluster"
              hint="Leave empty for the in-cluster service account or ~/.kube/config."
              stack
            >
              <template #help>
                A file on the agent's host. Give it a service account that may list pods and exec
                into them, not the cluster admin: running commands in a database pod is a privileged
                grant. The file never leaves the host.
              </template>
              <input
                id="db-kubeconfig"
                v-model="kubeconfigPath"
                class="input"
                type="text"
                placeholder="/etc/assimilate/kubeconfig"
              />
            </PaneRow>

            <PaneRow
              title="Dump directory"
              label-for="db-dump-dir"
              help="where the dumps are streamed from"
              stack
            >
              <template #help>
                An absolute path on the agent's host that holds one named pipe per database while a
                backup runs. The archive stores the dumps under it. It may not be below /dev, /proc,
                /sys or /run, which a run that dumps databases leaves out.
              </template>
              <input
                id="db-dump-dir"
                v-model="dumpDir"
                class="input"
                type="text"
                placeholder="/var/lib/assimilate/database-dumps"
              />
            </PaneRow>
          </div>
        </div>
      </template>
    </EditableSection>

    <section class="pane-section">
      <div class="pane-section-head">
        <span class="group-label group-label--lg">Sources</span>
        <button
          v-if="canEdit"
          type="button"
          class="btn btn-sm"
          :disabled="scanning"
          @click="scan"
        >
          <RefreshCw
            :size="14"
            :class="{ spinning: scanning }"
          />
          {{ scanning ? 'Scanning...' : 'Scan cluster' }}
        </button>
      </div>

      <p class="field-hint">{{ includedHint }}</p>

      <p
        v-if="scanError"
        class="form-error"
      >
        {{ scanError }}
      </p>
      <p
        v-if="rowError"
        class="form-error"
      >
        {{ rowError }}
      </p>

      <EmptyState
        v-if="sources.length === 0"
        :icon="Database"
        title="No database sources found"
        description="Scan the cluster to list its CloudNativePG clusters and annotated database pods."
      />

      <div
        v-else
        class="table-wrap"
      >
        <table
          class="data-table"
          data-testid="database-sources"
        >
          <thead>
            <tr>
              <th>Source</th>
              <th>Found as</th>
              <th>Databases</th>
              <th>Last scanned</th>
              <th>Backed up</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="source in sources"
              :key="sourceKey(source)"
            >
              <td>
                <div class="cell-mono">{{ sourceKey(source) }}</div>
                <div
                  v-if="source.pod"
                  class="cell-muted"
                >
                  {{ source.pod }} ({{ source.container }})
                </div>
                <div
                  v-if="source.last_error"
                  class="cell-muted db-error"
                >
                  {{ source.last_error }}
                </div>
              </td>
              <td>
                <span class="badge badge--neutral">{{ kindLabel(source) }}</span>
              </td>
              <td class="cell-mono">{{ databaseList(source) }}</td>
              <td class="cell-date">
                {{ source.last_scanned_at ? formatDate(source.last_scanned_at) : 'Never' }}
              </td>
              <td>
                <ToggleSwitch
                  :model-value="source.included"
                  :disabled="!canEdit || rowSaving === sourceKey(source)"
                  :label="`Dump ${sourceKey(source)}`"
                  @update:model-value="setIncluded(source, $event)"
                />
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </template>
</template>

<style scoped>
.db-error {
  color: var(--danger);
}
</style>

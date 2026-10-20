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
import {
  DEFAULT_HOST_OS_USER,
  DEFAULT_HOST_ROOT_USER,
  ENGINE_LABELS,
  KIND_LABELS,
  MYSQL_ENGINE_LABELS,
  NO_RUNTIMES,
  formPortProblem,
  runtimeForm,
  runtimeSummaries,
  runtimesFromForm,
  sourceKey,
  sourceName,
  sourcePlace,
  type RuntimeForm,
  type RuntimeSummary,
} from '../utils/databaseRuntimes'
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
  MysqlEngine,
} from '../types/generated'

/**
 * Dumping this host's databases into its backups: where the agent looks for
 * them (its Kubernetes cluster, Docker or Podman containers, PostgreSQL,
 * MySQL, MariaDB or MongoDB installed on the host itself), where it streams the dumps from, and which
 * database workloads are dumped.
 *
 * Like virtual-machine staging, the settings sit on the host and a schedule
 * only opts in. The source table is what the agent last found, so it is empty
 * until the host has been scanned once.
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
/** Why a runtime could not be searched in a scan that still found sources. */
const scanWarning = ref<string | null>(null)
const { error: rowError, run: runRow } = useAsyncAction()
/** The source whose switch is being saved, so only its row is disabled. */
const rowSaving = ref<string | null>(null)

const enabled = ref(false)
const selection = ref<DatabaseSelectionMode>('all')
const dumpDir = ref('')
const form = ref<RuntimeForm>(runtimeForm(NO_RUNTIMES))

const MYSQL_ENGINE_OPTIONS: readonly SegmentedOption<MysqlEngine>[] = [
  { value: 'mysql', label: MYSQL_ENGINE_LABELS.mysql },
  { value: 'mariadb', label: MYSQL_ENGINE_LABELS.mariadb },
]

const SELECTION_OPTIONS: readonly SegmentedOption<DatabaseSelectionMode>[] = [
  { value: 'all', label: 'All except excluded' },
  { value: 'selected', label: 'Only selected' },
]

const SELECTION_LABELS: Record<DatabaseSelectionMode, string> = {
  all: 'Every source except the ones excluded below',
  selected: 'Only the sources selected below',
}

const sources = computed<AgentDatabaseSourceResponse[]>(() => data.value?.sources ?? [])

const summaries = computed<RuntimeSummary[]>(() => runtimeSummaries(runtimesFromForm(form.value)))

const includedHint = computed<string>(() =>
  selection.value === 'all'
    ? 'Turn a source off to leave its databases out of the backup. A database deployed after the last scan is included automatically.'
    : 'Turn a source on to dump its databases. A database deployed after the last scan is left out until you select it.',
)

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
  form.value = runtimeForm(response.settings.runtimes)
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

/** What is wrong with a host port in the form; a hidden one never matters. */
function portProblem(): string | null {
  return formPortProblem(form.value)
}

async function save(): Promise<void> {
  const problem = portProblem()
  if (problem) {
    saveError.value = problem
    return
  }
  const response = await runSave(() =>
    updateAgentDatabaseBackup(
      props.agent.hostname,
      {
        enabled: enabled.value,
        selection: selection.value,
        dump_dir: dumpDir.value.trim(),
        runtimes: runtimesFromForm(form.value),
      },
      props.agent.domain,
    ),
  )
  if (!response) return
  applySettings(response)
  editing.value = false
}

/**
 * Scans the host. While the settings are being edited, the runtimes in the
 * form are searched rather than the saved ones, so new settings can be tested
 * before they are saved; the edited settings stay as they are.
 */
async function scan(): Promise<void> {
  scanWarning.value = null
  const problem = editing.value ? portProblem() : null
  if (problem) {
    scanError.value = problem
    return
  }
  const response = await runScan(() =>
    scanAgentDatabases(
      props.agent.hostname,
      editing.value ? runtimesFromForm(form.value) : null,
      props.agent.domain,
    ),
  )
  if (!response) return
  scanWarning.value = response.scan_error ?? null
  // The response carries the saved settings, so an open form keeps its edits.
  if (editing.value) data.value = response
  else applySettings(response)
}

async function setIncluded(source: AgentDatabaseSourceResponse, included: boolean): Promise<void> {
  rowSaving.value = sourceKey(source)
  const response = await runRow(() =>
    updateAgentDatabaseSource(props.agent.hostname, source, included, props.agent.domain),
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
      lede="Dump the databases this host runs, in its Kubernetes cluster, in Docker or Podman
          containers or installed on the host itself, into its backups, streamed straight into the
          archive without staging them on disk. Schedules opt in one by one."
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
          <template
            v-for="summary in summaries"
            :key="summary.label"
          >
            <dt>{{ summary.label }}</dt>
            <dd :class="{ mono: summary.mono }">{{ summary.value }}</dd>
          </template>
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
              title="Search the Kubernetes cluster"
              help="how the agent reaches the cluster"
            >
              <template #help>
                The agent lists the cluster's CloudNativePG clusters and annotated pods and runs the
                engine's own dump program inside them: pg_dump, mysqldump, mariadb-dump or
                mongodump. Give it a service account that may list pods and exec into them, not the
                cluster admin: running commands in a database pod is a privileged grant. The
                kubeconfig never leaves the host.
              </template>
              <ToggleSwitch
                v-model="form.kubernetes"
                label="Search the Kubernetes cluster"
              />
            </PaneRow>
            <div
              v-if="form.kubernetes"
              class="pane-nest"
            >
              <PaneRow
                title="Kubeconfig"
                label-for="db-kubeconfig"
                hint="Leave empty for the in-cluster service account or ~/.kube/config."
                stack
              >
                <input
                  id="db-kubeconfig"
                  v-model="form.kubeconfigPath"
                  class="input"
                  type="text"
                  placeholder="/etc/assimilate/kubeconfig"
                />
              </PaneRow>
            </div>

            <PaneRow
              title="Search Docker or Podman containers"
              help="how the agent finds containers"
            >
              <template #help>
                The agent lists the running containers labelled
                assimilate.alexmohr.github.io/database=postgresql, or mysql, mariadb or mongodb, and
                runs that database's dump program inside them through the engine's API socket.
                Access to that socket is root on the host, which the agent already is.
              </template>
              <ToggleSwitch
                v-model="form.docker"
                label="Search Docker or Podman containers"
              />
            </PaneRow>
            <div
              v-if="form.docker"
              class="pane-nest"
            >
              <PaneRow
                title="Engine socket"
                label-for="db-docker-socket"
                hint="Leave empty for $DOCKER_HOST or /var/run/docker.sock. Podman's is /run/podman/podman.sock."
                stack
              >
                <input
                  id="db-docker-socket"
                  v-model="form.dockerSocket"
                  class="input"
                  type="text"
                  placeholder="/var/run/docker.sock"
                />
              </PaneRow>
            </div>

            <PaneRow
              title="Dump PostgreSQL on this host"
              help="how the agent signs in to the host's server"
            >
              <template #help>
                The agent runs psql and pg_dump on the host as the OS user below. Over the server's
                Unix socket, peer authentication lets that user in as the role of the same name; a
                server that wants a password reads it from that user's ~/.pgpass. Assimilate stores
                no database password.
              </template>
              <ToggleSwitch
                v-model="form.host"
                label="Dump PostgreSQL on this host"
              />
            </PaneRow>
            <div
              v-if="form.host"
              class="pane-nest"
            >
              <PaneRow
                title="OS user"
                label-for="db-host-user"
                :hint="`Leave empty for ${DEFAULT_HOST_OS_USER}.`"
                stack
              >
                <input
                  id="db-host-user"
                  v-model="form.hostOsUser"
                  class="input"
                  type="text"
                  :placeholder="DEFAULT_HOST_OS_USER"
                />
              </PaneRow>
              <PaneRow
                title="Socket directory"
                label-for="db-host-socket"
                hint="Leave empty for the client's default, usually /var/run/postgresql."
                stack
              >
                <input
                  id="db-host-socket"
                  v-model="form.hostSocketDir"
                  class="input"
                  type="text"
                  placeholder="/var/run/postgresql"
                />
              </PaneRow>
              <PaneRow
                title="Port"
                label-for="db-host-port"
                hint="Leave empty for 5432."
                stack
              >
                <input
                  id="db-host-port"
                  v-model="form.hostPort"
                  class="input"
                  type="text"
                  inputmode="numeric"
                  placeholder="5432"
                />
              </PaneRow>
              <PaneRow
                title="Programs directory"
                label-for="db-host-bin"
                hint="Where pg_dump and psql live, when the server's version is not the one on the PATH."
                stack
              >
                <input
                  id="db-host-bin"
                  v-model="form.hostBinDir"
                  class="input"
                  type="text"
                  placeholder="/usr/lib/postgresql/16/bin"
                />
              </PaneRow>
            </div>

            <PaneRow
              title="Dump MySQL or MariaDB on this host"
              help="how the agent signs in to the host's MySQL or MariaDB"
            >
              <template #help>
                The agent runs the server's own client programs on the host as the OS user below.
                Over the server's socket, root is let in as the account of the same name on Debian
                and Ubuntu; a server that wants a password reads it from that user's ~/.my.cnf.
                Assimilate stores no database password.
              </template>
              <ToggleSwitch
                v-model="form.hostMysql"
                label="Dump MySQL or MariaDB on this host"
              />
            </PaneRow>
            <div
              v-if="form.hostMysql"
              class="pane-nest"
            >
              <PaneRow
                title="Server"
                stack
              >
                <BaseSegmented
                  v-model="form.hostMysqlEngine"
                  :options="MYSQL_ENGINE_OPTIONS"
                  label="Which server is installed"
                />
              </PaneRow>
              <PaneRow
                title="OS user"
                label-for="db-host-mysql-user"
                :hint="`Leave empty for ${DEFAULT_HOST_ROOT_USER}.`"
                stack
              >
                <input
                  id="db-host-mysql-user"
                  v-model="form.hostMysqlOsUser"
                  class="input"
                  type="text"
                  :placeholder="DEFAULT_HOST_ROOT_USER"
                />
              </PaneRow>
              <PaneRow
                title="Socket"
                label-for="db-host-mysql-socket"
                hint="Leave empty for the client's default, usually /run/mysqld/mysqld.sock."
                stack
              >
                <input
                  id="db-host-mysql-socket"
                  v-model="form.hostMysqlSocket"
                  class="input"
                  type="text"
                  placeholder="/run/mysqld/mysqld.sock"
                />
              </PaneRow>
              <PaneRow
                title="Port"
                label-for="db-host-mysql-port"
                hint="Leave empty to connect over the socket; a port connects over TCP to 127.0.0.1."
                stack
              >
                <input
                  id="db-host-mysql-port"
                  v-model="form.hostMysqlPort"
                  class="input"
                  type="text"
                  inputmode="numeric"
                  placeholder="3306"
                />
              </PaneRow>
              <PaneRow
                title="Programs directory"
                label-for="db-host-mysql-bin"
                hint="Where the dump and client programs live, when they are not on the PATH."
                stack
              >
                <input
                  id="db-host-mysql-bin"
                  v-model="form.hostMysqlBinDir"
                  class="input"
                  type="text"
                  placeholder="/usr/bin"
                />
              </PaneRow>
            </div>

            <PaneRow
              title="Dump MongoDB on this host"
              help="how the agent signs in to the host's MongoDB"
            >
              <template #help>
                The agent runs mongodump, mongorestore and mongosh on the host as the OS user below.
                A server with access control on is reached through a config file of the database
                tools on the host holding its uri, account and password included; only its path is
                stored here. Without one, the agent connects to 127.0.0.1 without signing in.
              </template>
              <ToggleSwitch
                v-model="form.hostMongodb"
                label="Dump MongoDB on this host"
              />
            </PaneRow>
            <div
              v-if="form.hostMongodb"
              class="pane-nest"
            >
              <PaneRow
                title="OS user"
                label-for="db-host-mongodb-user"
                :hint="`Leave empty for ${DEFAULT_HOST_ROOT_USER}.`"
                stack
              >
                <input
                  id="db-host-mongodb-user"
                  v-model="form.hostMongodbOsUser"
                  class="input"
                  type="text"
                  :placeholder="DEFAULT_HOST_ROOT_USER"
                />
              </PaneRow>
              <PaneRow
                title="Config file"
                label-for="db-host-mongodb-config"
                hint="A file only that user can read, holding uri: mongodb://user:password@127.0.0.1:27017/?authSource=admin."
                stack
              >
                <input
                  id="db-host-mongodb-config"
                  v-model="form.hostMongodbConfigFile"
                  class="input"
                  type="text"
                  placeholder="/etc/assimilate/mongodump.yaml"
                />
              </PaneRow>
              <PaneRow
                title="Port"
                label-for="db-host-mongodb-port"
                hint="Leave empty for 27017. A config file names the server itself."
                stack
              >
                <input
                  id="db-host-mongodb-port"
                  v-model="form.hostMongodbPort"
                  class="input"
                  type="text"
                  inputmode="numeric"
                  placeholder="27017"
                />
              </PaneRow>
              <PaneRow
                title="Programs directory"
                label-for="db-host-mongodb-bin"
                hint="Where mongodump, mongorestore and mongosh live, when they are not on the PATH."
                stack
              >
                <input
                  id="db-host-mongodb-bin"
                  v-model="form.hostMongodbBinDir"
                  class="input"
                  type="text"
                  placeholder="/usr/bin"
                />
              </PaneRow>
            </div>

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
          {{ scanning ? 'Scanning...' : 'Scan for databases' }}
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
        v-if="scanWarning"
        class="scan-warning"
        role="status"
      >
        Not every runtime could be searched, so its sources are kept as they were: {{ scanWarning }}
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
        description="Scan to list the CloudNativePG clusters, annotated pods, labelled containers and host server this host's settings search."
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
                <div class="cell-mono">{{ sourceName(source) }}</div>
                <div
                  v-if="sourcePlace(source)"
                  class="cell-muted"
                >
                  {{ sourcePlace(source) }}
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
                <div
                  v-if="source.engine"
                  class="cell-muted"
                >
                  {{ ENGINE_LABELS[source.engine] }}
                </div>
              </td>
              <td class="cell-mono">{{ databaseList(source) }}</td>
              <td class="cell-date">
                {{ source.last_scanned_at ? formatDate(source.last_scanned_at) : 'Never' }}
              </td>
              <td>
                <ToggleSwitch
                  :model-value="source.included"
                  :disabled="!canEdit || rowSaving === sourceKey(source)"
                  :label="`Dump ${sourceName(source)}`"
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

.scan-warning {
  color: var(--color-warning);
}
</style>

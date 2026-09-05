<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { MonitorCog, RefreshCw } from '@lucide/vue'
import { getAgentVms, scanAgentVms, updateAgentVm, updateAgentVmSnapshot } from '../api/vms'
import { badgeClass, type BadgeTone } from '../utils/badge'
import { extractError } from '../utils/error'
import { formatBytes } from '../utils/format'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import EmptyState from './EmptyState.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import VmRestoreWizard from './VmRestoreWizard.vue'
import type { AgentRow } from '../types/agent'
import type {
  AgentVmResponse,
  AgentVmSnapshotResponse,
  VmSelectionMode,
  VmSnapshotMode,
  VmState,
} from '../types/generated'

/**
 * Staging this host's libvirt domains before a backup: where they are staged,
 * how long a chain of increments may grow, and what each domain may occupy.
 *
 * The settings sit on the host rather than on a schedule because the staging
 * directory is shared by every schedule that targets it; a schedule only opts
 * in. The domain table is what the agent last reported, so it is empty until
 * the host has been scanned once.
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to scan or push to. */
  canEdit: boolean
}>()

const GIB = 1024 * 1024 * 1024

const loading = ref(true)
const loadError = ref<string | null>(null)
const data = ref<AgentVmSnapshotResponse | null>(null)

const editing = ref(false)
const saving = ref(false)
const saveError = ref<string | null>(null)

const scanning = ref(false)
const scanError = ref<string | null>(null)

const rowSaving = ref<string | null>(null)
const rowError = ref<string | null>(null)

/** The domain whose restore wizard is open, if any. */
const restoring = ref<string | null>(null)

const enabled = ref(false)
const selection = ref<VmSelectionMode>('all')
const stagingDir = ref('')

const SELECTION_OPTIONS: readonly SegmentedOption<VmSelectionMode>[] = [
  { value: 'all', label: 'All except excluded' },
  { value: 'selected', label: 'Only selected' },
]

const SELECTION_LABELS: Record<VmSelectionMode, string> = {
  all: 'Every domain except the ones excluded below',
  selected: 'Only the domains selected below',
}

/**
 * What the Include column decides, which is the opposite thing in each mode.
 * Under `all` turning a domain off is what takes it out of the backup; under
 * `selected` turning one on is what puts it in, and a domain nobody has
 * touched stays out.
 */
const includeHint = computed<string>(() =>
  selection.value === 'all'
    ? 'Turn a domain off to leave it out of the backup. A machine created after the last scan is included automatically.'
    : 'Turn a domain on to back it up. A machine created after the last scan is left alone until you select it.',
)
const fullInterval = ref(7)
const timeoutSeconds = ref(1800)
const defaultLimitGib = ref(0)

const vms = computed<AgentVmResponse[]>(() => data.value?.vms ?? [])

/** Bytes to whole GiB for the form, which is the unit operators think in. */
function toGib(bytes: number): number {
  return Math.round(bytes / GIB)
}

function fromGib(gib: number): number {
  return Math.max(0, Math.round(gib)) * GIB
}

function limitLabel(vm: AgentVmResponse): string {
  if (vm.effective_limit_bytes === 0) return 'No limit'
  return formatBytes(vm.effective_limit_bytes)
}

/**
 * Share of its limit a domain uses, as a percentage capped at 100. Only called
 * for a domain that has a limit: the bar it fills is not rendered at all for an
 * unlimited one, which is what keeps the divisor here above zero.
 */
function usedPercent(vm: AgentVmResponse): number {
  return Math.min(100, Math.round((vm.staged_bytes / vm.effective_limit_bytes) * 100))
}

function usageTone(vm: AgentVmResponse): string {
  const percent = usedPercent(vm)
  if (percent >= 95) return 'progress-bar--danger'
  if (percent >= 80) return 'progress-bar--warning'
  return ''
}

const MODE_LABELS: Record<VmSnapshotMode, string> = {
  incremental: 'Incremental',
  full_copy: 'Full copy',
  offline_copy: 'Offline copy',
  excluded: 'Excluded',
  unknown: 'Not scanned',
}

function modeLabel(mode: VmSnapshotMode): string {
  return MODE_LABELS[mode]
}

const MODE_TONES: Record<VmSnapshotMode, BadgeTone> = {
  incremental: 'info',
  full_copy: 'neutral',
  offline_copy: 'neutral',
  excluded: 'neutral',
  unknown: 'neutral',
}

function modeBadge(mode: VmSnapshotMode): string {
  return badgeClass(MODE_TONES[mode])
}

const STATE_LABELS: Record<VmState, string> = {
  running: 'Running',
  paused: 'Paused',
  shut_off: 'Shut off',
  suspended: 'Suspended',
  unknown: 'Unknown',
}

function stateLabel(state: VmState): string {
  return STATE_LABELS[state]
}

const STATE_TONES: Record<VmState, BadgeTone> = {
  running: 'success',
  paused: 'warning',
  suspended: 'warning',
  shut_off: 'neutral',
  unknown: 'neutral',
}

function stateBadge(state: VmState): string {
  return badgeClass(STATE_TONES[state])
}

function applyResponse(response: AgentVmSnapshotResponse): void {
  data.value = response
  enabled.value = response.settings.enabled
  selection.value = response.settings.selection
  stagingDir.value = response.settings.staging_dir
  fullInterval.value = response.settings.full_interval
  timeoutSeconds.value = response.settings.timeout_seconds
  defaultLimitGib.value = toGib(response.settings.default_limit_bytes)
}

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    applyResponse(await getAgentVms(props.agent.hostname, props.agent.domain))
  } catch (e: unknown) {
    loadError.value = extractError(e)
  } finally {
    loading.value = false
  }
}

function startEdit(): void {
  if (data.value) applyResponse(data.value)
  saveError.value = null
  editing.value = true
}

/**
 * Puts the loaded settings back before leaving the form. The summary renders
 * the same refs the form binds to, so without this an abandoned edit stays on
 * screen as though it had been saved.
 */
function cancelEdit(): void {
  if (data.value) applyResponse(data.value)
  saveError.value = null
  editing.value = false
}

async function save(): Promise<void> {
  saving.value = true
  saveError.value = null
  try {
    applyResponse(
      await updateAgentVmSnapshot(
        props.agent.hostname,
        {
          enabled: enabled.value,
          selection: selection.value,
          staging_dir: stagingDir.value.trim(),
          full_interval: fullInterval.value,
          timeout_seconds: timeoutSeconds.value,
          default_limit_bytes: fromGib(defaultLimitGib.value),
        },
        props.agent.domain,
      ),
    )
    editing.value = false
  } catch (e: unknown) {
    saveError.value = extractError(e)
  } finally {
    saving.value = false
  }
}

async function scan(): Promise<void> {
  scanning.value = true
  scanError.value = null
  try {
    applyResponse(await scanAgentVms(props.agent.hostname, props.agent.domain))
  } catch (e: unknown) {
    scanError.value = extractError(e)
  } finally {
    scanning.value = false
  }
}

/**
 * Saves one domain's row. `included` is sent only when the operator actually
 * decided it, because `vm.included` is the host's mode already resolved for
 * an undecided domain - echoing it back on a limit edit would silently
 * promote "nobody has decided" into "explicitly included".
 */
async function saveVm(
  vm: AgentVmResponse,
  included: boolean | undefined,
  limitGib: string,
): Promise<void> {
  rowSaving.value = vm.name
  rowError.value = null
  const trimmed = limitGib.trim()
  try {
    applyResponse(
      await updateAgentVm(
        props.agent.hostname,
        vm.name,
        {
          ...(included === undefined ? {} : { included }),
          limit_bytes: trimmed === '' ? null : fromGib(Number(trimmed)),
        },
        props.agent.domain,
      ),
    )
  } catch (e: unknown) {
    rowError.value = extractError(e)
  } finally {
    rowSaving.value = null
  }
}

function limitInput(vm: AgentVmResponse): string {
  return vm.limit_bytes === null ? '' : String(toGib(vm.limit_bytes))
}

function onLimitChange(vm: AgentVmResponse, event: Event): void {
  const value = (event.target as HTMLInputElement).value
  void saveVm(vm, undefined, value)
}

function onIncludedChange(vm: AgentVmResponse, included: boolean): void {
  void saveVm(vm, included, limitInput(vm))
}

function restored(): void {
  restoring.value = null
  void load()
}

onMounted(load)
</script>

<template>
  <!-- No wrapper element: SettingsRail already puts this card's content inside
       the `.settings-pane` column, and nesting a second one both duplicates the
       class on the page and re-applies the column gap a level too deep. -->
  <BaseSpinner
    v-if="loading"
    label="Loading virtual machines"
  />
  <p
    v-else-if="loadError"
    class="form-error"
  >
    {{ loadError }}
  </p>

  <template v-else>
    <EditableSection
      lede="Stage this host's virtual machines into a directory before a backup runs, so borg
          picks them up as ordinary files. Schedules opt in one by one."
      :editing="editing"
      :can-edit="canEdit"
      :saving="saving"
      :error="saveError"
      @edit="startEdit"
      @cancel="cancelEdit"
      @save="save"
    >
      <template #view>
        <section
          class="pane-section"
          style="border-top: none; padding-top: 0"
        >
          <div class="pane-section-head">
            <span class="group-label group-label--lg">Staging</span>
          </div>
          <dl class="info-grid">
            <dt>Stage virtual machines</dt>
            <dd>{{ enabled ? 'Enabled' : 'Disabled' }}</dd>
            <dt>Which domains</dt>
            <dd>{{ SELECTION_LABELS[selection] }}</dd>
            <dt>Staging directory</dt>
            <dd class="mono">{{ stagingDir }}</dd>
            <dt>New full image after</dt>
            <dd>{{ fullInterval }} increments</dd>
            <dt>Snapshot timeout</dt>
            <dd>{{ timeoutSeconds }} seconds per domain</dd>
            <dt>Default limit per domain</dt>
            <dd>{{ defaultLimitGib === 0 ? 'No limit' : `${defaultLimitGib} GiB` }}</dd>
          </dl>
        </section>
      </template>

      <template #edit>
        <section
          class="pane-section"
          style="border-top: none; padding-top: 0"
        >
          <div class="pane-section-head">
            <span class="group-label group-label--lg">Staging</span>
          </div>

          <div class="field field-inline">
            <div class="field-body">
              <p class="field-title">Stage virtual machines</p>
              <p class="field-hint">
                When off, a schedule that opts in stages nothing on this host.
              </p>
            </div>
            <ToggleSwitch
              v-model="enabled"
              label="Stage virtual machines"
            />
          </div>

          <div class="field">
            <span class="field-label">Which domains</span>
            <BaseSegmented
              v-model="selection"
              :options="SELECTION_OPTIONS"
              label="Which domains to stage"
            />
            <span class="field-hint">{{ includeHint }}</span>
          </div>

          <div class="field">
            <label
              class="field-label"
              for="vm-staging-dir"
            >
              Staging directory
            </label>
            <input
              id="vm-staging-dir"
              v-model="stagingDir"
              class="input"
              type="text"
              placeholder="/home/virt/backups"
            />
            <span class="field-hint">
              An absolute path with one subdirectory per domain. It must be writable by the user
              QEMU runs as, and it joins the sources of every schedule that opts in.
            </span>
          </div>

          <div class="field-row">
            <div class="field field-narrow">
              <label
                class="field-label"
                for="vm-full-interval"
              >
                New full image after
              </label>
              <input
                id="vm-full-interval"
                v-model.number="fullInterval"
                class="input"
                type="number"
                min="1"
              />
              <span class="field-hint">Increments per chain.</span>
            </div>
            <div class="field field-narrow">
              <label
                class="field-label"
                for="vm-timeout"
              >
                Snapshot timeout
              </label>
              <input
                id="vm-timeout"
                v-model.number="timeoutSeconds"
                class="input"
                type="number"
                min="1"
              />
              <span class="field-hint">Seconds, per domain.</span>
            </div>
            <div class="field field-narrow">
              <label
                class="field-label"
                for="vm-default-limit"
              >
                Default limit per domain
              </label>
              <input
                id="vm-default-limit"
                v-model.number="defaultLimitGib"
                class="input"
                type="number"
                min="0"
              />
              <span class="field-hint">GiB. 0 means no limit.</span>
            </div>
          </div>
        </section>
      </template>
    </EditableSection>

    <section class="pane-section">
      <div class="pane-section-head">
        <span class="group-label group-label--lg">Domains</span>
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
          {{ scanning ? 'Scanning...' : 'Rescan host' }}
        </button>
      </div>

      <p class="field-hint">{{ includeHint }}</p>

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
        v-if="vms.length === 0"
        :icon="MonitorCog"
        title="No domains reported"
        description="Rescan the host to list the libvirt domains it has."
      />

      <div
        v-else
        class="table-wrap"
      >
        <table class="data-table">
          <thead>
            <tr>
              <th>Domain</th>
              <th>State</th>
              <th>Mode</th>
              <th>Staged size</th>
              <th>Limit (GiB)</th>
              <th>Include</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="vm in vms"
              :key="vm.name"
            >
              <td>
                <div class="cell-mono">{{ vm.name }}</div>
                <div class="cell-muted">
                  {{ vm.disk_count }} disk<span v-if="vm.disk_count !== 1">s</span>
                  <span v-if="vm.chain_length > 0">
                    &middot; full + {{ vm.chain_length }} increments
                  </span>
                </div>
                <div
                  v-if="vm.last_error"
                  class="cell-muted vm-error"
                >
                  {{ vm.last_error }}
                </div>
              </td>
              <td>
                <span
                  class="badge"
                  :class="stateBadge(vm.state)"
                >
                  {{ stateLabel(vm.state) }}
                </span>
              </td>
              <td>
                <span
                  class="badge"
                  :class="modeBadge(vm.mode)"
                >
                  {{ modeLabel(vm.mode) }}
                </span>
              </td>
              <td>
                <div class="cell-size">
                  {{ formatBytes(vm.staged_bytes) }} of {{ limitLabel(vm) }}
                </div>
                <div
                  v-if="vm.effective_limit_bytes > 0"
                  class="progress-track vm-usage"
                >
                  <div
                    class="progress-bar"
                    :class="usageTone(vm)"
                    :style="{ width: `${usedPercent(vm)}%` }"
                  />
                </div>
              </td>
              <td>
                <input
                  class="input input-sm vm-limit"
                  type="number"
                  min="0"
                  :value="limitInput(vm)"
                  :disabled="!canEdit || rowSaving === vm.name"
                  :aria-label="`Limit for ${vm.name} in GiB`"
                  placeholder="Default"
                  @change="onLimitChange(vm, $event)"
                />
                <div class="cell-muted">
                  {{ vm.limit_bytes === null ? 'Host default' : 'Overridden' }}
                </div>
              </td>
              <td>
                <ToggleSwitch
                  :model-value="vm.included"
                  :disabled="!canEdit || rowSaving === vm.name"
                  :label="`Include ${vm.name}`"
                  @update:model-value="onIncludedChange(vm, $event)"
                />
              </td>
              <td class="td-action">
                <button
                  v-if="canEdit"
                  type="button"
                  class="btn btn-sm"
                  @click="restoring = vm.name"
                >
                  Restore
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <VmRestoreWizard
      v-if="restoring"
      :open="restoring !== null"
      :agent="agent"
      :domain-name="restoring"
      :staging-dir="stagingDir"
      @close="restoring = null"
      @restored="restored"
    />
  </template>
</template>

<style scoped>
.vm-usage {
  margin-top: var(--space-2);
}

.vm-limit {
  width: 5.5rem;
  text-align: right;
}

.vm-error {
  color: var(--danger);
}
</style>

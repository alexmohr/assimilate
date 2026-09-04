<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check } from '@lucide/vue'
import { restoreArchiveFiles } from '../api/archives'
import { listAgentReports } from '../api/agents'
import { buildAgentVm } from '../api/vms'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import BaseModal from './BaseModal.vue'
import BaseSpinner from './BaseSpinner.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'
import type { ReportRow } from '../types/report'
import type { VmBuildAction, VmBuildOutcome } from '../types/generated'

/**
 * Restoring a staged domain, in the two stages it actually happens in: borg
 * puts the staged files back on disk, then the agent merges the chain and
 * defines the domain out of them.
 *
 * Stage two reads whatever directory it is pointed at, so an operator who
 * restored the files earlier can skip stage one entirely.
 */
const props = defineProps<{
  open: boolean
  agent: AgentRow
  /** The domain to restore, as the host reported it. */
  domainName: string
  /** The host's staging directory, which is where the files sit in the archive. */
  stagingDir: string
}>()

const emit = defineEmits<{ close: []; restored: [outcome: VmBuildOutcome] }>()

const TOTAL_STEPS = 3

const step = ref(1)
const reports = ref<ReportRow[]>([])
const loadingReports = ref(false)
/**
 * The archive stage one restores from, with the repository holding it. One
 * ref rather than two: an archive name is only ever meaningful together with
 * its repository, and keeping them apart would let the two drift into a state
 * `run()` would have to guard against.
 */
const selected = ref<{ repoId: number; archive: string } | null>(null)

const restoreFiles = ref(true)
const workingDir = ref('/var/tmp/assimilate-restore')
const sourceDirInput = ref('')

const restoreAs = ref('')
const imageDir = ref('/var/lib/libvirt/images')
const action = ref<VmBuildAction>('define')

const running = ref(false)
const stageOneDone = ref(false)
const stageTwoDone = ref(false)
const outcome = ref<VmBuildOutcome | null>(null)
const error = ref<string | null>(null)

/** A run that produced an archive, so it can be restored from. */
type RestorableReport = ReportRow & { archive_name: string }

/**
 * Archives of this host, newest first: each successful run is a point in time.
 * Narrowed as it is filtered, so what the list hands to `pickArchive` carries
 * the archive name in its type rather than needing a second check for one the
 * filter has already ruled out.
 */
const restorable = computed<RestorableReport[]>(() =>
  reports.value.filter((report): report is RestorableReport => report.archive_name !== null),
)

/**
 * Where stage one leaves the files. borg recreates the archived path below the
 * directory it extracts into, so the domain lands under its staging path.
 */
const restoredPath = computed<string>(() => {
  const staged = `${props.stagingDir.replace(/^\/+/, '')}/${props.domainName}`
  return `${workingDir.value.replace(/\/+$/, '')}/${staged}`
})

const sourceDir = computed<string>(() =>
  restoreFiles.value ? restoredPath.value : sourceDirInput.value.trim(),
)

const canProceed = computed<boolean>(() => {
  if (step.value === 1) return !restoreFiles.value || selected.value !== null
  if (step.value === 2) return sourceDir.value.startsWith('/')
  return restoreAs.value.trim().length > 0 && imageDir.value.trim().startsWith('/')
})

async function loadReports(): Promise<void> {
  loadingReports.value = true
  error.value = null
  try {
    reports.value = await listAgentReports(props.agent.hostname, undefined, props.agent.domain)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    loadingReports.value = false
  }
}

function reset(): void {
  step.value = 1
  selected.value = null
  restoreFiles.value = true
  workingDir.value = '/var/tmp/assimilate-restore'
  sourceDirInput.value = ''
  restoreAs.value = `${props.domainName}-restored`
  imageDir.value = '/var/lib/libvirt/images'
  action.value = 'define'
  running.value = false
  stageOneDone.value = false
  stageTwoDone.value = false
  outcome.value = null
  error.value = null
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    reset()
    void loadReports()
  },
  { immediate: true },
)

function pickArchive(report: RestorableReport): void {
  selected.value = { repoId: report.repo_id, archive: report.archive_name }
}

function next(): void {
  if (step.value < TOTAL_STEPS) step.value += 1
}

function back(): void {
  if (step.value > 1) step.value -= 1
}

async function run(): Promise<void> {
  running.value = true
  error.value = null
  stageOneDone.value = false
  stageTwoDone.value = false

  try {
    // `canProceed` will not let step one be left without an archive picked,
    // so a restore that reaches here always has one.
    if (restoreFiles.value && selected.value !== null) {
      const response = await restoreArchiveFiles(selected.value.repoId, selected.value.archive, {
        paths: [`${props.stagingDir.replace(/^\/+/, '')}/${props.domainName}`],
        target_path: workingDir.value.trim(),
        hostname: props.agent.hostname,
      })
      if (!response.success) {
        throw new Error(response.error_message ?? 'The files could not be restored')
      }
    }
    stageOneDone.value = true

    outcome.value = await buildAgentVm(
      props.agent.hostname,
      {
        source_dir: sourceDir.value,
        name: restoreAs.value.trim(),
        image_dir: imageDir.value.trim(),
        action: action.value,
      },
      props.agent.domain,
    )
    stageTwoDone.value = true
    emit('restored', outcome.value)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    running.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Restore virtual machine"
    size="lg"
    @close="emit('close')"
  >
    <div
      v-if="!running && !outcome"
      class="steps-indicator"
    >
      <div
        v-for="s in TOTAL_STEPS"
        :key="s"
        class="step-dot"
        :class="{ active: s === step, completed: s < step }"
      >
        {{ s }}
      </div>
    </div>

    <!-- Running, and afterwards: the two stages and what they produced. -->
    <div v-if="running || outcome">
      <ol class="stage-list">
        <li class="stage">
          <BaseSpinner
            v-if="running && !stageOneDone"
            size="sm"
          />
          <Check
            v-else-if="stageOneDone"
            :size="14"
            class="stage-done"
          />
          <span
            v-else
            class="stage-pending"
          />
          <span>
            <span class="stage-label">Restore the files with borg</span>
            <span class="field-hint">
              {{ restoreFiles ? `Into ${workingDir}` : 'Skipped, the files are already on disk' }}
            </span>
          </span>
        </li>
        <li class="stage">
          <BaseSpinner
            v-if="running && stageOneDone && !stageTwoDone"
            size="sm"
          />
          <Check
            v-else-if="stageTwoDone"
            :size="14"
            class="stage-done"
          />
          <span
            v-else
            class="stage-pending"
          />
          <span>
            <span class="stage-label">Merge the chain and define the domain</span>
            <span class="field-hint">{{ sourceDir }}</span>
          </span>
        </li>
      </ol>

      <dl
        v-if="outcome"
        class="info-grid"
      >
        <dt>Domain</dt>
        <dd class="mono">{{ outcome.name }}</dd>
        <dt>Increments merged</dt>
        <dd>{{ outcome.merged_increments }}</dd>
        <dt>Images</dt>
        <dd class="mono">{{ outcome.images.join(', ') }}</dd>
        <dt>Defined</dt>
        <dd>
          {{
            outcome.defined
              ? outcome.started
                ? 'Defined and started'
                : 'Defined, shut off'
              : 'Files only'
          }}
        </dd>
      </dl>
    </div>

    <!-- Step 1: which point in time. -->
    <div
      v-else-if="step === 1"
      class="step-content wizard-step"
    >
      <p class="field-hint">
        Every archive holds the whole chain as it stood that night, so an archive is a point in
        time. Restoring one never needs a second archive.
      </p>

      <div class="field field-inline">
        <div class="field-body">
          <p class="field-title">Restore the files from an archive</p>
          <p class="field-hint">Turn this off to build from files already on disk.</p>
        </div>
        <ToggleSwitch
          v-model="restoreFiles"
          label="Restore the files from an archive"
        />
      </div>

      <BaseSpinner
        v-if="loadingReports"
        label="Loading archives"
      />
      <p
        v-else-if="restoreFiles && restorable.length === 0"
        class="state-msg"
      >
        This host has no archives to restore from.
      </p>
      <div
        v-else-if="restoreFiles"
        class="archive-list"
      >
        <label
          v-for="report in restorable"
          :key="`${report.repo_id}-${report.archive_name}`"
          class="archive-option"
          :class="{ 'is-selected': selected?.archive === report.archive_name }"
        >
          <input
            type="radio"
            name="vm-restore-archive"
            :value="report.archive_name"
            :checked="selected?.archive === report.archive_name"
            @change="pickArchive(report)"
          />
          <span class="option-text">
            <span class="mono">{{ report.archive_name }}</span>
            <span class="field-hint">
              {{ formatDate(report.started_at) }} &middot; {{ report.repo_name ?? 'unknown repo' }}
            </span>
          </span>
        </label>
      </div>
    </div>

    <!-- Step 2: stage one, where the files land. -->
    <div
      v-else-if="step === 2"
      class="step-content wizard-step"
    >
      <p class="field-hint">
        This is the ordinary agent-side restore: borg runs on {{ agent.hostname }} and no data
        passes through the server. Stage two reads the result and leaves it alone, so a failed build
        can be retried without fetching from borg again.
      </p>

      <div
        v-if="restoreFiles"
        class="field"
      >
        <label
          class="field-label"
          for="vm-restore-workdir"
        >
          Restore the files to
        </label>
        <input
          id="vm-restore-workdir"
          v-model="workingDir"
          class="input"
          type="text"
        />
        <span class="field-hint">The domain lands at {{ restoredPath }}.</span>
      </div>

      <div
        v-else
        class="field"
      >
        <label
          class="field-label"
          for="vm-restore-source"
        >
          Directory holding the restored domain
        </label>
        <input
          id="vm-restore-source"
          v-model="sourceDirInput"
          class="input"
          type="text"
          placeholder="/var/tmp/assimilate-restore/srv/vm-staging/web01"
        />
        <span class="field-hint">
          It must hold the domain's chain.txt, its images and its definition.
        </span>
      </div>
    </div>

    <!-- Step 3: stage two, what to build. -->
    <div
      v-else
      class="step-content wizard-step"
    >
      <div class="field">
        <label
          class="field-label"
          for="vm-restore-name"
        >
          Restore as
        </label>
        <input
          id="vm-restore-name"
          v-model="restoreAs"
          class="input"
          type="text"
        />
        <span class="field-hint">
          A new name, so the restored domain can be defined beside the one it came from. It gets a
          fresh UUID and keeps its MAC address, so do not start both at once.
        </span>
      </div>

      <div class="field">
        <label
          class="field-label"
          for="vm-restore-image-dir"
        >
          Image directory
        </label>
        <input
          id="vm-restore-image-dir"
          v-model="imageDir"
          class="input"
          type="text"
        />
      </div>

      <div class="field">
        <span class="field-label">When the images are in place</span>
        <label class="radio-option">
          <input
            v-model="action"
            type="radio"
            value="files_only"
          />
          <span>Leave the images only</span>
        </label>
        <label class="radio-option">
          <input
            v-model="action"
            type="radio"
            value="define"
          />
          <span>Define the domain, leave it shut off</span>
        </label>
        <label class="radio-option">
          <input
            v-model="action"
            type="radio"
            value="define_and_start"
          />
          <span>Define the domain and start it</span>
        </label>
      </div>
    </div>

    <template #footer>
      <p
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </p>
      <div class="modal-actions">
        <button
          v-if="outcome"
          type="button"
          class="btn btn-primary"
          @click="emit('close')"
        >
          Done
        </button>
        <template v-else>
          <button
            type="button"
            class="btn"
            :disabled="running || step === 1"
            @click="back"
          >
            Back
          </button>
          <button
            v-if="step < TOTAL_STEPS"
            type="button"
            class="btn btn-primary"
            :disabled="!canProceed"
            @click="next"
          >
            Next
          </button>
          <button
            v-else
            type="button"
            class="btn btn-primary"
            :disabled="!canProceed || running"
            @click="run"
          >
            {{ running ? 'Restoring...' : 'Restore' }}
          </button>
        </template>
      </div>
    </template>
  </BaseModal>
</template>

<style scoped>
.wizard-step {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.option-text {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
}

.archive-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  max-height: 16rem;
  overflow-y: auto;
}

.archive-option,
.radio-option {
  display: flex;
  align-items: flex-start;
  gap: var(--space-4);
  padding: var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.archive-option.is-selected {
  border-color: var(--accent);
  background: var(--accent-subtle);
}

.stage-list {
  list-style: none;
  margin: 0 0 var(--space-6);
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.stage {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
}

.stage-label {
  display: block;
  font-weight: 550;
}

.stage-done {
  color: var(--success);
}

.stage-pending {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 1px dashed var(--border);
}

.modal-actions {
  display: flex;
  gap: var(--space-4);
  justify-content: flex-end;
}
</style>

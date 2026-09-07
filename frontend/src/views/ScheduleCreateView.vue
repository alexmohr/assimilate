<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Check, ChevronLeft, ChevronRight, TriangleAlert } from '@lucide/vue'
import { createSchedule, type ScheduleRepoTarget } from '../api/schedules'
import { listAgents } from '../api/agents'
import { listRepos } from '../api/repos'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import {
  agentOverridePayload,
  repoTargetsProblem,
  scheduleFormPayload,
} from '../utils/schedulePayload'
import { parseLines } from '../utils/validation'
import { useAsyncAction } from '../composables/useAsyncAction'
import AgentMultiSelect from '../components/AgentMultiSelect.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import CronBuilder from '../components/CronBuilder.vue'
import EmptyState from '../components/EmptyState.vue'
import ScheduleAdvancedTab from '../components/ScheduleAdvancedTab.vue'
import ScheduleRepoTargets from '../components/ScheduleRepoTargets.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { DEFAULT_SCHEDULE_FORM_STATE } from '../types/scheduleForm'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'
import type { ScheduleType } from '../types/schedule'
import { Database } from '@lucide/vue'

/**
 * Creating a schedule, as a wizard.
 *
 * The create form used to be the detail page's Settings tab with the fields
 * that only apply to a new schedule switched on: every section was reachable
 * at once and "Create schedule" was always clickable, so the usual way to
 * learn a repository was mandatory was to press it and read the error. Here
 * each step states what it still needs, the next step unlocks only once that
 * is answered, and the create action exists on the last step alone.
 */
type StepId = 'basics' | 'sources' | 'targets' | 'timing' | 'retention' | 'advanced' | 'review'

interface Step {
  id: StepId
  label: string
  sub: string
}

const router = useRouter()
const route = useRoute()

const agents = ref<AgentRow[]>([])
const repos = ref<Repo[]>([])
const { loading, error, run } = useAsyncAction('Failed to load agents and repositories.')

const form = ref<ScheduleFormState>({ ...DEFAULT_SCHEDULE_FORM_STATE })
const selectedAgentIds = ref<number[]>([])
const repoTargets = ref<ScheduleRepoTarget[]>([])
const selectedType = ref<ScheduleType>('backup')
const onFailure = ref<'stop' | 'continue'>('stop')
const agentOverrides = ref<ScheduleAgentOverrides>({
  usePerHostExcludes: false,
  perHostExcludes: {},
  usePerHostFileChangePatterns: false,
  perHostFileChangePatterns: {},
  usePerAgentCmds: false,
  perAgentPreCmds: {},
  perAgentPostCmds: {},
})

const stepIndex = ref(0)
const visited = ref<Set<StepId>>(new Set(['basics']))
const submitting = ref(false)
const submitError = ref<string | null>(null)

const isBackup = computed(() => selectedType.value === 'backup')

/** Retention and the borg options only mean anything for a backup schedule. */
const steps = computed<Step[]>(() => [
  { id: 'basics', label: 'Basics', sub: 'Name and type' },
  { id: 'sources', label: 'Sources', sub: 'Hosts and paths' },
  { id: 'targets', label: 'Targets', sub: 'Where it is written' },
  { id: 'timing', label: 'Timing', sub: 'When it runs' },
  ...(isBackup.value
    ? ([
        { id: 'retention', label: 'Retention', sub: 'What is kept' },
        { id: 'advanced', label: 'Advanced', sub: 'Patterns and hooks' },
      ] as const)
    : []),
  { id: 'review', label: 'Review', sub: 'Create schedule' },
])

const currentStep = computed<Step>(
  () => steps.value[Math.min(stepIndex.value, steps.value.length - 1)],
)
const isLastStep = computed(() => stepIndex.value >= steps.value.length - 1)

const agentMap = computed(() => new Map(agents.value.map((a) => [a.id, a])))

function agentLabel(id: number): string {
  const agent = agentMap.value.get(id)
  return agent ? (agent.display_name ?? agent.hostname) : `#${id}`
}

const cronFieldCount = computed(() => form.value.cron_expression.trim().split(/\s+/).length)

/** What a step still needs before it counts as answered. */
function blockers(step: StepId): string[] {
  const missing: string[] = []
  if (step === 'basics' && form.value.name.trim().length === 0) {
    missing.push('a name')
  }
  if (step === 'sources' && selectedAgentIds.value.length === 0) {
    missing.push('at least one host')
  }
  if (step === 'targets') {
    const problem = repoTargetsProblem(repoTargets.value)
    if (problem) missing.push(problem)
  }
  if (step === 'timing' && cronFieldCount.value !== 5) {
    missing.push('a five-field cron expression')
  }
  return missing
}

const currentBlockers = computed(() => blockers(currentStep.value.id))

const allBlockers = computed(() =>
  steps.value.flatMap((step) => blockers(step.id).map((text) => ({ step, text }))),
)

type StepState = 'current' | 'done' | 'todo' | 'blank'

function stepState(step: Step, index: number): StepState {
  if (index === stepIndex.value) return 'current'
  if (blockers(step.id).length === 0) return 'done'
  return visited.value.has(step.id) ? 'todo' : 'blank'
}

function goTo(index: number): void {
  visited.value.add(currentStep.value.id)
  stepIndex.value = Math.max(0, Math.min(index, steps.value.length - 1))
  visited.value.add(currentStep.value.id)
}

function next(): void {
  goTo(stepIndex.value + 1)
}

function back(): void {
  goTo(stepIndex.value - 1)
}

function stepIndexOf(id: StepId): number {
  return steps.value.findIndex((s) => s.id === id)
}

const cronSummary = computed(
  () => cronToHuman(form.value.cron_expression) ?? form.value.cron_expression,
)

const requiredTargetCount = computed(() => repoTargets.value.filter((t) => t.required).length)

/**
 * `on_failure` governs a failing *host* on a multi-host schedule and a failing
 * *required target* on a multi-target one, so a single host writing to two
 * repositories - the case this feature exists for - needs it just as much.
 * It lives on the Targets step because that is the first point where both
 * counts are known; on Sources the second target has not been added yet.
 */
const showOnFailure = computed(
  () => selectedAgentIds.value.length > 1 || repoTargets.value.length > 1,
)

const SCHEDULE_TYPE_LABELS: Record<ScheduleType, string> = {
  backup: 'Backup',
  check: 'Integrity check',
  verify: 'Verify (extract dry-run)',
}

const scheduleTypeLabel = computed<string>(() => SCHEDULE_TYPE_LABELS[selectedType.value])

function repoName(repoId: number): string {
  return repos.value.find((r) => r.id === repoId)?.name ?? `repo #${repoId}`
}

async function load(): Promise<void> {
  await run(async () => {
    const [agentRows, repoRows] = await Promise.all([listAgents(), listRepos()])
    agents.value = agentRows
    repos.value = repoRows
    const queryAgentId = Number(route.query.agent_id)
    if (queryAgentId && agentRows.some((a) => a.id === queryAgentId)) {
      selectedAgentIds.value = [queryAgentId]
    }
    if (repoRows.length > 0) {
      repoTargets.value = [{ repo_id: repoRows[0].id, required: true }]
    }
  })
}

onMounted(load)

async function submit(): Promise<void> {
  if (allBlockers.value.length > 0) return
  submitting.value = true
  submitError.value = null
  try {
    const created = await createSchedule({
      ...scheduleFormPayload(form.value),
      ...agentOverridePayload(agentOverrides.value, selectedAgentIds.value),
      agent_ids: selectedAgentIds.value,
      repo_id: repoTargets.value[0].repo_id,
      repo_targets: repoTargets.value,
      schedule_type: selectedType.value,
      on_failure: onFailure.value,
    })
    await router.push(`/schedules/${created.id}`)
  } catch (e: unknown) {
    submitError.value = extractError(e, 'Failed to create schedule')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="schedule-create-view">
    <div class="detail-breadcrumb">
      <RouterLink
        to="/schedules"
        class="crumb-link"
        >Schedules</RouterLink
      >
      <span class="crumb-sep">/</span>
      <span class="crumb-current">New</span>
    </div>

    <div class="page-header">
      <h1 class="page-title">New Schedule</h1>
    </div>
    <p class="page-description">
      Each step says what it still needs, and the schedule is only created once every one of them is
      answered.
    </p>

    <div
      v-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />

    <EmptyState
      v-else-if="repos.length === 0"
      :icon="Database"
      title="No repositories yet"
      description="A schedule needs somewhere to write. Create a repository first."
      action="New repository"
      @action="router.push('/repos')"
    />

    <div
      v-else
      class="panel wizard"
    >
      <nav
        class="wizard-rail"
        aria-label="Schedule steps"
      >
        <button
          v-for="(step, index) in steps"
          :key="step.id"
          type="button"
          class="wizard-step"
          :class="{ 'wizard-step--current': index === stepIndex }"
          :aria-current="index === stepIndex"
          @click="goTo(index)"
        >
          <span
            class="wizard-num"
            :class="`wizard-num--${stepState(step, index)}`"
          >
            <Check
              v-if="stepState(step, index) === 'done'"
              :size="12"
            />
            <template v-else>{{ index + 1 }}</template>
          </span>
          <span class="wizard-step-text">
            <span>{{ step.label }}</span>
            <span class="wizard-step-sub">{{ step.sub }}</span>
          </span>
        </button>
      </nav>

      <section class="wizard-pane">
        <template v-if="currentStep.id === 'basics'">
          <p class="pane-lede">
            What this schedule is called and what it does. The type decides which steps follow.
          </p>
          <div class="field">
            <label
              class="field-label"
              for="schedule-name"
            >
              Name
              <span class="required">*</span>
            </label>
            <input
              id="schedule-name"
              v-model="form.name"
              class="input"
              placeholder="e.g. Nightly production backup"
            />
            <span class="field-hint">Shown on the schedules list and in every run report.</span>
          </div>
          <div class="field">
            <label
              class="field-label"
              for="schedule-type"
              >Type</label
            >
            <select
              id="schedule-type"
              v-model="selectedType"
              class="input"
            >
              <option value="backup">Backup</option>
              <option value="check">Integrity check</option>
              <option value="verify">Verify (extract dry-run)</option>
            </select>
            <span class="field-hint">
              Backup creates archives; Check validates repo integrity; Verify tests extractability.
            </span>
          </div>
          <div class="field field-inline">
            <label class="field-label">Enabled</label>
            <ToggleSwitch v-model="form.enabled" />
          </div>
          <span class="field-hint">
            Leave this off to create the schedule paused and run it by hand first.
          </span>
        </template>

        <template v-else-if="currentStep.id === 'sources'">
          <p class="pane-lede">The machines this schedule runs on and what it reads from them.</p>
          <div class="field">
            <label class="field-label">
              Hosts
              <span class="required">*</span>
            </label>
            <AgentMultiSelect
              v-model="selectedAgentIds"
              :agents="agents"
            />
            <span class="field-hint">
              {{
                selectedAgentIds.length === 0
                  ? 'The agents that will execute this schedule.'
                  : selectedAgentIds.map(agentLabel).join(', ')
              }}
            </span>
          </div>
          <div
            v-if="isBackup"
            class="field"
          >
            <label
              class="field-label"
              for="backup-paths"
              >Backup paths</label
            >
            <textarea
              id="backup-paths"
              v-model="form.backup_sources"
              class="input area-input"
              placeholder="Directories to back up, one per line"
              spellcheck="false"
            />
            <span class="field-hint">
              One per line. Leave empty to use each agent's own default paths. The same paths are
              written to every target.
            </span>
          </div>
        </template>

        <template v-else-if="currentStep.id === 'targets'">
          <p class="pane-lede">
            The repositories this schedule writes into. Add more than one to keep independent copies
            on one schedule, written one after another.
          </p>
          <ScheduleRepoTargets
            v-model="repoTargets"
            :repos="repos"
          />
          <div
            v-if="showOnFailure"
            class="field"
          >
            <label
              class="field-label"
              for="on-failure"
              >On failure</label
            >
            <select
              id="on-failure"
              v-model="onFailure"
              class="input"
            >
              <option value="stop">Stop the run</option>
              <option value="continue">Carry on</option>
            </select>
            <span class="field-hint">
              What a failing host, or a failing required target, does to the rest of the run.
            </span>
          </div>

          <p
            v-if="repoTargets.length > 1"
            class="wizard-note"
          >
            <TriangleAlert :size="14" />
            <span>
              {{ requiredTargetCount }} of {{ repoTargets.length }} targets
              {{ requiredTargetCount === 1 ? 'is' : 'are' }} required. A failing required target
              {{
                onFailure === 'stop'
                  ? 'ends the run, skipping the targets and hosts after it'
                  : 'is recorded and the run carries on'
              }}; a best-effort one is reported as a warning either way.
            </span>
          </p>
        </template>

        <template v-else-if="currentStep.id === 'timing'">
          <p class="pane-lede">
            When the schedule fires, and how many misses are tolerated before it is called failed.
          </p>
          <div class="field">
            <label class="field-label">
              Schedule
              <span class="required">*</span>
            </label>
            <CronBuilder v-model="form.cron_expression" />
          </div>
          <div class="field">
            <label
              class="field-label"
              for="missed-threshold"
              >Mark as failed after</label
            >
            <input
              id="missed-threshold"
              v-model.number="form.missed_backup_threshold"
              type="number"
              min="1"
              class="input"
            />
            <span class="field-hint">
              Consecutive missed backups tolerated before this schedule is marked failed and
              disabled.
            </span>
          </div>
        </template>

        <template v-else-if="currentStep.id === 'retention'">
          <p class="pane-lede">
            How many archives borg keeps when this schedule prunes. Zero keeps none of that
            interval, and every target keeps the same history.
          </p>
          <div class="retention-grid">
            <div class="field">
              <label
                class="field-label"
                for="keep-hourly"
                >Hourly</label
              >
              <input
                id="keep-hourly"
                v-model.number="form.keep_hourly"
                type="number"
                min="0"
                class="input"
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="keep-daily"
                >Daily</label
              >
              <input
                id="keep-daily"
                v-model.number="form.keep_daily"
                type="number"
                min="0"
                class="input"
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="keep-weekly"
                >Weekly</label
              >
              <input
                id="keep-weekly"
                v-model.number="form.keep_weekly"
                type="number"
                min="0"
                class="input"
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="keep-monthly"
                >Monthly</label
              >
              <input
                id="keep-monthly"
                v-model.number="form.keep_monthly"
                type="number"
                min="0"
                class="input"
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="keep-yearly"
                >Yearly</label
              >
              <input
                id="keep-yearly"
                v-model.number="form.keep_yearly"
                type="number"
                min="0"
                class="input"
              />
            </div>
          </div>
        </template>

        <template v-else-if="currentStep.id === 'advanced'">
          <ScheduleAdvancedTab
            v-model:form="form"
            v-model:overrides="agentOverrides"
            :agent-ids="selectedAgentIds"
            :agent-label="agentLabel"
          />
        </template>

        <template v-else>
          <p class="pane-lede">
            What will be created. Every block links back to the step that owns it.
          </p>

          <div
            v-if="allBlockers.length > 0"
            class="wizard-note wizard-note--warning"
          >
            <TriangleAlert :size="14" />
            <span>
              Still missing:
              {{ allBlockers.map((b) => `${b.text} (${b.step.label})`).join(', ') }}.
            </span>
          </div>
          <div
            v-else
            class="wizard-note wizard-note--success"
          >
            <Check :size="14" />
            <span>
              Each of {{ selectedAgentIds.length }}
              {{ selectedAgentIds.length === 1 ? 'host' : 'hosts' }} writes
              {{ repoTargets.length }}
              {{ repoTargets.length === 1 ? 'copy' : 'copies' }}, {{ cronSummary.toLowerCase() }}.
            </span>
          </div>

          <div class="review-grid">
            <div class="review-block">
              <div class="review-head">
                <span class="group-label">Schedule</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="goTo(stepIndexOf('basics'))"
                >
                  Edit
                </button>
              </div>
              <dl class="info-grid">
                <dt>Name</dt>
                <dd>{{ form.name || '-' }}</dd>
                <dt>Type</dt>
                <dd>{{ scheduleTypeLabel }}</dd>
                <dt>State</dt>
                <dd>{{ form.enabled ? 'Enabled' : 'Created paused' }}</dd>
              </dl>
            </div>

            <div class="review-block">
              <div class="review-head">
                <span class="group-label">Timing</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="goTo(stepIndexOf('timing'))"
                >
                  Edit
                </button>
              </div>
              <dl class="info-grid">
                <dt>Runs</dt>
                <dd>{{ cronSummary }}</dd>
                <dt>Cron</dt>
                <dd class="mono">{{ form.cron_expression }}</dd>
                <dt>Missed limit</dt>
                <dd>{{ form.missed_backup_threshold }}</dd>
              </dl>
            </div>

            <div class="review-block">
              <div class="review-head">
                <span class="group-label">Sources</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="goTo(stepIndexOf('sources'))"
                >
                  Edit
                </button>
              </div>
              <dl class="info-grid">
                <dt>Hosts</dt>
                <dd>{{ selectedAgentIds.map(agentLabel).join(', ') || '-' }}</dd>
                <dt v-if="isBackup">Paths</dt>
                <dd
                  v-if="isBackup"
                  class="mono"
                >
                  {{ parseLines(form.backup_sources).join(', ') || 'agent defaults' }}
                </dd>
              </dl>
            </div>

            <div class="review-block">
              <div class="review-head">
                <span class="group-label">Targets</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="goTo(stepIndexOf('targets'))"
                >
                  Edit
                </button>
              </div>
              <dl class="info-grid">
                <dt>Repositories</dt>
                <dd>{{ repoTargets.map((t) => repoName(t.repo_id)).join(' then ') || '-' }}</dd>
                <dt>Required</dt>
                <dd>{{ requiredTargetCount }} of {{ repoTargets.length }}</dd>
                <dt v-if="showOnFailure">On failure</dt>
                <dd v-if="showOnFailure">
                  {{ onFailure === 'stop' ? 'Stop the run' : 'Carry on' }}
                </dd>
              </dl>
            </div>

            <div
              v-if="isBackup"
              class="review-block"
            >
              <div class="review-head">
                <span class="group-label">Retention</span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="goTo(stepIndexOf('retention'))"
                >
                  Edit
                </button>
              </div>
              <dl class="info-grid">
                <dt>Every target</dt>
                <dd class="mono">
                  {{ form.keep_hourly }}H {{ form.keep_daily }}d {{ form.keep_weekly }}w
                  {{ form.keep_monthly }}m {{ form.keep_yearly }}y
                </dd>
              </dl>
            </div>
          </div>

          <div
            v-if="submitError"
            class="form-error"
          >
            {{ submitError }}
          </div>
        </template>
      </section>

      <div class="wizard-foot">
        <span
          v-if="isLastStep && allBlockers.length > 0"
          class="wizard-status wizard-status--blocked"
        >
          <TriangleAlert :size="14" />
          {{ allBlockers.length }} required
          {{ allBlockers.length === 1 ? 'field is' : 'fields are' }} still empty.
        </span>
        <span
          v-else-if="currentBlockers.length > 0"
          class="wizard-status wizard-status--blocked"
        >
          <TriangleAlert :size="14" />
          Needs {{ currentBlockers.join(' and ') }} to continue.
        </span>
        <span
          v-else
          class="wizard-status"
        >
          Step {{ stepIndex + 1 }} of {{ steps.length }}.
        </span>

        <div class="wizard-actions">
          <button
            type="button"
            class="btn btn-ghost"
            @click="router.push('/schedules')"
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn btn-ghost"
            :disabled="stepIndex === 0"
            @click="back"
          >
            <ChevronLeft :size="14" />
            Back
          </button>
          <button
            v-if="!isLastStep"
            type="button"
            class="btn btn-primary"
            :disabled="currentBlockers.length > 0"
            @click="next"
          >
            Continue
            <ChevronRight :size="14" />
          </button>
          <button
            v-else
            type="button"
            class="btn btn-primary"
            :disabled="allBlockers.length > 0 || submitting"
            @click="submit"
          >
            {{ submitting ? 'Creating...' : 'Create schedule' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.schedule-create-view {
  max-width: 1000px;
  min-width: 0;
}

.wizard {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  padding: 0;
  overflow: hidden;
}

.wizard-rail {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-6) var(--space-5);
  background: var(--bg-sidebar);
  border-right: 1px solid var(--border);
}

.wizard-step {
  display: grid;
  grid-template-columns: 1.375rem minmax(0, 1fr);
  align-items: center;
  gap: var(--space-5);
  width: 100%;
  padding: var(--space-4);
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-base);
}

.wizard-step:hover {
  background: var(--bg-hover);
}

.wizard-step--current {
  background: var(--accent-subtle);
  color: var(--accent);
  font-weight: 600;
}

.wizard-num {
  display: grid;
  place-items: center;
  width: 1.375rem;
  height: 1.375rem;
  border-radius: 50%;
  border: 1px solid var(--border);
  background: var(--bg-input);
  color: var(--text-muted);
  font-size: var(--fs-2xs);
  font-weight: 700;
}

.wizard-num--current {
  border-color: var(--accent);
  color: var(--accent);
}

.wizard-num--done {
  border-color: var(--success);
  background: var(--success);
  color: var(--text-on-accent);
}

.wizard-num--todo {
  border-color: var(--warning);
  color: var(--warning);
}

.wizard-step-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  font-size: var(--fs-sm);
}

.wizard-step-sub {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-weight: 400;
}

.wizard-pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-7);
  padding: var(--space-8);
  min-width: 0;
}

.wizard-foot {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-6);
  padding: var(--space-6) var(--space-8);
  border-top: 1px solid var(--border);
  background: var(--bg-elevated);
}

.wizard-status {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.wizard-status--blocked {
  color: var(--warning);
}

.wizard-actions {
  display: flex;
  gap: var(--space-4);
}

.wizard-note {
  display: flex;
  align-items: flex-start;
  gap: var(--space-5);
  margin: 0;
  padding: var(--space-5) var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.wizard-note--warning {
  border-color: var(--warning);
  background: var(--warning-subtle);
}

.wizard-note--success {
  border-color: var(--success);
  background: var(--success-subtle);
}

.review-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
  gap: var(--space-6);
}

.review-block {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  padding: var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
}

.review-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

@media (max-width: 768px) {
  .wizard {
    grid-template-columns: minmax(0, 1fr);
  }

  .wizard-rail {
    flex-direction: row;
    overflow-x: auto;
    border-right: none;
    border-bottom: 1px solid var(--border);
  }

  .wizard-step-sub {
    display: none;
  }

  .wizard-pane {
    padding: var(--space-6);
  }
}
</style>

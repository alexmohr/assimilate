<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowDown, ArrowUp } from '@lucide/vue'
import AgentMultiSelect from './AgentMultiSelect.vue'
import CronBuilder from './CronBuilder.vue'
import HelpHint from './HelpHint.vue'
import ScheduleRepoTargets from './ScheduleRepoTargets.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import PerAgentFields from './PerAgentFields.vue'
import ScheduleAdvancedTab from './ScheduleAdvancedTab.vue'
import SchedulePowerTab from './SchedulePowerTab.vue'
import SettingsRail, { type SettingsSections } from './SettingsRail.vue'
import type { ScheduleRepoTarget } from '../api/schedules'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'
import type { ScheduleSettingsSection } from '../utils/scheduleSettings'

/**
 * Everything that configures a schedule, behind one tab with a sub-nav.
 *
 * These were seven cards stacked in a flat column - General, Target (create
 * only), Schedule Info (read-only, edit only), Target Settings (edit only,
 * re-editing four of the fields Schedule Info had just displayed), Timing,
 * Backup Paths, Retention - with Advanced as a separate top-level tab and
 * Danger Zone as a page footer. Schedule Info's live facts moved to the
 * Overview tab; Target Settings and the create-only Target card turned out to
 * be the same fields and merged into Targets; Timing had only two fields and
 * folded into General; Danger Zone is a single action, so it lives in the
 * header's overflow menu instead of a section of its own.
 */
const props = defineProps<{
  section: ScheduleSettingsSection
  isBackup: boolean
  agents: readonly AgentRow[]
  repos: readonly Repo[]
  agentLabel: (id: number) => string
  /** Whether the viewer may see wake details - see `SchedulePowerTab`. */
  canSeeWakeDetails: boolean
  /** True while the page's save is in flight, so the target editor locks. */
  saving?: boolean
}>()

const emit = defineEmits<{ 'update:section': [value: ScheduleSettingsSection] }>()

const form = defineModel<ScheduleFormState>('form', { required: true })
const overrides = defineModel<ScheduleAgentOverrides>('overrides', { required: true })
const selectedAgentIds = defineModel<number[]>('selectedAgentIds', { required: true })
const repoTargets = defineModel<ScheduleRepoTarget[]>('repoTargets', { required: true })
const onFailure = defineModel<'stop' | 'continue'>('onFailure', { required: true })
const usePerHostPaths = defineModel<boolean>('usePerHostPaths', { required: true })
const perHostSources = defineModel<Record<number, string>>('perHostSources', { required: true })

/**
 * Retention and Advanced only apply to backup-type schedules. Power applies
 * to all of them: a check or verify run needs its hosts reachable just as
 * much as a backup does.
 */
const sections = computed<SettingsSections<ScheduleSettingsSection>>(() => [
  { id: 'general', label: 'General' },
  { id: 'targets', label: 'Targets' },
  { id: 'power', label: 'Power' },
  ...(props.isBackup
    ? [
        { id: 'retention', label: 'Retention' } as const,
        { id: 'advanced', label: 'Advanced' } as const,
      ]
    : []),
])

function moveAgentUp(index: number): void {
  if (index === 0) return
  const ids = [...selectedAgentIds.value]
  ;[ids[index - 1], ids[index]] = [ids[index], ids[index - 1]]
  selectedAgentIds.value = ids
}

function moveAgentDown(index: number): void {
  if (index >= selectedAgentIds.value.length - 1) return
  const ids = [...selectedAgentIds.value]
  ;[ids[index], ids[index + 1]] = [ids[index + 1], ids[index]]
  selectedAgentIds.value = ids
}

/**
 * The catch-up floor is stored in minutes, but nobody thinks in "120 minutes"
 * for a nightly schedule. The field shows whichever unit the stored value reads
 * naturally in, until the user picks one - then their choice wins for the rest
 * of the edit.
 */
type LeadUnit = 'minutes' | 'hours'
const MINUTES_PER_HOUR = 60

const leadUnitChoice = ref<LeadUnit | null>(null)

const leadUnit = computed<LeadUnit>({
  get: () => {
    if (leadUnitChoice.value) return leadUnitChoice.value
    const minutes = form.value.catch_up_min_lead_minutes
    return minutes >= MINUTES_PER_HOUR && minutes % MINUTES_PER_HOUR === 0 ? 'hours' : 'minutes'
  },
  set: (unit: LeadUnit) => {
    leadUnitChoice.value = unit
  },
})

const leadValue = computed<number>({
  get: () =>
    leadUnit.value === 'hours'
      ? form.value.catch_up_min_lead_minutes / MINUTES_PER_HOUR
      : form.value.catch_up_min_lead_minutes,
  set: (value: number) => {
    if (!Number.isFinite(value)) return
    const minutes = leadUnit.value === 'hours' ? value * MINUTES_PER_HOUR : value
    form.value.catch_up_min_lead_minutes = Math.max(1, Math.round(minutes))
  },
})
</script>

<template>
  <SettingsRail
    v-slot="{ section: currentSection }"
    :sections="sections"
    :section="section"
    even
    label="Schedule settings sections"
    @update:section="emit('update:section', $event)"
  >
    <template v-if="currentSection === 'general'">
      <div class="pane-head pane-head--end">
        <HelpHint
          label="naming and timing"
          align="end"
        >
          What this schedule is called, and when it runs.
        </HelpHint>
      </div>
      <div class="field">
        <label class="field-label">Name</label>
        <input
          v-model="form.name"
          type="text"
          class="input"
          placeholder="e.g. Daily web server backup"
        />
        <span class="field-hint">Optional display name for this schedule</span>
      </div>
      <div class="field">
        <label class="field-label">Schedule</label>
        <CronBuilder v-model="form.cron_expression" />
      </div>
      <div class="field field-inline">
        <label class="field-label">Enabled</label>
        <ToggleSwitch v-model="form.enabled" />
      </div>
      <div class="field">
        <div class="field-label-row field-label-row--tight">
          <label class="field-label">Mark as failed after</label>
          <HelpHint label="mark as failed after">
            Consecutive missed backups (agent or target unreachable at trigger time) tolerated
            before this schedule is marked failed and disabled. Below this count, a miss only shows
            as a warning.
          </HelpHint>
        </div>
        <input
          v-model.number="form.missed_backup_threshold"
          type="number"
          min="1"
          class="input"
        />
      </div>
      <div class="field field-inline">
        <div class="field-body">
          <p class="field-title">
            Catch up missed runs
            <HelpHint label="running once after an outage">
              If a host was offline when this schedule was due, run it once as soon as the host
              reconnects. Missed runs never stack: 35 missed occurrences still produce a single
              catch-up run.
            </HelpHint>
          </p>
        </div>
        <ToggleSwitch
          v-model="form.catch_up_missed_runs"
          label="Catch up missed runs"
        />
      </div>
      <div
        v-if="form.catch_up_missed_runs"
        class="field catch-up-lead"
      >
        <div class="field-label-row field-label-row--tight">
          <label
            class="field-label"
            for="catch-up-lead"
            >Only if the next run is at least</label
          >
          <HelpHint label="avoiding a collision with the next run">
            A catch-up is skipped when the next scheduled run is closer than this, so it never
            collides with the regular one. A host reconnecting 30 minutes before a 02:00 backup
            waits for that run instead.
          </HelpHint>
        </div>
        <div class="field-row">
          <input
            id="catch-up-lead"
            v-model.number="leadValue"
            type="number"
            min="1"
            class="input field-narrow"
          />
          <select
            v-model="leadUnit"
            class="input select-input select-input--sm"
            aria-label="Catch-up lead time unit"
          >
            <option value="minutes">minutes</option>
            <option value="hours">hours</option>
          </select>
          <span class="muted">away</span>
        </div>
      </div>
    </template>

    <template v-else-if="currentSection === 'targets'">
      <div class="pane-head pane-head--end">
        <HelpHint
          label="hosts and destinations"
          align="end"
        >
          Which hosts this schedule runs on, which repositories they write to, and what happens when
          one of them fails.
        </HelpHint>
      </div>
      <div class="field">
        <div class="field-label-row field-label-row--tight">
          <label class="field-label">Hosts</label>
          <HelpHint label="hosts">The agents that will execute this schedule.</HelpHint>
        </div>
        <AgentMultiSelect
          v-model="selectedAgentIds"
          :agents="agents"
        />
      </div>

      <ScheduleRepoTargets
        v-model="repoTargets"
        :repos="repos"
        :disabled="saving"
      />

      <div class="field">
        <div class="field-label-row field-label-row--tight">
          <label class="field-label">On failure</label>
          <HelpHint label="on failure">
            What a failing host, or a failing required target, does to the rest of the run.
          </HelpHint>
        </div>
        <select
          v-model="onFailure"
          class="input"
        >
          <option value="stop">Stop</option>
          <option value="continue">Continue</option>
        </select>
      </div>

      <div
        v-if="selectedAgentIds.length > 1"
        class="field"
      >
        <label class="field-label">Execution order</label>
        <div class="order-list">
          <div
            v-for="(agentId, idx) in selectedAgentIds"
            :key="agentId"
            class="order-item"
          >
            <span class="order-index">{{ idx + 1 }}</span>
            <span class="order-name">{{ agentLabel(agentId) }}</span>
            <div class="order-actions">
              <button
                type="button"
                class="order-btn"
                :disabled="idx === 0"
                title="Move up"
                aria-label="Move up"
                @click="moveAgentUp(idx)"
              >
                <ArrowUp :size="12" />
              </button>
              <button
                type="button"
                class="order-btn"
                :disabled="idx === selectedAgentIds.length - 1"
                title="Move down"
                aria-label="Move down"
                @click="moveAgentDown(idx)"
              >
                <ArrowDown :size="12" />
              </button>
            </div>
          </div>
        </div>
      </div>

      <template v-if="isBackup">
        <div
          v-if="selectedAgentIds.length > 1"
          class="field field-inline"
        >
          <label class="field-label">Configure paths per agent</label>
          <ToggleSwitch
            v-model="usePerHostPaths"
            label="Configure paths per agent"
          />
        </div>

        <div
          v-if="!usePerHostPaths"
          class="field"
        >
          <label class="field-label">Backup paths</label>
          <textarea
            v-model="form.backup_sources"
            class="input area-input"
            placeholder="Directories to back up, one per line"
            spellcheck="false"
          />
          <span class="field-hint">
            Leave empty to use the default paths configured for this agent.
          </span>
        </div>

        <div
          v-else
          class="field"
        >
          <label class="field-label">Backup paths</label>
          <PerAgentFields
            :agent-ids="selectedAgentIds"
            :agent-label="agentLabel"
          >
            <template #default="{ agentId }">
              <textarea
                :value="perHostSources[agentId] ?? ''"
                class="input area-input area-input-sm"
                placeholder="Directories to back up, one per line"
                spellcheck="false"
                @input="
                  ($event) =>
                    (perHostSources[agentId] = ($event.target as HTMLTextAreaElement).value)
                "
              />
            </template>
            <template #hint> Leave an agent empty to use its default backup paths. </template>
          </PerAgentFields>
        </div>
      </template>
    </template>

    <template v-else-if="currentSection === 'retention'">
      <div class="pane-head pane-head--end">
        <HelpHint
          label="how long archives are kept"
          align="end"
        >
          How many archives borg keeps when this schedule prunes. Blank or zero keeps none of that
          interval.
        </HelpHint>
      </div>
      <div class="retention-grid">
        <div class="field">
          <label class="field-label">Hourly</label>
          <input
            v-model.number="form.keep_hourly"
            type="number"
            min="0"
            class="input"
          />
        </div>
        <div class="field">
          <label class="field-label">Daily</label>
          <input
            v-model.number="form.keep_daily"
            type="number"
            min="0"
            class="input"
          />
        </div>
        <div class="field">
          <label class="field-label">Weekly</label>
          <input
            v-model.number="form.keep_weekly"
            type="number"
            min="0"
            class="input"
          />
        </div>
        <div class="field">
          <label class="field-label">Monthly</label>
          <input
            v-model.number="form.keep_monthly"
            type="number"
            min="0"
            class="input"
          />
        </div>
        <div class="field">
          <label class="field-label">Yearly</label>
          <input
            v-model.number="form.keep_yearly"
            type="number"
            min="0"
            class="input"
          />
        </div>
      </div>
    </template>

    <SchedulePowerTab
      v-else-if="currentSection === 'power'"
      v-model:wake-override="form.wake_override"
      :agents="agents"
      :repos="repos"
      :selected-agent-ids="selectedAgentIds"
      :selected-repo-ids="repoTargets.map((t) => t.repo_id)"
      :can-see-wake-details="canSeeWakeDetails"
    />

    <ScheduleAdvancedTab
      v-else-if="currentSection === 'advanced'"
      v-model:form="form"
      v-model:overrides="overrides"
      :agent-ids="selectedAgentIds"
      :agent-label="agentLabel"
    />
  </SettingsRail>
</template>

<style scoped>
/* The settings sub-nav shape (.settings-tab/-nav/-nav-item/-pane), .required,
   the execution-order list (.order-*) and the textarea sizes
   (.area-input/-sm) all live in style.css; the host picker and the repository
   target list are their own components. What is left here is this page's own:
   the catch-up floor's indent, and the mobile collapse that keeps the sub-nav
   on one row. */

/* The catch-up floor only means anything under the toggle that switches
   catch-up on, so it is indented against it rather than reading as a sibling
   setting of its own. */
.catch-up-lead {
  border-left: 2px solid var(--border);
  padding-left: var(--space-6);
  margin-left: var(--space-2);
}

/* At most four sections, and they fit one row at any width - so this rail
   spreads them into equal cells rather than wrapping the way the shared
   collapse in style.css does for an unbounded list. */
@media (max-width: 768px) {
  .settings-nav {
    flex-wrap: nowrap;
  }

  .settings-nav-item {
    flex: 1 1 0;
    min-width: 0;
    text-align: center;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}
</style>

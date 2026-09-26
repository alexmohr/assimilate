<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { ArrowDown, ArrowUp } from '@lucide/vue'
import AgentMultiSelect from './AgentMultiSelect.vue'
import CronBuilder from './CronBuilder.vue'
import DurationField from './DurationField.vue'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
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
import type { ScheduleCatchUpSourcesResponse } from '../types/generated'
import { humanizeMinutes } from '../utils/duration'

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
  /**
   * Which of this schedule's hosts and repositories are marked as not always
   * online - the ones its catch-up floor applies to. `null` until loaded.
   */
  catchUpSources?: ScheduleCatchUpSourcesResponse | null
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
 * The floor goes up to weeks because it is compared against the gap between
 * two runs, and on a weekly schedule "at least 2 hours away" never blocks
 * anything.
 */
const LEAD_UNITS = ['minutes', 'hours', 'days', 'weeks'] as const

/**
 * `null` until loaded - and then the field stays enabled, rather than
 * flickering disabled for a schedule that may well have somewhere to apply.
 */
const hasCatchUpSources = computed(() => {
  const sources = props.catchUpSources
  if (!sources) return true
  return sources.hosts.length + sources.repository_hosts.length > 0
})

/** The cutoff in words, so the hint reads as a whole sentence with its value. */
const cutoffText = computed(() => humanizeMinutes(form.value.catch_up_min_lead_minutes))

const catchUpSourceLinks = computed(() => {
  const sources = props.catchUpSources
  if (!sources) return []
  return [
    ...sources.hosts.map((h) => ({
      name: h.name,
      kind: 'host',
      to: `/agents/${encodeURIComponent(h.name)}`,
    })),
    ...sources.repository_hosts.map((h) => ({
      name: h.name,
      kind: 'repository host',
      to: `/repo-hosts/${h.id}`,
    })),
  ]
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
      <div class="pane-rows">
        <PaneRow
          title="Name"
          stack
        >
          <input
            v-model="form.name"
            type="text"
            class="input"
            placeholder="e.g. Daily web server backup"
          />
          <span class="field-hint">Optional display name for this schedule</span>
        </PaneRow>
        <PaneRow
          title="Schedule"
          stack
        >
          <CronBuilder v-model="form.cron_expression" />
        </PaneRow>
        <PaneRow title="Enabled">
          <ToggleSwitch
            v-model="form.enabled"
            label="Enabled"
          />
        </PaneRow>
        <PaneRow
          title="Mark as failed after"
          help="mark as failed after"
        >
          <template #help>
            Consecutive missed backups (agent or target unreachable at trigger time) tolerated
            before this schedule is marked failed and disabled. Below this count, a miss only shows
            as a warning.
          </template>
          <input
            v-model.number="form.missed_backup_threshold"
            type="number"
            min="1"
            class="input"
            aria-label="Mark as failed after"
          />
        </PaneRow>
        <PaneRow
          title="Catch-up cutoff"
          label-for="catch-up-lead"
          help="avoiding a collision with the next run"
          stack
        >
          <template #help>
            When an agent or repository host that is marked as not always online comes back after
            missing one of this schedule's runs, the run is caught up - unless the schedule is about
            to run again anyway. If the next scheduled run is closer than this, the catch-up is
            dropped rather than delayed. On a weekly schedule two hours never blocks anything, which
            is why this goes up to weeks. Whether a host is waited for at all is set on its own
            Power pane.
          </template>
          <template #hint>
            A missed run is not caught up if this schedule runs again within
            {{ cutoffText }} anyway.
            <template v-if="catchUpSources && hasCatchUpSources">
              Applies to runs missed because one of these was offline:
              <template
                v-for="(source, index) in catchUpSourceLinks"
                :key="source.to"
              >
                <RouterLink :to="source.to">{{ source.name }}</RouterLink>
                ({{ source.kind }}){{ index < catchUpSourceLinks.length - 1 ? ', ' : '' }}
              </template>
            </template>
            <template v-else-if="catchUpSources">
              None of this schedule's agents or repository hosts is marked as not always online -
              set that on their Power pane.
            </template>
          </template>
          <DurationField
            v-model="form.catch_up_min_lead_minutes"
            input-id="catch-up-lead"
            :units="LEAD_UNITS"
            unit-label="Catch-up cutoff unit"
            :disabled="!hasCatchUpSources"
          />
        </PaneRow>
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
      <div class="pane-rows">
        <PaneRow
          title="Hosts"
          help="hosts"
          stack
        >
          <template #help> The agents that will execute this schedule. </template>
          <AgentMultiSelect
            v-model="selectedAgentIds"
            :agents="agents"
          />
        </PaneRow>

        <ScheduleRepoTargets
          v-model="repoTargets"
          :repos="repos"
          :disabled="saving"
        />

        <PaneRow
          title="On failure"
          help="on failure"
        >
          <template #help>
            What a failing host, or a failing required target, does to the rest of the run.
          </template>
          <select
            v-model="onFailure"
            class="input"
            aria-label="On failure"
          >
            <option value="stop">Stop</option>
            <option value="continue">Continue</option>
          </select>
        </PaneRow>

        <PaneRow
          v-if="selectedAgentIds.length > 1"
          title="Execution order"
          hint="Hosts run in this order, top to bottom."
          stack
        >
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
        </PaneRow>

        <template v-if="isBackup">
          <PaneRow
            v-if="selectedAgentIds.length > 1"
            title="Configure paths per agent"
            help="splitting the paths by host"
          >
            <template #help>
              Give each host its own backup paths instead of one list for the schedule.
            </template>
            <ToggleSwitch
              v-model="usePerHostPaths"
              label="Configure paths per agent"
            />
          </PaneRow>

          <PaneRow
            title="Backup paths"
            stack
          >
            <textarea
              v-if="!usePerHostPaths"
              v-model="form.backup_sources"
              class="input area-input"
              aria-label="Backup paths"
              placeholder="Directories to back up, one per line"
              spellcheck="false"
            />
            <span
              v-if="!usePerHostPaths"
              class="field-hint"
            >
              Leave empty to use the default paths configured for this agent.
            </span>
            <PerAgentFields
              v-else
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
          </PaneRow>
        </template>
      </div>
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
      <div class="pane-rows">
        <PaneRow title="Hourly">
          <input
            v-model.number="form.keep_hourly"
            type="number"
            min="0"
            class="input field-narrow"
            aria-label="Hourly"
          />
        </PaneRow>
        <PaneRow title="Daily">
          <input
            v-model.number="form.keep_daily"
            type="number"
            min="0"
            class="input field-narrow"
            aria-label="Daily"
          />
        </PaneRow>
        <PaneRow title="Weekly">
          <input
            v-model.number="form.keep_weekly"
            type="number"
            min="0"
            class="input field-narrow"
            aria-label="Weekly"
          />
        </PaneRow>
        <PaneRow title="Monthly">
          <input
            v-model.number="form.keep_monthly"
            type="number"
            min="0"
            class="input field-narrow"
            aria-label="Monthly"
          />
        </PaneRow>
        <PaneRow title="Yearly">
          <input
            v-model.number="form.keep_yearly"
            type="number"
            min="0"
            class="input field-narrow"
            aria-label="Yearly"
          />
        </PaneRow>
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
/* The settings sub-nav shape (.settings-tab/-nav/-nav-item/-pane), the row
   convention (.pane-row*), the nesting indent (.pane-nest), .required, the
   execution-order list (.order-*) and the textarea sizes (.area-input/-sm) all
   live in style.css; the host picker and the repository target list are their
   own components. What is left here is this page's own: the mobile collapse
   that keeps the sub-nav on one row. */

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

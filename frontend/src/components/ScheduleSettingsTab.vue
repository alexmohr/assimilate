<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { ArrowDown, ArrowUp, RefreshCw } from '@lucide/vue'
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
import type { RepoCatchUpWait, ScheduleRepoTarget } from '../api/schedules'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import type { AgentRow } from '../types/agent'
import type { Repo } from '../types/repo'
import type { ScheduleSettingsSection } from '../utils/scheduleSettings'
import { humanizeMinutes } from '../utils/duration'
import { relativeTime } from '../utils/format'

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
   * Repositories this schedule is currently waiting on before it can catch up
   * a run they were away for. Empty for a schedule with nothing pending, which
   * is the normal case - the whole block only renders when there is something
   * to say.
   */
  catchUpWaits?: readonly RepoCatchUpWait[]
  /** True while a manual re-check is in flight. */
  checkingCatchUp?: boolean
}>()

const emit = defineEmits<{
  'update:section': [value: ScheduleSettingsSection]
  'check-catch-up': []
}>()

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
 * What each catch-up field offers, finest unit first.
 *
 * The floor goes up to weeks because it is compared against the gap between
 * two runs, and on a weekly schedule "at least 2 hours away" never blocks
 * anything. The re-check stops at days: an interval longer than any schedule's
 * own period would only ever find the repository after a regular run had
 * already covered the gap. The give-up window starts at hours, because a
 * window shorter than that cannot hold even one default re-check.
 */
const LEAD_UNITS = ['minutes', 'hours', 'days', 'weeks'] as const
const RECHECK_UNITS = ['minutes', 'hours', 'days'] as const
const GIVE_UP_UNITS = ['hours', 'days', 'weeks'] as const

const waits = computed<readonly RepoCatchUpWait[]>(() => props.catchUpWaits ?? [])

/**
 * One line per waiting repository: what it missed, when it was last asked,
 * when it is asked next, and - when the schedule has a window - how much of
 * that window is left. A countdown nobody can see is a countdown that only
 * surprises people.
 */
function waitDetail(wait: RepoCatchUpWait): string {
  const parts = [
    `missed ${relativeTime(wait.pending_for)}`,
    wait.last_probe_at ? `last checked ${relativeTime(wait.last_probe_at)}` : 'not checked yet',
    `next check ${relativeTime(wait.next_probe_at)}`,
  ]
  if (wait.give_up_at) parts.push(`giving up ${relativeTime(wait.give_up_at)}`)
  return parts.join(' \u00b7 ')
}

const giveUpHint = computed(() =>
  form.value.catch_up_give_up_minutes === 0
    ? 'Leave empty to wait indefinitely.'
    : `Reported as a failed backup after ${humanizeMinutes(form.value.catch_up_give_up_minutes)}.`,
)
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
          title="Catch up missed runs"
          help="running once after an outage"
        >
          <template #help>
            If a host or a repository was offline when this schedule was due, run it once as soon as
            it is back. Missed runs never stack: however many occurrences pass during an outage, at
            most one catch-up run follows.
          </template>
          <ToggleSwitch
            v-model="form.catch_up_missed_runs"
            label="Catch up missed runs"
          />
        </PaneRow>
        <PaneRow
          v-if="form.catch_up_missed_runs"
          class="pane-nest"
          title="Only if the next run is at least"
          label-for="catch-up-lead"
          help="avoiding a collision with the next run"
          stack
        >
          <template #help>
            Decides whether a catch-up is still worth doing once the host or repository is back. If
            the next scheduled run is closer than this, the catch-up is dropped rather than delayed,
            because that run is about to do the same work. A repository answering 30 minutes before
            a 02:00 backup waits for that run instead.
          </template>
          <DurationField
            v-model="form.catch_up_min_lead_minutes"
            input-id="catch-up-lead"
            :units="LEAD_UNITS"
            unit-label="Catch-up lead time unit"
          >
            <span class="muted">away</span>
          </DurationField>
        </PaneRow>
        <PaneRow
          v-if="form.catch_up_missed_runs"
          class="pane-nest"
          title="Re-check an offline repository every"
          label-for="catch-up-recheck"
          help="asking a repository that was away"
          stack
        >
          <template #help>
            A host announces its own return by reconnecting, so its catch-up runs at once. A
            repository cannot, so Assimilate asks it over SSH on this interval and catches up on the
            first answer. Longer than this schedule's own period is self-defeating: the repository
            is then usually found only after a scheduled run has already covered the gap.
          </template>
          <DurationField
            v-model="form.catch_up_repo_recheck_minutes"
            input-id="catch-up-recheck"
            :units="RECHECK_UNITS"
            unit-label="Repository re-check interval unit"
          />
        </PaneRow>
        <PaneRow
          v-if="form.catch_up_missed_runs"
          class="pane-nest"
          title="Stop waiting after"
          label-for="catch-up-give-up"
          help="bounding how long a catch-up stays pending"
          :hint="giveUpHint"
          stack
        >
          <template #help>
            A pending catch-up is abandoned once it has waited this long, and the run is reported as
            failed rather than staying skipped, because the backup is not going to happen. Without
            it a weekly schedule can sit waiting on a repository for the whole week and report
            nothing worse than "skipped" the entire time. Separate from
            <strong>Mark as failed after</strong>, which counts consecutive missed occurrences and
            disables the schedule.
          </template>
          <DurationField
            v-model="form.catch_up_give_up_minutes"
            input-id="catch-up-give-up"
            :units="GIVE_UP_UNITS"
            unit-label="Give-up window unit"
            clearable
          />
        </PaneRow>
        <PaneRow
          v-if="form.catch_up_missed_runs && waits.length > 0"
          class="pane-nest"
          title="Waiting on"
          help="what is pending right now"
          stack
        >
          <template #help>
            Runs this schedule has already missed and will catch up as soon as the repository
            answers. Checking now asks every one of them immediately instead of waiting out the
            interval above.
          </template>
          <template #titleAside>
            <button
              type="button"
              class="btn btn-sm"
              :disabled="checkingCatchUp"
              @click="emit('check-catch-up')"
            >
              <RefreshCw
                :size="14"
                :class="{ spinning: checkingCatchUp }"
              />
              {{ checkingCatchUp ? 'Checking...' : 'Check now' }}
            </button>
          </template>
          <div class="rows">
            <div
              v-for="wait in waits"
              :key="wait.repo_id"
              class="agent-row"
            >
              <span class="agent-row-stripe agent-row-stripe--warning" />
              <div class="agent-row-name">{{ wait.repo_name }}</div>
              <div class="agent-row-sub">{{ waitDetail(wait) }}</div>
            </div>
          </div>
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

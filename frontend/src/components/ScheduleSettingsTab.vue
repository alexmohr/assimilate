<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import CronBuilder from './CronBuilder.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import PerAgentFields from './PerAgentFields.vue'
import ScheduleAdvancedTab from './ScheduleAdvancedTab.vue'
import type { ScheduleAgentOverrides, ScheduleFormState } from '../types/scheduleForm'
import type { ScheduleType } from '../types/schedule'
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
  isCreate: boolean
  isBackup: boolean
  agents: readonly AgentRow[]
  repos: readonly Repo[]
  agentLabel: (id: number) => string
}>()

const emit = defineEmits<{ 'update:section': [value: ScheduleSettingsSection] }>()

const form = defineModel<ScheduleFormState>('form', { required: true })
const overrides = defineModel<ScheduleAgentOverrides>('overrides', { required: true })
const selectedAgentIds = defineModel<number[]>('selectedAgentIds', { required: true })
const selectedRepoId = defineModel<number | null>('selectedRepoId', { required: true })
const selectedType = defineModel<ScheduleType>('selectedType', { required: true })
const onFailure = defineModel<'stop' | 'continue'>('onFailure', { required: true })
const usePerHostPaths = defineModel<boolean>('usePerHostPaths', { required: true })
const perHostSources = defineModel<Record<number, string>>('perHostSources', { required: true })

interface SectionOption {
  id: ScheduleSettingsSection
  label: string
}

/** Retention and Advanced only apply to backup-type schedules. */
const sections = computed<SectionOption[]>(() => {
  const list: SectionOption[] = [
    { id: 'general', label: 'General' },
    { id: 'targets', label: 'Targets' },
  ]
  if (props.isBackup) {
    list.push({ id: 'retention', label: 'Retention' })
    list.push({ id: 'advanced', label: 'Advanced' })
  }
  return list
})

/**
 * What the pane renders, derived from `sections` rather than the `section`
 * prop directly: switching Schedule Type away from Backup mid-create can
 * strand the prop on a section (Retention, Advanced) this schedule no longer
 * has, and this falls back to General rather than rendering behind the
 * sub-nav's back. Same shape as AgentSettingsTab's `currentSection`.
 */
const currentSection = computed<ScheduleSettingsSection>(() =>
  sections.value.some((s) => s.id === props.section) ? props.section : 'general',
)

function multiSelectLabel(): string {
  const ids = selectedAgentIds.value
  if (ids.length === 0) return 'Select agents...'
  if (ids.length === 1) return props.agentLabel(ids[0])
  return `${ids.length} agents selected`
}

function toggleAgentSelection(id: number): void {
  if (selectedAgentIds.value.includes(id)) {
    selectedAgentIds.value = selectedAgentIds.value.filter((x) => x !== id)
  } else {
    selectedAgentIds.value = [...selectedAgentIds.value, id]
  }
}

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

const showAgentDropdown = ref(false)
const agentDropdownRef = ref<HTMLElement | null>(null)

function handleClickOutside(event: MouseEvent): void {
  if (
    showAgentDropdown.value &&
    agentDropdownRef.value &&
    !agentDropdownRef.value.contains(event.target as Node)
  ) {
    showAgentDropdown.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
})

onBeforeUnmount(() => {
  document.removeEventListener('click', handleClickOutside)
})
</script>

<template>
  <div class="settings-tab">
    <nav
      class="settings-nav"
      aria-label="Schedule settings sections"
    >
      <button
        v-for="s in sections"
        :key="s.id"
        type="button"
        class="settings-nav-item"
        :aria-current="s.id === currentSection"
        @click="emit('update:section', s.id)"
      >
        {{ s.label }}
      </button>
    </nav>

    <div class="settings-pane">
      <div
        v-if="currentSection === 'general'"
        class="info-card"
      >
        <h3 class="info-title">General</h3>
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
        <div
          v-if="isCreate"
          class="field"
        >
          <label class="field-label">Schedule Type</label>
          <select
            v-model="selectedType"
            class="input"
          >
            <option value="backup">Backup</option>
            <option value="check">Integrity Check</option>
            <option value="verify">Verify (extract dry-run)</option>
          </select>
          <span class="field-hint">
            Backup creates archives; Check validates repo integrity; Verify tests extractability.
          </span>
        </div>
      </div>

      <div
        v-else-if="currentSection === 'targets'"
        class="info-card"
      >
        <h3 class="info-title">Targets</h3>
        <div class="field">
          <label class="field-label"
            >Hosts
            <span
              v-if="isCreate"
              class="required"
              >*</span
            ></label
          >
          <div
            ref="agentDropdownRef"
            class="multi-select-wrapper"
          >
            <button
              type="button"
              class="multi-select-trigger"
              :class="{ open: showAgentDropdown }"
              @click.stop="showAgentDropdown = !showAgentDropdown"
            >
              <span class="multi-select-label">{{ multiSelectLabel() }}</span>
              <span class="multi-select-arrow">{{ showAgentDropdown ? '▲' : '▼' }}</span>
            </button>
            <div
              v-if="showAgentDropdown"
              class="multi-select-dropdown"
            >
              <label
                v-for="a in agents"
                :key="a.id"
                class="multi-select-item"
              >
                <input
                  type="checkbox"
                  :checked="selectedAgentIds.includes(a.id)"
                  @change="toggleAgentSelection(a.id)"
                />
                <span class="multi-select-name">{{ a.display_name ?? a.hostname }}</span>
              </label>
            </div>
          </div>
          <span class="field-hint">The agents that will execute this schedule</span>
        </div>

        <div class="field">
          <label class="field-label"
            >Repository
            <span
              v-if="isCreate"
              class="required"
              >*</span
            ></label
          >
          <select
            v-model.number="selectedRepoId"
            class="input"
          >
            <option
              v-if="isCreate"
              :value="null"
              disabled
            >
              Select a repository...
            </option>
            <option
              v-for="r in repos"
              :key="r.id"
              :value="r.id"
            >
              {{ r.name }}
            </option>
          </select>
          <span
            v-if="isCreate"
            class="field-hint"
            >The borg repository to back up to</span
          >
        </div>

        <div class="field">
          <label class="field-label">On Failure</label>
          <select
            v-model="onFailure"
            class="input"
          >
            <option value="stop">Stop</option>
            <option value="continue">Continue</option>
          </select>
          <span class="field-hint">
            Whether to stop or continue to the next agent when one fails.
          </span>
        </div>

        <div
          v-if="selectedAgentIds.length > 1"
          class="field"
        >
          <label class="field-label">Execution Order</label>
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
                  @click="moveAgentUp(idx)"
                >
                  ▲
                </button>
                <button
                  type="button"
                  class="order-btn"
                  :disabled="idx === selectedAgentIds.length - 1"
                  title="Move down"
                  @click="moveAgentDown(idx)"
                >
                  ▼
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
            <ToggleSwitch v-model="usePerHostPaths" />
          </div>

          <div
            v-if="!usePerHostPaths"
            class="field"
          >
            <label class="field-label">Backup Paths</label>
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
            <label class="field-label">Backup Paths</label>
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
      </div>

      <div
        v-else-if="currentSection === 'retention'"
        class="info-card"
      >
        <h3 class="info-title">Retention</h3>
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
      </div>

      <ScheduleAdvancedTab
        v-else-if="currentSection === 'advanced'"
        v-model:form="form"
        v-model:overrides="overrides"
        :agent-ids="selectedAgentIds"
        :agent-label="agentLabel"
      />
    </div>
  </div>
</template>

<style scoped>
.settings-tab {
  display: flex;
  gap: 1.25rem;
  align-items: flex-start;
}

.settings-nav {
  width: 150px;
  flex: none;
  display: flex;
  flex-direction: column;
}

.settings-nav-item {
  font: inherit;
  text-align: left;
  font-size: var(--fs-sm);
  padding: 0.35rem 0.6rem;
  border: none;
  border-left: 2px solid transparent;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.settings-nav-item:hover {
  color: var(--text-primary);
}

.settings-nav-item[aria-current='true'] {
  color: var(--accent);
  font-weight: 600;
  border-left-color: var(--accent);
  background: var(--accent-subtle);
}

.settings-pane {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.required {
  color: var(--danger);
}

.retention-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(90px, 100%), 1fr));
  gap: 0.75rem;
}

.area-input {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.area-input-sm {
  min-height: 56px;
}

/* Multi-select */
.multi-select-wrapper {
  position: relative;
}

.multi-select-trigger {
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--fs-base);
  outline: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  transition: border-color var(--duration-base);
  box-sizing: border-box;
  text-align: left;
}

.multi-select-trigger:hover,
.multi-select-trigger.open {
  border-color: var(--accent);
}

.multi-select-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.multi-select-arrow {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  flex-shrink: 0;
}

.multi-select-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg);
  padding: 0.4rem;
  z-index: 100;
  max-height: 220px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.multi-select-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: var(--fs-base);
  color: var(--text-secondary);
  transition: background var(--duration-fast);
}

.multi-select-item:hover {
  background: var(--bg-hover);
}

.multi-select-item input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
  flex-shrink: 0;
}

.multi-select-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Ordering list */
.order-list {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.order-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.6rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
}

.order-index {
  font-size: var(--fs-2xs);
  font-weight: 700;
  color: var(--text-muted);
  min-width: 1.2rem;
  text-align: center;
}

.order-name {
  flex: 1;
  font-size: var(--fs-base);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.order-actions {
  display: flex;
  gap: 0.2rem;
  flex-shrink: 0;
}

.order-btn {
  padding: 0.4rem 0.6rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-lg);
  cursor: pointer;
  transition:
    background var(--duration-fast),
    color var(--duration-fast);
  line-height: 1;
}

.order-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.order-btn:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}

/* Four sections fit one row at any width, so the sub-nav never wraps to a
   second row the way an unbounded list would. */
@media (max-width: 720px) {
  .settings-tab {
    flex-direction: column;
  }

  .settings-nav {
    width: 100%;
    flex-direction: row;
    flex-wrap: nowrap;
    border-bottom: 1px solid var(--border);
  }

  .settings-nav-item {
    flex: 1 1 0;
    min-width: 0;
    text-align: center;
    border-left: none;
    border-bottom: 2px solid transparent;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .settings-nav-item[aria-current='true'] {
    border-left-color: transparent;
    border-bottom-color: var(--accent);
  }
}
</style>

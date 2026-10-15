<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { getScheduleMachines, updateScheduleMachines } from '../api/machines'
import { useWebSocket } from '../composables/useWebSocket'
import type {
  MachineWakeMode,
  ScheduleMachineInput,
  ScheduleMachineResponse,
  ScheduleWakeOverride,
} from '../types/generated'
import { badgeClass } from '../utils/badge'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import {
  choiceOf,
  machineRoleLabel,
  modeOf,
  scheduleMachineEffect,
  type MachineChoice,
  type MachineEffect,
} from '../utils/machine'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import EditableSection from './EditableSection.vue'
import MachineEffectList, { type MachineEffectRow } from './MachineEffectList.vue'
import PaneRow from './PaneRow.vue'

/**
 * Every machine this schedule needs - its agents, the hosts of its
 * repositories, its dependencies - and what the schedule does about each:
 * what "Wake hosts" above says, wake it anyway, never wake it, or wait for it
 * without waking it. Waiting is the one a machine's own settings cannot say
 * for one schedule only: a run that finds it down skips its targets on it,
 * runs them last, and catches them up once it answers.
 *
 * Saved on its own, apart from the schedule form, like the dependencies
 * below: it is a separate resource on the server. Each row shows what its
 * choice resolves to against the schedule-wide setting still being edited
 * above, so the effect of either is on screen before anything is saved.
 */
const props = defineProps<{
  scheduleId: number
  /** The schedule-wide setting as the form holds it now. */
  wakeOverride: ScheduleWakeOverride
  canEdit: boolean
}>()

const machines = ref<ScheduleMachineResponse[]>([])
const loaded = ref(false)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const draft = ref<Record<number, MachineChoice>>({})

async function load(): Promise<void> {
  const id = props.scheduleId
  try {
    const response = await getScheduleMachines(id)
    if (props.scheduleId !== id) return
    machines.value = response.machines
    loadError.value = null
  } catch (e: unknown) {
    if (props.scheduleId !== id) return
    if (loaded.value) logger.error('background schedule machines refresh failed', e)
    else loadError.value = extractError(e, 'Failed to load the machines this schedule needs')
  } finally {
    loaded.value = true
  }
}

watch(
  () => props.scheduleId,
  () => {
    editing.value = false
    loaded.value = false
    void load()
  },
  { immediate: true },
)

// Targets saved elsewhere on the page change which machines are needed.
const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  if (!editing.value) void load()
})

function optionsFor(machine: ScheduleMachineResponse): SegmentedOption<MachineChoice>[] {
  const options: SegmentedOption<MachineChoice>[] = [
    { value: 'follow', label: 'Same as above' },
    { value: 'wake', label: 'Wake' },
    { value: 'no_wake', label: "Don't wake" },
  ]
  if (machine.can_wait) options.push({ value: 'wait', label: 'Wait for it' })
  return options
}

const CHOICE_TEXT: Record<MachineChoice, string> = {
  follow: 'Same as above',
  wake: 'Wake',
  no_wake: "Don't wake",
  wait: 'Wait for it',
}

function rolesText(machine: ScheduleMachineResponse): string {
  return machine.roles.map(machineRoleLabel).join(', ')
}

function effectOf(machine: ScheduleMachineResponse, mode: MachineWakeMode | null): MachineEffect {
  return scheduleMachineEffect(machine, props.wakeOverride, mode)
}

/** What a row shows: the draft while editing, the saved choice otherwise. */
function modeShown(machine: ScheduleMachineResponse): MachineWakeMode | null {
  return editing.value ? modeOf(draft.value[machine.machine_id] ?? 'follow') : machine.mode
}

const effectRows = computed<MachineEffectRow[]>(() =>
  machines.value.map((machine) => ({
    key: machine.machine_id,
    name: machine.label,
    role: rolesText(machine),
    effect: effectOf(machine, machine.mode),
    choice: machine.mode === null ? undefined : CHOICE_TEXT[machine.mode],
  })),
)

const hasChoices = computed(() => machines.value.some((m) => m.mode !== null))

function startEdit(): void {
  const next: Record<number, MachineChoice> = {}
  for (const machine of machines.value) next[machine.machine_id] = choiceOf(machine.mode)
  draft.value = next
  error.value = null
  editing.value = true
}

/** Only real choices go back: a machine left out follows the schedule-wide setting. */
const payload = computed<ScheduleMachineInput[]>(() =>
  machines.value.flatMap((machine) => {
    const mode = modeOf(draft.value[machine.machine_id] ?? 'follow')
    return mode === null ? [] : [{ machine_id: machine.machine_id, mode }]
  }),
)

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    machines.value = (await updateScheduleMachines(props.scheduleId, payload.value)).machines
    editing.value = false
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="pane-section">
    <div
      v-if="!loaded"
      class="loading-row"
    >
      <BaseSpinner size="sm" />
    </div>
    <div
      v-else-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </div>
    <EditableSection
      v-else
      label="Per machine"
      lede="Override the setting above for one machine, or wait for a machine without waking it:
        its targets run last, and are skipped and caught up if it is still down."
      lede-label="per-machine choices"
      :editing="editing"
      :can-edit="canEdit && machines.length > 0"
      :saving="saving"
      :error="error"
      @edit="startEdit"
      @cancel="editing = false"
      @save="save"
    >
      <template #view>
        <p
          v-if="machines.length === 0"
          class="field-hint"
        >
          Pick this job's agents and repository first, under Targets.
        </p>
        <MachineEffectList
          v-else
          :rows="effectRows"
        />
        <p
          v-if="machines.length > 0 && !hasChoices"
          class="field-hint"
        >
          Every machine follows the setting above.
        </p>
      </template>

      <template #edit>
        <div class="pane-rows">
          <PaneRow
            v-for="machine in machines"
            :key="machine.machine_id"
            :title="machine.label"
            :hint="rolesText(machine)"
            stack
          >
            <BaseSegmented
              v-model="draft[machine.machine_id]"
              :options="optionsFor(machine)"
              :label="`What this job does about ${machine.label}`"
            />
            <p class="field-hint">
              <span
                class="badge"
                :class="badgeClass(effectOf(machine, modeShown(machine)).tone)"
                >{{ effectOf(machine, modeShown(machine)).label }}</span
              >
              {{ effectOf(machine, modeShown(machine)).reason }}
            </p>
          </PaneRow>
        </div>
      </template>
    </EditableSection>
  </section>
</template>

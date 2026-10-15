<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import CatchUpSection from './CatchUpSection.vue'
import CatchUpWaitList from './CatchUpWaitList.vue'
import CatchUpWindowFields from './CatchUpWindowFields.vue'
import { checkMachine, updateMachineAvailability, type Machine } from '../api/machines'
import { useToast } from '../composables/useToast'
import type { MachineWaitResponse } from '../types/generated'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'
import { machineCheckText, scheduleLabel } from '../utils/machine'

/**
 * "When the machine is offline" - whether it is expected to be reachable, and
 * how long a run waits for it when it is not. One answer for every role the
 * machine plays: off, a machine that does not answer fails the run unless a
 * schedule chose to wait for it; on, the run is skipped and caught up.
 *
 * An agent reconnects on its own, so a machine that is only an agent has
 * nothing to be asked on an interval; anything else on it is asked over SSH
 * or on its port, which is why the re-check row depends on the roles.
 */
const props = defineProps<{
  machine: Machine
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [machine: Machine]; checked: [] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const checking = ref(false)

const intermittent = ref(false)
const recheckMinutes = ref(15)
const giveUpMinutes = ref(0)

const { success: toastSuccess, error: toastError } = useToast()

const isAsked = computed(
  () => props.machine.roles.repo_host !== null || props.machine.roles.dependencies.length > 0,
)

/** The settings as `CatchUpSection` reads them, without a re-check for an agent alone. */
const settings = computed(() => ({
  intermittent: props.machine.availability.intermittent,
  catch_up_recheck_minutes: isAsked.value
    ? props.machine.availability.catch_up_recheck_minutes
    : null,
  catch_up_give_up_minutes: props.machine.availability.catch_up_give_up_minutes,
}))

watch(
  () => props.machine.id,
  () => {
    editing.value = false
  },
)

function startEdit(): void {
  const current = props.machine.availability
  intermittent.value = current.intermittent
  recheckMinutes.value = current.catch_up_recheck_minutes
  giveUpMinutes.value = current.catch_up_give_up_minutes
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const updated = await updateMachineAvailability(props.machine.id, {
      intermittent: intermittent.value,
      catch_up_recheck_minutes: recheckMinutes.value,
      catch_up_give_up_minutes: giveUpMinutes.value,
    })
    editing.value = false
    emit('saved', updated)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}

async function checkNow(): Promise<void> {
  checking.value = true
  try {
    toastSuccess(machineCheckText(await checkMachine(props.machine.id)))
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    checking.value = false
    emit('checked')
  }
}

/**
 * A machine lists one wait per schedule and thing waited on: a schedule
 * writing to two repositories on it waits once for each, and a dependency
 * is waited on once per agent.
 */
function waitKey(wait: MachineWaitResponse): string {
  return [
    wait.role,
    wait.schedule_id,
    wait.repo_id ?? '',
    wait.hostname ?? '',
    wait.dependency_name ?? '',
  ].join(':')
}

function waitSubject(wait: MachineWaitResponse): string | null {
  switch (wait.role) {
    case 'agent':
      return wait.hostname
    case 'repo_host':
      return wait.repo_name
    case 'dependency':
      return wait.dependency_name === null || wait.hostname === null
        ? wait.dependency_name
        : `${wait.dependency_name} for ${wait.hostname}`
  }
}

/**
 * What it missed, when it was last and is next asked, and how much of the
 * window is left. A countdown nobody can see only surprises people.
 */
function waitDetail(wait: MachineWaitResponse): string {
  const subject = waitSubject(wait)
  const parts = subject === null ? [] : [subject]
  parts.push(`missed ${relativeTime(wait.pending_for)}`)
  if (wait.role === 'agent') {
    parts.push('runs when the agent reconnects')
  } else {
    parts.push(
      wait.last_probe_at ? `last checked ${relativeTime(wait.last_probe_at)}` : 'not checked yet',
    )
    if (wait.next_probe_at) parts.push(`next check ${relativeTime(wait.next_probe_at)}`)
  }
  if (wait.give_up_at) parts.push(`giving up ${relativeTime(wait.give_up_at)}`)
  return parts.join(' · ')
}
</script>

<template>
  <CatchUpSection
    v-model:intermittent="intermittent"
    :load-error="null"
    :settings="settings"
    marked-hint="A backup that cannot reach this host is reported as skipped and run again once it is back."
    unmarked-hint="A backup that cannot reach this host fails like any other error, unless its schedule waits for it."
    lede="Off, a backup that cannot reach this machine fails. On, it is skipped and run again once
      the machine answers."
    lede-label="an unreachable machine"
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    intermittent-help-label="what an unreachable machine means"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #intermittentHelp>
      For a machine that sleeps, a NAS that is only on part of the day, or a laptop that is off when
      its backups come due. A backup that cannot reach it, even after a wake attempt, is reported as
      <strong>skipped</strong> rather than failed, and run once as soon as it is back. However many
      occurrences it misses, at most one catch-up run follows. Off, a machine that does not answer
      is a <strong>failed backup</strong> - unless a schedule is set to wait for it, which makes
      that one schedule behave as if this were on.
    </template>

    <template #edit>
      <CatchUpWindowFields
        v-model:recheck="recheckMinutes"
        v-model:give-up="giveUpMinutes"
        id-prefix="availability"
        :show-recheck="isAsked"
        recheck-help-label="asking a machine that was away"
        give-up-help-label="bounding how long a run waits for a machine"
      >
        <template #recheckHelp>
          A repository host or a dependency cannot say it is back, so it is asked - over SSH, or on
          its port - on this interval. Every schedule waiting on this machine is caught up on the
          first answer.
        </template>
        <template #giveUpHelp>
          A run waiting on this machine is abandoned once it has waited this long, measured from the
          run it missed, and reported as failed. Separate from a schedule's
          <strong>Mark as failed after</strong>, which counts missed runs and disables the schedule.
        </template>
      </CatchUpWindowFields>
    </template>

    <CatchUpWaitList
      v-if="machine.waiting.length > 0"
      :can-check="canEdit"
      :checking="checking"
      @check="checkNow"
    >
      <div
        v-for="wait in machine.waiting"
        :key="waitKey(wait)"
        class="agent-row"
      >
        <i
          class="agent-row-stripe agent-row-stripe--warning"
          aria-hidden="true"
        />
        <RouterLink
          class="agent-row-name"
          :to="`/schedules/${wait.schedule_id}`"
        >
          {{ scheduleLabel(wait) }}
        </RouterLink>
        <span class="agent-row-stats">{{ waitDetail(wait) }}</span>
      </div>
    </CatchUpWaitList>
  </CatchUpSection>
</template>

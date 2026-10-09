<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import CatchUpSection from './CatchUpSection.vue'
import CatchUpWaitList from './CatchUpWaitList.vue'
import CatchUpWindowFields from './CatchUpWindowFields.vue'
import {
  checkDependencyHostNow,
  getDependencyHostAvailability,
  updateDependencyHostAvailability,
} from '../api/dependencyHosts'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import type {
  DependencyAvailabilityResponse,
  DependencyWaitResponse,
  RepoCatchUpCheckResponse,
} from '../types/generated'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'

/**
 * "When the host is offline" for a dependency - `HostAvailabilityCard`'s
 * counterpart. Off, a backup that cannot reach the machine fails; on, it is
 * skipped and caught up once the machine answers on its port again.
 *
 * A dependency is asked the way a repository host is - on an interval, from
 * the server - so it always has a re-check row and a Check now button. Its
 * waits are per schedule and agent, since each agent mounts its own shares.
 */
const props = defineProps<{
  hostId: number
  /** The port the re-check asks, for the help text. */
  port: number
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [] }>()

const availability = ref<DependencyAvailabilityResponse | null>(null)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const checking = ref(false)

const form = ref({ intermittent: false, recheck: 15, giveUp: 0 })

const { success: toastSuccess, error: toastError } = useToast()

const waits = computed<readonly DependencyWaitResponse[]>(() => availability.value?.waiting ?? [])

async function load(): Promise<void> {
  // A reply for the dependency shown a moment ago is dropped rather than
  // displayed under the one shown now.
  const id = props.hostId
  try {
    const loaded = await getDependencyHostAvailability(id)
    if (props.hostId !== id) return
    availability.value = loaded
    loadError.value = null
  } catch (e: unknown) {
    if (props.hostId !== id) return
    loadError.value = extractError(e)
  }
}

onMounted(load)

watch(
  () => props.hostId,
  () => {
    editing.value = false
    availability.value = null
    void load()
  },
)

// A run that starts waiting, or catches up, changes the list below.
const { onMessage } = useWebSocket()
onMessage('DataChanged', () => {
  void load()
})

function startEdit(): void {
  const current = availability.value
  form.value = {
    intermittent: current?.intermittent ?? false,
    recheck: current?.catch_up_recheck_minutes ?? 15,
    giveUp: current?.catch_up_give_up_minutes ?? 0,
  }
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    availability.value = await updateDependencyHostAvailability(props.hostId, {
      intermittent: form.value.intermittent,
      catch_up_recheck_minutes: form.value.recheck,
      catch_up_give_up_minutes: form.value.giveUp,
    })
    editing.value = false
    emit('saved')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}

function runs(count: number): string {
  return count === 1 ? '1 run' : `${count} runs`
}

/** What Check now found, because "done" leaves the one question worth answering unanswered. */
function checkOutcomeText(outcome: RepoCatchUpCheckResponse): string {
  if (outcome.started > 0) return `The dependency is back - catching up ${runs(outcome.started)}`
  if (outcome.reachable > 0) {
    return 'The dependency is back, but each schedule runs again soon enough on its own'
  }
  if (outcome.probed > 0) return 'The dependency is still not answering'
  if (outcome.abandoned > 0) {
    return `Stopped waiting - ${runs(outcome.abandoned)} past the window, reported as failed`
  }
  return 'Nothing is waiting on this dependency'
}

async function checkNow(): Promise<void> {
  checking.value = true
  try {
    toastSuccess(checkOutcomeText(await checkDependencyHostNow(props.hostId)))
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    checking.value = false
    await load()
  }
}

function waitDetail(wait: DependencyWaitResponse): string {
  if (wait.catching_up) return `missed ${relativeTime(wait.pending_for)} · catching up now`
  const parts = [
    `missed ${relativeTime(wait.pending_for)}`,
    wait.last_probe_at ? `last checked ${relativeTime(wait.last_probe_at)}` : 'not checked yet',
  ]
  if (wait.next_probe_at) parts.push(`next check ${relativeTime(wait.next_probe_at)}`)
  if (wait.give_up_at) parts.push(`giving up ${relativeTime(wait.give_up_at)}`)
  return parts.join(' · ')
}
</script>

<template>
  <CatchUpSection
    v-model:intermittent="form.intermittent"
    :load-error="loadError"
    :settings="availability"
    marked-hint="A backup that cannot reach this dependency is reported as skipped and run again once it answers."
    unmarked-hint="A backup that cannot reach this dependency fails like any other error."
    lede="Off, a backup that cannot reach this machine fails. On, it is skipped and run again once the machine answers."
    lede-label="an unreachable dependency"
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    intermittent-help-label="what an unreachable dependency means"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #intermittentHelp>
      For a NAS that sleeps or a server that is only on part of the day. If this machine is still
      not answering after the wake attempt, the backup is reported as
      <strong>skipped</strong> and run once as soon as it answers. However many runs it misses, at
      most one catch-up run follows. Off, a machine that does not answer is a
      <strong>failed backup</strong>.
    </template>

    <template #edit>
      <CatchUpWindowFields
        v-model:recheck="form.recheck"
        v-model:give-up="form.giveUp"
        id-prefix="dependency"
        show-recheck
        recheck-help-label="asking a dependency that was away"
        give-up-help-label="bounding how long a run waits for a dependency"
      >
        <template #recheckHelp>
          Assimilate checks port {{ port }} on this interval. Every schedule waiting on this machine
          is caught up on the first answer.
        </template>
        <template #giveUpHelp>
          A run waiting on this machine is abandoned once it has waited this long, measured from the
          run it missed, and reported as failed. Separate from a schedule's
          <strong>Mark as failed after</strong>, which counts missed runs and disables the schedule.
        </template>
      </CatchUpWindowFields>
    </template>

    <CatchUpWaitList
      v-if="waits.length > 0"
      :can-check="canEdit"
      :checking="checking"
      @check="checkNow"
    >
      <RouterLink
        v-for="wait in waits"
        :key="`${wait.schedule_id}:${wait.agent_id}`"
        class="agent-row"
        :to="`/schedules/${wait.schedule_id}`"
      >
        <i
          class="agent-row-stripe"
          :class="wait.catching_up ? 'agent-row-stripe--accent' : 'agent-row-stripe--warning'"
          aria-hidden="true"
        />
        <span class="agent-row-name">{{ wait.schedule_name }}</span>
        <span class="muted">on {{ wait.hostname }}</span>
        <span class="agent-row-stats">{{ waitDetail(wait) }}</span>
      </RouterLink>
    </CatchUpWaitList>
  </CatchUpSection>
</template>

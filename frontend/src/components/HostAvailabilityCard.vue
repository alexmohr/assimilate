<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import CatchUpSection from './CatchUpSection.vue'
import CatchUpWaitList from './CatchUpWaitList.vue'
import CatchUpWindowFields from './CatchUpWindowFields.vue'
import type { HostAvailabilityApi } from '../api/availability'
import type {
  CatchUpWaitResponse,
  HostAvailabilityResponse,
  RepoCatchUpCheckResponse,
} from '../types/generated'
import { extractError } from '../utils/error'
import { relativeTime } from '../utils/format'
import { useToast } from '../composables/useToast'

/**
 * "When the host is offline" - whether this agent's or repository's host is
 * expected to be reachable, and how long to wait for it when it is not.
 *
 * One section for both, because the switch means the same thing on each:
 * off, an unreachable host is a failed backup; on, it is an expected skip
 * that is caught up once the host is back. The only difference is how "back"
 * is found out. An agent reconnects on its own, so there is nothing to ask it;
 * a repository cannot say anything, so it is asked over SSH on an interval -
 * which is why only a repository gets a re-check row and a Check now button,
 * and why `api.check` is optional.
 */
const props = defineProps<{
  api: HostAvailabilityApi
  canEdit: boolean
}>()

const availability = ref<HostAvailabilityResponse | null>(null)
const loadError = ref<string | null>(null)

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)
const checking = ref(false)

const intermittent = ref(false)
const recheckMinutes = ref(15)
const giveUpMinutes = ref(0)

const { success: toastSuccess, error: toastError } = useToast()

const asksOverSsh = computed(() => props.api.check !== undefined)
const waiting = computed<readonly CatchUpWaitResponse[]>(() => availability.value?.waiting ?? [])

async function load(): Promise<void> {
  // A reply for the host this section was showing a moment ago is dropped
  // rather than displayed under the one it shows now.
  const host = props.api.host
  try {
    const loaded = await props.api.load()
    if (props.api.host !== host) return
    availability.value = loaded
    loadError.value = null
  } catch (e: unknown) {
    if (props.api.host !== host) return
    loadError.value = extractError(e)
  }
}

onMounted(load)

// The page can switch to another host without remounting this section (an
// agent with the same hostname in another domain differs only by a query
// parameter). Anything shown or half-edited belongs to the previous host, and
// saving it would write that host's settings onto this one.
watch(
  () => props.api.host,
  () => {
    editing.value = false
    availability.value = null
    void load()
  },
)

function startEdit(): void {
  const current = availability.value
  intermittent.value = current?.intermittent ?? false
  recheckMinutes.value = current?.catch_up_recheck_minutes ?? 15
  giveUpMinutes.value = current?.catch_up_give_up_minutes ?? 0
  error.value = null
  editing.value = true
}

async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    availability.value = await props.api.save({
      intermittent: intermittent.value,
      catch_up_recheck_minutes: asksOverSsh.value ? recheckMinutes.value : undefined,
      catch_up_give_up_minutes: giveUpMinutes.value,
    })
    editing.value = false
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}

/**
 * Says what the check actually found, because "done" leaves the one question
 * worth answering - is it back? - unanswered.
 */
function checkOutcomeText(outcome: RepoCatchUpCheckResponse): string {
  if (outcome.probed === 0) {
    // Nothing got as far as a probe - but a wait past its window is given up
    // on before that, and reported as a failed backup; say so rather than
    // claim there was nothing to do.
    if (outcome.abandoned > 0) {
      return outcome.abandoned === 1
        ? 'Stopped waiting - 1 run was past its window and is reported as failed'
        : `Stopped waiting - ${outcome.abandoned} runs were past their window and are reported as failed`
    }
    return 'Nothing is waiting on this repository'
  }
  if (outcome.started > 0) {
    return outcome.started === 1
      ? 'The host is back - catching up 1 run'
      : `The host is back - catching up ${outcome.started} runs`
  }
  if (outcome.reachable > 0) {
    return 'The host is back, but each schedule runs again soon enough on its own'
  }
  return 'The host is still not answering'
}

async function checkNow(): Promise<void> {
  const check = props.api.check
  if (!check) return
  checking.value = true
  try {
    toastSuccess(checkOutcomeText(await check()))
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    checking.value = false
    await load()
  }
}

/**
 * One line per waiting schedule: what it missed, when the host was last and
 * is next asked (a repository only), and how much of the window is left. A
 * countdown nobody can see is a countdown that only surprises people.
 */
/**
 * A host lists one wait per schedule and repository: a schedule writing to
 * two repositories on the same machine waits once for each. An agent's waits
 * carry no repository.
 */
function waitKey(wait: CatchUpWaitResponse): string {
  return `${wait.schedule_id}:${wait.repo_id ?? ''}`
}

function waitDetail(wait: CatchUpWaitResponse): string {
  const parts = wait.repo_name === null ? [] : [wait.repo_name]
  parts.push(`missed ${relativeTime(wait.pending_for)}`)
  if (asksOverSsh.value) {
    parts.push(
      wait.last_probe_at ? `last checked ${relativeTime(wait.last_probe_at)}` : 'not checked yet',
    )
    if (wait.next_probe_at) parts.push(`next check ${relativeTime(wait.next_probe_at)}`)
  } else {
    parts.push('runs when the agent reconnects')
  }
  if (wait.give_up_at) parts.push(`giving up ${relativeTime(wait.give_up_at)}`)
  return parts.join(' · ')
}
</script>

<template>
  <CatchUpSection
    v-model:intermittent="intermittent"
    :load-error="loadError"
    :settings="availability"
    marked-hint="A backup that cannot reach this host is reported as skipped and run again once it is back."
    unmarked-hint="A backup that cannot reach this host fails like any other error."
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    intermittent-help-label="what an unreachable host means"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #intermittentHelp>
      For a machine that sleeps, a VM that boots on its own, or a laptop that is off when its
      backups come due. A backup that fails because this host is not there is reported as
      <strong>skipped</strong> rather than failed, and run once as soon as it is back. However many
      occurrences it misses, at most one catch-up run follows. Off, an unreachable host is a
      <strong>failed backup</strong> like any other error, because for a machine that should always
      be up it is one.
    </template>

    <template #edit>
      <CatchUpWindowFields
        v-model:recheck="recheckMinutes"
        v-model:give-up="giveUpMinutes"
        id-prefix="availability"
        :show-recheck="asksOverSsh"
        recheck-help-label="asking a host that was away"
        give-up-help-label="bounding how long a catch-up stays pending"
      >
        <template #recheckHelp>
          A repository has no connection to the server and cannot say it is back, so it is asked
          over SSH on this interval. Every schedule waiting here is caught up on the first answer.
        </template>
        <template #giveUpHelp>
          A pending catch-up is abandoned once it has waited this long, measured from the run it
          missed, and the run is reported as failed - the backup is not going to happen. Separate
          from a schedule's <strong>Mark as failed after</strong>, which counts missed runs and
          disables the schedule.
        </template>
      </CatchUpWindowFields>
    </template>

    <CatchUpWaitList
      v-if="waiting.length > 0"
      :can-check="api.check !== undefined && canEdit"
      :checking="checking"
      @check="checkNow"
    >
      <div
        v-for="wait in waiting"
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
          {{ wait.schedule_name }}
        </RouterLink>
        <span class="agent-row-stats">{{ waitDetail(wait) }}</span>
      </div>
    </CatchUpWaitList>
  </CatchUpSection>
</template>

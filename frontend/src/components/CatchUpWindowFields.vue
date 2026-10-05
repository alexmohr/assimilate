<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import DurationField from './DurationField.vue'
import PaneRow from './PaneRow.vue'
import { humanizeMinutes } from '../utils/duration'

/**
 * The two rows nested under "Host is not always online": how often a host
 * that was away is asked whether it is back, and how long a run waits for it.
 * Shared by the repository host's and the dependency's sections, which say
 * different things about each row (the `recheckHelp` and `giveUpHelp` slots)
 * but must store them the same way. An agent announces its own return, so it
 * has no re-check row.
 */
const props = defineProps<{
  /** Prefix of the two inputs' ids, e.g. `availability` for `availability-recheck`. */
  idPrefix: string
  showRecheck: boolean
  /** `HelpHint` labels for the two rows. */
  recheckHelpLabel: string
  giveUpHelpLabel: string
}>()

const recheck = defineModel<number>('recheck', { required: true })
const giveUp = defineModel<number>('giveUp', { required: true })

/** The units each field offers, finest first - see `ScheduleSettingsTab`. */
const RECHECK_UNITS = ['minutes', 'hours', 'days'] as const
// Minutes first as the fallback: the API takes any whole number of minutes,
// and a window that is not a whole number of hours must still read as one.
const GIVE_UP_UNITS = ['minutes', 'hours', 'days', 'weeks'] as const

const giveUpHint = computed(() =>
  giveUp.value === 0
    ? 'Leave empty to wait indefinitely.'
    : `Reported as a failed backup after ${humanizeMinutes(giveUp.value)}.`,
)
</script>

<template>
  <div class="pane-nest">
    <PaneRow
      v-if="showRecheck"
      title="Re-check every"
      :label-for="`${props.idPrefix}-recheck`"
      :help="recheckHelpLabel"
      stack
    >
      <template #help>
        <slot name="recheckHelp" />
      </template>
      <DurationField
        v-model="recheck"
        :input-id="`${props.idPrefix}-recheck`"
        :units="RECHECK_UNITS"
        unit-label="Re-check interval unit"
      />
    </PaneRow>

    <PaneRow
      title="Stop waiting after"
      :label-for="`${props.idPrefix}-give-up`"
      :help="giveUpHelpLabel"
      :hint="giveUpHint"
      stack
    >
      <template #help>
        <slot name="giveUpHelp" />
      </template>
      <DurationField
        v-model="giveUp"
        :input-id="`${props.idPrefix}-give-up`"
        :units="GIVE_UP_UNITS"
        unit-label="Give-up window unit"
        clearable
      />
    </PaneRow>
  </div>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The schedules that overrule the power settings of the host you are looking
 * at - they wake it whatever its own toggle says, so the toggle alone would
 * misrepresent what actually happens.
 *
 * Each schedule is a row of its own rather than a run of links sharing one
 * wrapped line: on a phone three links in a sentence are not separable tap
 * targets. Renders nothing when no schedule overrides.
 *
 * A component because the agent's power pane and the repository's power pane
 * had the same block character for character - the duplication this pane's
 * own `PaneRow` extraction exists to stop.
 */
import { ChevronRight, CornerDownRight } from '@lucide/vue'
import type { ScheduleRow } from '../types/schedule'

defineProps<{
  /** The schedules that override this host, in the order to list them. */
  schedules: ScheduleRow[]
}>()
</script>

<template>
  <div
    v-if="schedules.length > 0"
    class="override-note"
  >
    <p class="override-lead">
      <CornerDownRight :size="14" />
      <span>
        {{ schedules.length }}
        {{ schedules.length === 1 ? 'schedule wakes' : 'schedules wake' }} this host whatever the
        setting above says
      </span>
    </p>
    <div class="override-links">
      <RouterLink
        v-for="s in schedules"
        :key="s.id"
        class="override-link"
        :to="`/schedules/${s.id}?tab=settings&section=power`"
      >
        <span>{{ s.name || `Schedule #${s.id}` }}</span>
        <ChevronRight :size="14" />
      </RouterLink>
    </div>
  </div>
</template>

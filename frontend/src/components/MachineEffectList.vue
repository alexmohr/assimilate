<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { badgeClass } from '../utils/badge'
import type { MachineEffect } from '../utils/machine'

/** One machine a job needs, and what the job's power settings resolve to for it. */
export interface MachineEffectRow {
  key: string | number
  name: string
  /** What the job needs it as. */
  role: string
  effect: MachineEffect
  /** The job's own choice for it, when it made one. */
  choice?: string
}

/**
 * The read-out under a job's power settings: per machine, a badge and the
 * sentence saying what happens to it, so the answer is on screen rather than
 * reconstructed from the machine's own page.
 */
defineProps<{ rows: readonly MachineEffectRow[] }>()
</script>

<template>
  <dl class="info-grid">
    <template
      v-for="row in rows"
      :key="row.key"
    >
      <dt>
        <span class="mono">{{ row.name }}</span>
        <span class="host-role muted">{{ row.role }}</span>
      </dt>
      <dd class="host-effect">
        <span>
          <span
            class="badge"
            :class="badgeClass(row.effect.tone)"
            >{{ row.effect.label }}</span
          >
          <span
            v-if="row.choice"
            class="host-role muted"
            >{{ row.choice }}</span
          >
        </span>
        <span class="host-reason muted">{{ row.effect.reason }}</span>
      </dd>
    </template>
  </dl>
</template>

<style scoped>
.host-role {
  margin-left: var(--space-4);
}

.host-effect {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-3);
}

.host-reason {
  line-height: 1.5;
  max-width: 52ch;
}
</style>

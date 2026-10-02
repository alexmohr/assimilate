<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { RefreshCw } from '@lucide/vue'
import PaneRow from './PaneRow.vue'

/**
 * "Waiting to catch up": the runs a host that was away still owes, and the
 * Check now that asks it whether it is back. Shared by every "When the host
 * is offline" section - a repository host's, an agent's and a dependency's -
 * which differ only in what one waiting run says about itself, so the rows
 * are the caller's.
 */
defineProps<{
  /** Whether Check now is offered: only a host the server asks, and only to an admin. */
  canCheck: boolean
  checking: boolean
}>()

const emit = defineEmits<{ check: [] }>()
</script>

<template>
  <PaneRow
    title="Waiting to catch up"
    stack
  >
    <template
      v-if="canCheck"
      #titleAside
    >
      <button
        type="button"
        class="btn btn-sm"
        :disabled="checking"
        @click="emit('check')"
      >
        <RefreshCw
          :size="14"
          :class="{ spinning: checking }"
        />
        {{ checking ? 'Checking...' : 'Check now' }}
      </button>
    </template>
    <div class="rows">
      <slot />
    </div>
  </PaneRow>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { AlertTriangle, Lock } from '@lucide/vue'

/**
 * How a repository's append-only state reads on a card and in the page
 * header: the purple badge, plus the amber one when maintenance stopped
 * because archives vanished that Assimilate did not delete.
 */
defineProps<{
  appendOnly: boolean
  maintenanceStopped: boolean
}>()
</script>

<template>
  <template v-if="appendOnly">
    <span
      class="badge badge--violet"
      title="Agents can add archives but not delete them"
    >
      <Lock :size="12" />
      Append-only
    </span>
    <span
      v-if="maintenanceStopped"
      class="badge badge--warning"
      title="Archives vanished that Assimilate did not delete"
    >
      <AlertTriangle :size="12" />
      Maintenance stopped
    </span>
  </template>
</template>

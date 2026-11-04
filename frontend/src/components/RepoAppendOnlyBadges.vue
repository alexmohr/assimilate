<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { AlertTriangle, Lock } from '@lucide/vue'
import type { RepoAppendOnlyResponse } from '../types/generated'

/**
 * How a repository's append-only state reads on a card and in the page
 * header: the purple badge, plus the amber one when maintenance stopped
 * because archives vanished that Assimilate did not delete.
 */
defineProps<{
  /** What the repository list says of it; absent when it is not append-only. */
  appendOnly?: RepoAppendOnlyResponse | null
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
      v-if="appendOnly.maintenance_stopped"
      class="badge badge--warning"
      title="Archives vanished that Assimilate did not delete"
    >
      <AlertTriangle :size="12" />
      Maintenance stopped
    </span>
  </template>
</template>

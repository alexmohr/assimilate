<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * A labelled block per selected agent, used by the schedule form's per-agent
 * overrides. The same "one column of agent-labelled fields plus a hint"
 * markup was repeated for excludes, file change patterns and commands.
 */
defineProps<{
  agentIds: number[]
  /** Display name for an agent id, resolved by the owning view. */
  agentLabel: (id: number) => string
}>()

defineSlots<{
  default: (props: { agentId: number }) => unknown
  hint?: () => unknown
}>()
</script>

<template>
  <div class="per-host-paths">
    <div
      v-for="agentId in agentIds"
      :key="agentId"
      class="per-host-entry"
    >
      <label class="field-label">{{ agentLabel(agentId) }}</label>
      <slot :agent-id="agentId" />
    </div>
    <span
      v-if="$slots.hint"
      class="field-hint"
    >
      <slot name="hint" />
    </span>
  </div>
</template>

<style scoped></style>

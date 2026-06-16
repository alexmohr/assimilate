<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
defineProps<{
  hostname: string
}>()
</script>

<template>
  <RouterLink :to="`/agents/${encodeURIComponent(hostname)}`">
    {{ hostname }}
  </RouterLink>
</template>

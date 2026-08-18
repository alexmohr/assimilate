<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import ErrorPage from '../components/ErrorPage.vue'
</script>

<template>
  <ErrorPage
    code="404"
    tone="accent"
    title="Page not found"
    message="The page you're looking for doesn't exist or has been moved."
  />
</template>

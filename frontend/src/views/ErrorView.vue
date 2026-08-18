<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import CardError from '../components/CardError.vue'
import ErrorPage from '../components/ErrorPage.vue'
import { consumeErrorDetails } from '../utils/errorDetails'

const route = useRoute()

const statusCode = String(route.query.code ?? '500')
const message = String(route.query.message ?? 'Something went wrong. Please try again later.')

const errorDetails = consumeErrorDetails()

const sourceLabel = computed(() => {
  if (errorDetails?.source === 'frontend') {
    return 'Frontend error'
  }
  if (errorDetails?.source === 'backend') {
    return 'Backend error'
  }
  return undefined
})

const detailsMessage = computed(() => {
  if (!errorDetails) {
    return undefined
  }
  const lines = [
    errorDetails.name ? `Type: ${errorDetails.name}` : undefined,
    `Message: ${errorDetails.message}`,
    errorDetails.stack ? `\nStack trace:\n${errorDetails.stack}` : undefined,
  ].filter((line): line is string => line !== undefined)
  return lines.join('\n')
})
</script>

<template>
  <ErrorPage
    :code="statusCode"
    title="Error"
    :message="message"
  >
    <template
      v-if="sourceLabel"
      #source
    >
      <p class="error-source">{{ sourceLabel }}</p>
    </template>
    <CardError
      v-if="detailsMessage"
      class="error-details"
      label="Show error details"
      :message="detailsMessage"
    />
  </ErrorPage>
</template>

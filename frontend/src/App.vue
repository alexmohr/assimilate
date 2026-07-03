<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { onErrorCaptured } from 'vue'
import AppLayout from './layouts/AppLayout.vue'
import ToastContainer from './components/ToastContainer.vue'
import { logger } from './utils/logger'
import { storeErrorDetails } from './utils/errorDetails'

const route = useRoute()
const router = useRouter()

const routerReady = ref(false)
router.isReady().then(() => {
  routerReady.value = true
})

const FULL_PAGE_ROUTE_NAMES = new Set(['login', 'not-found', 'error'])

const isFullPage = computed(
  () => typeof route.name === 'string' && FULL_PAGE_ROUTE_NAMES.has(route.name),
)

// Only redirect to the error page for unrecoverable render errors (TypeError, ReferenceError, etc.).
// Async fetch errors are handled locally in each view and must not trigger a full-page redirect.
onErrorCaptured((err) => {
  if (err instanceof TypeError || err instanceof ReferenceError || err instanceof SyntaxError) {
    logger.error('Unrecoverable render error', err)
    storeErrorDetails({
      source: 'frontend',
      name: err.name,
      message: err.message,
      stack: err.stack,
    })
    router.push({ name: 'error', query: { message: 'An unexpected error occurred.' } })
    return false
  }
  // Let other errors (e.g. rejected Promises from event handlers) propagate normally.
  return true
})
</script>

<template>
  <template v-if="routerReady">
    <RouterView v-if="isFullPage" />
    <AppLayout v-else />
  </template>
  <ToastContainer />
</template>

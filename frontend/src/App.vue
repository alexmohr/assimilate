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


const route = useRoute()
const router = useRouter()

const routerReady = ref(false)
router.isReady().then(() => {
  routerReady.value = true
})

const isFullPage = computed(
  () => route.name === 'login' || route.name === 'not-found' || route.name === 'error',
)

onErrorCaptured(() => {
  router.push({ name: 'error', query: { message: 'An unexpected error occurred.' } })
  return false
})
</script>

<template>
  <template v-if="routerReady">
    <RouterView v-if="isFullPage" />
    <AppLayout v-else />
  </template>
  <ToastContainer />
</template>

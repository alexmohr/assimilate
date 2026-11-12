<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { Plus } from '@lucide/vue'
import { useAuthStore } from '../stores/auth'
import DependencyHostsTab from '../components/DependencyHostsTab.vue'

/**
 * Every machine a backup needs besides its agent and its repository. Readable
 * by everyone signed in, like each dependency's own page; only admins create.
 */
const router = useRouter()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)

/** A new dependency is made on its own wizard page, like a new schedule. */
function openCreate(): void {
  router.push('/dependencies/new')
}
</script>

<template>
  <div class="dependencies-view">
    <div class="page-header">
      <h1 class="page-title">Dependencies</h1>
      <div
        v-if="isAdmin"
        class="header-actions"
      >
        <button
          class="btn btn-primary"
          @click="openCreate"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>
    <p class="page-description">
      Hosts a backup needs before it can run. Runs wait for them, and Assimilate can wake them.
    </p>

    <DependencyHostsTab
      :is-admin="isAdmin"
      @new="openCreate"
    />
  </div>
</template>

<style scoped>
.dependencies-view {
  max-width: 1100px;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { Plus } from '@lucide/vue'
import { useAuthStore } from '../stores/auth'
import type { DependencyHost } from '../api/dependencyHosts'
import DependencyCreateDialog from '../components/DependencyCreateDialog.vue'
import DependencyHostsTab from '../components/DependencyHostsTab.vue'

/**
 * Every machine a backup needs besides its agent and its repository. Readable
 * by everyone signed in, like each dependency's own page; only admins create.
 */
const router = useRouter()
const authStore = useAuthStore()
const isAdmin = computed(() => authStore.isAdmin)

const showDialog = ref(false)

function onCreated(host: DependencyHost): void {
  showDialog.value = false
  router.push(`/dependency-hosts/${host.id}`)
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
          @click="showDialog = true"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <DependencyHostsTab
      :is-admin="isAdmin"
      @new="showDialog = true"
    />

    <DependencyCreateDialog
      v-if="isAdmin"
      :open="showDialog"
      @close="showDialog = false"
      @created="onCreated"
    />
  </div>
</template>

<style scoped>
.dependencies-view {
  max-width: 1100px;
}
</style>

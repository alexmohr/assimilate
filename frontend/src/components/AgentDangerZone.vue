<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'
import BaseModal from './BaseModal.vue'
import type { AgentRow } from '../types/agent'

/**
 * The destructive actions on a host, plus their confirmation dialogs.
 * Imported hosts get a different pair (hide / delete archives) from managed
 * ones (delete agent).
 */
const props = defineProps<{ agent: AgentRow }>()

const router = useRouter()

const showDeleteDialog = ref(false)
const deleteLoading = ref(false)

async function confirmDeleteHost(): Promise<void> {
  deleteLoading.value = true
  try {
    await apiClient.delete(`/agents/${props.agent.hostname}`)
    router.push('/agents')
  } catch (e: unknown) {
    logger.error('Failed to delete host', e)
  } finally {
    deleteLoading.value = false
  }
}

// Hide imported agent
const hideLoading = ref(false)

async function hideAgent(): Promise<void> {
  hideLoading.value = true
  try {
    await apiClient.put(`/agents/${props.agent.hostname}/hide`)
    router.push('/agents')
  } catch (e: unknown) {
    logger.error('Failed to hide agent', e)
  } finally {
    hideLoading.value = false
  }
}

// Delete archives & remove imported agent
const showDeleteArchivesDialog = ref(false)
const deleteArchivesLoading = ref(false)

async function confirmDeleteArchives(): Promise<void> {
  deleteArchivesLoading.value = true
  try {
    await apiClient.post(`/agents/${props.agent.hostname}/delete-archives`)
    router.push('/agents')
  } catch (e: unknown) {
    logger.error('Failed to delete archives', e)
  } finally {
    deleteArchivesLoading.value = false
  }
}
</script>

<template>
  <template v-if="agent.is_imported">
    <p class="pane-lede">
      This host was discovered in a repository rather than registered by an agent. Hiding keeps its
      archives and drops it from the list; deleting destroys the archives themselves.
    </p>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Hide agent</span>
        <span class="danger-desc">Stays in the repository, out of the default list view.</span>
      </div>
      <button
        class="btn btn-sm btn-ghost"
        :disabled="hideLoading"
        @click="hideAgent"
      >
        {{ hideLoading ? 'Hiding...' : 'Hide' }}
      </button>
    </div>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Delete archives and remove</span>
        <span class="danger-desc">Destroys every borg archive belonging to this host.</span>
      </div>
      <button
        class="btn btn-sm btn-danger"
        :disabled="deleteArchivesLoading"
        @click="showDeleteArchivesDialog = true"
      >
        {{ deleteArchivesLoading ? 'Deleting...' : 'Delete archives' }}
      </button>
    </div>
  </template>
  <template v-else>
    <p class="pane-lede">
      Deleting <span class="mono">{{ agent.hostname }}</span> removes its schedules, its backup
      reports and its token. Archives already written to a repository are not touched.
    </p>
    <div class="danger-body">
      <div class="danger-info">
        <span class="danger-heading">Delete agent</span>
        <span class="danger-desc">Cannot be undone.</span>
      </div>
      <button
        class="btn btn-sm btn-danger"
        :disabled="deleteLoading"
        @click="showDeleteDialog = true"
      >
        {{ deleteLoading ? 'Deleting...' : 'Delete' }}
      </button>
    </div>
  </template>

  <!-- Delete agent confirmation dialog -->
  <BaseModal
    :open="showDeleteDialog"
    title="Delete agent"
    @close="showDeleteDialog = false"
  >
    <p>
      Permanently delete <strong>{{ agent.hostname }}</strong
      >? All associated schedules and backup reports will be removed. This action cannot be undone.
    </p>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showDeleteDialog = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="deleteLoading"
        @click="confirmDeleteHost"
      >
        {{ deleteLoading ? 'Deleting...' : 'Delete agent' }}
      </button>
    </template>
  </BaseModal>

  <!-- Delete archives confirmation dialog -->
  <BaseModal
    :open="showDeleteArchivesDialog"
    title="Delete archives and remove agent"
    @close="showDeleteArchivesDialog = false"
  >
    <p class="danger-warning-text">
      This will <strong>permanently destroy all borg archives</strong> belonging to
      <strong>{{ agent.hostname }}</strong> and remove the agent from the system.
    </p>
    <p class="danger-warning-text">
      This operation is <strong>irreversible</strong>. Backup data will be permanently lost and
      cannot be recovered.
    </p>

    <template #footer>
      <button
        class="btn btn-ghost"
        @click="showDeleteArchivesDialog = false"
      >
        Cancel
      </button>
      <button
        class="btn btn-danger"
        :disabled="deleteArchivesLoading"
        @click="confirmDeleteArchives"
      >
        {{ deleteArchivesLoading ? 'Deleting...' : 'Delete archives and remove' }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.danger-warning-text {
  font-size: var(--fs-base);
  color: var(--danger);
  margin-bottom: 0.5rem;
}
</style>

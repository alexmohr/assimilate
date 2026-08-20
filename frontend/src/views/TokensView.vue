<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted } from 'vue'
import { useApiTokens } from '../composables/useApiTokens'
import { Plus, Key } from '@lucide/vue'
import ApiTokenTable from '../components/ApiTokenTable.vue'
import AsyncSection from '../components/AsyncSection.vue'
import EmptyState from '../components/EmptyState.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import BaseModal from '../components/BaseModal.vue'

const {
  tokens,
  loading,
  loadError,
  showCreateModal,
  createName,
  createError,
  createSubmitting,
  newTokenPlaintext,
  tokenCopied,
  copyToClipboard,
  showDeleteModal,
  deleteTarget,
  deleteSubmitting,
  deleteError,
  fetchTokens,
  openCreate,
  submitCreate,
  closeCreateModal,
  openDelete,
  confirmDelete,
} = useApiTokens()

onMounted(fetchTokens)
</script>

<template>
  <div class="tokens-page">
    <div class="page-header">
      <h1 class="page-title">API Tokens</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openCreate"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <AsyncSection
      :loading="loading"
      :error="loadError"
      :empty="tokens.length === 0"
    >
      <ApiTokenTable
        :tokens="tokens"
        @delete="openDelete"
      />
      <template #empty>
        <EmptyState
          :icon="Key"
          title="No API tokens"
          description="Create one to get started."
          action="New token"
          @action="showCreateModal = true"
        />
      </template>
    </AsyncSection>

    <!-- One dialog, two states: collect a name, then reveal the token once. -->
    <BaseModal
      :open="showCreateModal"
      :title="newTokenPlaintext ? 'Token created' : 'New API token'"
      :form="!newTokenPlaintext"
      @close="closeCreateModal"
      @submit="submitCreate"
    >
      <template v-if="!newTokenPlaintext">
        <div class="field">
          <label
            class="field-label"
            for="token-name"
            >Token name</label
          >
          <input
            id="token-name"
            v-model="createName"
            type="text"
            class="input"
            required
            placeholder="e.g. CI pipeline"
          />
        </div>
        <div
          v-if="createError"
          class="form-error"
        >
          {{ createError }}
        </div>
      </template>
      <div
        v-else
        class="token-notice"
      >
        <p class="token-warning">Copy this token now. It will not be shown again.</p>
        <div class="token-box">
          <code class="token-text">{{ newTokenPlaintext }}</code>
          <button
            type="button"
            class="btn btn-sm"
            @click="copyToClipboard(newTokenPlaintext)"
          >
            {{ tokenCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </div>

      <template #footer>
        <ModalFormActions
          v-if="!newTokenPlaintext"
          :submitting="createSubmitting"
          :disabled="!createName.trim()"
          submit-label="Create"
          submitting-label="Creating..."
          @cancel="closeCreateModal"
        />
        <button
          v-else
          type="button"
          class="btn btn-primary"
          @click="closeCreateModal"
        >
          Done
        </button>
      </template>
    </BaseModal>

    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete token"
      :submitting="deleteSubmitting"
      :error="deleteError"
      @cancel="showDeleteModal = false"
      @confirm="confirmDelete"
    >
      Are you sure you want to delete token <strong>{{ deleteTarget?.name }}</strong
      >? This action cannot be undone.
    </ConfirmDeleteDialog>
  </div>
</template>

<style scoped>
.tokens-page {
  max-width: 900px;
}
</style>

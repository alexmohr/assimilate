<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { onMounted } from 'vue'
import { useApiTokens } from '../composables/useApiTokens'
import { Plus, Key } from '@lucide/vue'
import ApiTokenTable from '../components/ApiTokenTable.vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'

const {
  tokens,
  loading,
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

    <BaseSpinner
      v-if="loading"
      size="lg"
    />

    <ApiTokenTable
      v-else-if="tokens.length"
      :tokens="tokens"
      @delete="openDelete"
    />

    <EmptyState
      v-else
      :icon="Key"
      title="No API tokens"
      description="Create one to get started."
      action="Create Token"
      @action="showCreateModal = true"
    />

    <div
      v-if="showCreateModal"
      class="overlay"
      @click.self="closeCreateModal"
    >
      <div class="dialog">
        <template v-if="!newTokenPlaintext">
          <div class="dialog-header">
            <h2 class="dialog-title">Create API Token</h2>
            <button
              class="close-btn"
              @click="closeCreateModal"
            >
              &times;
            </button>
          </div>
          <form @submit.prevent="submitCreate">
            <div class="dialog-body">
              <div class="field">
                <label
                  class="field-label"
                  for="token-name"
                  >Token Name</label
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
            </div>
            <ModalFormActions
              :submitting="createSubmitting"
              :disabled="!createName.trim()"
              :error="createError"
              submit-label="Create"
              submitting-label="Create"
              @cancel="closeCreateModal"
            />
          </form>
        </template>
        <template v-else>
          <div class="dialog-header">
            <h2 class="dialog-title">Token Created</h2>
          </div>
          <div class="dialog-body">
            <div class="token-notice">
              <p class="token-warning">Copy this token now. It will not be shown again.</p>
              <div class="token-box">
                <code class="token-text">{{ newTokenPlaintext }}</code>
                <button
                  class="btn btn-sm"
                  @click="copyToClipboard(newTokenPlaintext)"
                >
                  {{ tokenCopied ? 'Copied!' : 'Copy' }}
                </button>
              </div>
            </div>
          </div>
          <div class="dialog-footer">
            <button
              class="btn btn-primary"
              @click="closeCreateModal"
            >
              Done
            </button>
          </div>
        </template>
      </div>
    </div>

    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete Token"
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

.token-notice {
  margin: 1rem 0;
}

.token-warning {
  font-size: 0.8125rem;
  color: var(--warning, var(--text-secondary));
  font-weight: 600;
  margin-bottom: 0.5rem;
}

.token-box {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem 0.75rem;
}

.token-text {
  flex: 1;
  font-size: 0.75rem;
  font-family: monospace;
  word-break: break-all;
  color: var(--text-primary);
}
</style>

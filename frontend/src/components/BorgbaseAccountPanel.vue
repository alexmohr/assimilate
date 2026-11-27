<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * The System page's BorgBase panel: the account's API token, the repositories on the account
 * with the quota and usage BorgBase reports, and a form that creates a repository for the
 * server's own SSH key. BorgBase gives every repository its own host name, so the token belongs
 * to the account rather than to one repository host.
 */
import { computed, onMounted, reactive, ref } from 'vue'
import {
  createBorgbaseRepo,
  deleteBorgbaseToken,
  getBorgbaseStatus,
  listBorgbaseRepos,
  setBorgbaseToken,
} from '../api/borgbase'
import { extractError } from '../utils/error'
import BaseSpinner from './BaseSpinner.vue'
import ConfirmDeleteDialog from './ConfirmDeleteDialog.vue'
import EmptyState from './EmptyState.vue'
import PaneRow from './PaneRow.vue'
import type {
  BorgbaseKeyAccess,
  BorgbaseRegion,
  BorgbaseRepo,
  CreatedBorgbaseRepo,
} from '../types/generated'

/** Inside a settings section, which already frames and names it. */
defineProps<{ embedded?: boolean }>()

const loading = ref(true)
const error = ref('')
const tokenSet = ref(false)
const repos = ref<BorgbaseRepo[]>([])
const reposError = ref('')

const tokenInput = ref('')
const savingToken = ref(false)
const tokenError = ref('')

const showRemove = ref(false)
const removing = ref(false)
const removeError = ref('')

const form = reactive<{ name: string; region: BorgbaseRegion; access: BorgbaseKeyAccess }>({
  name: '',
  region: 'eu',
  access: 'append_only',
})
const creating = ref(false)
const createError = ref('')
const created = ref<CreatedBorgbaseRepo | null>(null)

const canCreate = computed(() => form.name.trim() !== '' && !creating.value)

/** BorgBase does not document the unit of these numbers, so they are shown as it reports them. */
function formatFigure(value: number | null): string {
  return value === null ? 'None' : String(value)
}

async function loadRepos(): Promise<void> {
  reposError.value = ''
  try {
    repos.value = await listBorgbaseRepos()
  } catch (e: unknown) {
    repos.value = []
    reposError.value = extractError(e, 'Failed to load the BorgBase repositories')
  }
}

onMounted(async () => {
  try {
    tokenSet.value = (await getBorgbaseStatus()).token_set
    if (tokenSet.value) {
      await loadRepos()
    }
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load the BorgBase account')
  } finally {
    loading.value = false
  }
})

async function saveToken(): Promise<void> {
  savingToken.value = true
  tokenError.value = ''
  try {
    tokenSet.value = (await setBorgbaseToken(tokenInput.value)).token_set
    tokenInput.value = ''
    await loadRepos()
  } catch (e: unknown) {
    tokenError.value = extractError(e, 'Failed to save the BorgBase API token')
  } finally {
    savingToken.value = false
  }
}

async function removeToken(): Promise<void> {
  removing.value = true
  removeError.value = ''
  try {
    await deleteBorgbaseToken()
    tokenSet.value = false
    repos.value = []
    reposError.value = ''
    created.value = null
    showRemove.value = false
  } catch (e: unknown) {
    removeError.value = extractError(e, 'Failed to remove the BorgBase API token')
  } finally {
    removing.value = false
  }
}

async function createRepo(): Promise<void> {
  creating.value = true
  createError.value = ''
  created.value = null
  try {
    created.value = await createBorgbaseRepo({
      name: form.name.trim(),
      region: form.region,
      access: form.access,
    })
    form.name = ''
    await loadRepos()
  } catch (e: unknown) {
    createError.value = extractError(e, 'Failed to create the BorgBase repository')
  } finally {
    creating.value = false
  }
}
</script>

<template>
  <div :class="embedded ? 'settings-pane' : 'panel'">
    <div
      v-if="!embedded"
      class="panel-header"
    >
      <h2 class="panel-title">BorgBase</h2>
    </div>
    <p class="pane-lede">
      Create and track repositories on your BorgBase account. Enter an API token with
      <strong>Full Access</strong> or <strong>Create Only</strong> permission; it is checked with
      BorgBase, stored encrypted and never shown again. The server adds its own SSH key to the
      account, so a new repository works as soon as it is added as a repository with the BorgBase
      provider.
    </p>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <template v-else>
      <form
        class="form-stack"
        @submit.prevent="saveToken"
      >
        <div class="pane-rows">
          <PaneRow
            title="API token"
            label-for="borgbase-token"
          >
            <input
              id="borgbase-token"
              v-model="tokenInput"
              class="input mono"
              type="password"
              autocomplete="off"
              spellcheck="false"
              :placeholder="tokenSet ? 'New token' : 'Token'"
            />
          </PaneRow>
        </div>
        <div
          v-if="tokenError"
          class="form-error"
        >
          {{ tokenError }}
        </div>
        <div class="info-actions">
          <button
            v-if="tokenSet"
            class="btn btn-ghost btn-danger-text"
            type="button"
            @click="showRemove = true"
          >
            Remove token
          </button>
          <button
            class="btn btn-primary"
            type="submit"
            :disabled="savingToken || tokenInput.trim() === ''"
          >
            {{ savingToken ? 'Checking...' : 'Save token' }}
          </button>
        </div>
      </form>

      <template v-if="tokenSet">
        <form
          class="form-stack"
          @submit.prevent="createRepo"
        >
          <p class="group-label">New repository</p>
          <div class="pane-rows">
            <PaneRow
              title="Name"
              label-for="borgbase-repo-name"
            >
              <input
                id="borgbase-repo-name"
                v-model="form.name"
                class="input"
                placeholder="offsite-laptop"
              />
            </PaneRow>
            <PaneRow
              title="Region"
              label-for="borgbase-repo-region"
            >
              <select
                id="borgbase-repo-region"
                v-model="form.region"
                class="input select-input"
              >
                <option value="eu">Europe</option>
                <option value="us">United States</option>
              </select>
            </PaneRow>
            <PaneRow
              title="Server key access"
              label-for="borgbase-repo-access"
              hint="Append-only keeps data safe from a compromised server."
            >
              <select
                id="borgbase-repo-access"
                v-model="form.access"
                class="input select-input"
              >
                <option value="append_only">Append only</option>
                <option value="full">Full access</option>
              </select>
            </PaneRow>
          </div>
          <div
            v-if="createError"
            class="form-error"
          >
            {{ createError }}
          </div>
          <div
            v-if="created"
            class="form-success"
            data-testid="borgbase-created"
          >
            Created {{ created.name }}. Add it as a repository with the BorgBase provider: host
            <span class="mono">{{ created.ssh_host }}</span>, user
            <span class="mono">{{ created.ssh_user }}</span>, path
            <span class="mono">{{ created.repo_path }}</span>.
          </div>
          <div class="info-actions">
            <button
              class="btn btn-primary"
              type="submit"
              :disabled="!canCreate"
            >
              {{ creating ? 'Creating...' : 'Create repository' }}
            </button>
          </div>
        </form>

        <p class="group-label">Repositories on the account</p>
        <div
          v-if="reposError"
          class="error-banner"
        >
          {{ reposError }}
        </div>
        <EmptyState
          v-else-if="repos.length === 0"
          title="No repositories on this account"
        />
        <div
          v-else
          class="table-wrap"
        >
          <table class="data-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Host</th>
                <th>Usage</th>
                <th>Quota</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="repo in repos"
                :key="repo.id"
                data-testid="borgbase-repo"
              >
                <td>{{ repo.name }}</td>
                <td class="cell-mono">{{ repo.ssh_host }}</td>
                <td>{{ formatFigure(repo.usage) }}</td>
                <td>{{ repo.quota_enabled ? formatFigure(repo.quota) : 'Not enforced' }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <span class="field-hint"
          >Usage and quota are shown in the unit BorgBase reports them in.</span
        >
      </template>
    </template>

    <ConfirmDeleteDialog
      :show="showRemove"
      title="Remove the BorgBase API token?"
      confirm-label="Remove token"
      submitting-label="Removing..."
      :submitting="removing"
      :error="removeError"
      @cancel="showRemove = false"
      @confirm="removeToken"
    />
  </div>
</template>

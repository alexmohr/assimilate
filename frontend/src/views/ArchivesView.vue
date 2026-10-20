<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, useTemplateRef, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useRoute, useRouter } from 'vue-router'
import { RefreshCw } from '@lucide/vue'
import { listRepoArchives } from '../api/archives'
import { getRepoPassphrase, listRepos } from '../api/repos'
import { useAuthStore } from '../stores/auth'
import { useEscapeKey } from '../composables/useEscapeKey'
import { useClipboard } from '../composables/useClipboard'
import { useArchiveDeletionEvents } from '../composables/useArchiveDeletionEvents'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import ArchiveExplorer from '../components/ArchiveExplorer.vue'
import ArchiveDiff from '../components/ArchiveDiff.vue'
import FileSearch from '../components/FileSearch.vue'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import { inRepo, type RepoArchive } from '../utils/repoArchive'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'

/** The route query parameter the selected repository is kept under. */
const REPO_QUERY_KEY = 'repo'

const route = useRoute()
const router = useRouter()

const repos = ref<Repo[]>([])
const reposLoading = ref(false)
const reposError = ref<string | null>(null)
const selectedRepoId = ref<number | null>(null)

const archives = ref<ArchiveEntry[]>([])
const archivesLoading = ref(false)
const archivesError = ref<string | null>(null)
const selectedArchive = ref<RepoArchive | null>(null)

const showPassphraseDialog = ref(false)

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

const sortedArchives = computed(() =>
  [...archives.value].sort((a, b) => b.start.localeCompare(a.start)),
)

const { isAdmin } = storeToRefs(useAuthStore())

const selectedRepoName = computed(
  () => repos.value.find((r) => r.id === selectedRepoId.value)?.name ?? '',
)

const repoArchives = computed<RepoArchive[]>(() =>
  selectedRepoId.value === null
    ? []
    : inRepo(sortedArchives.value, { id: selectedRepoId.value, name: selectedRepoName.value }),
)

async function loadRepos(): Promise<void> {
  reposLoading.value = true
  reposError.value = null
  try {
    repos.value = await listRepos()
  } catch (e: unknown) {
    reposError.value = extractError(e)
  } finally {
    reposLoading.value = false
  }
}

/**
 * The repository the route names, once it is one of the loaded repositories.
 * Back, Forward and links carry it, so an archive the route names is always
 * looked up in the repository it was opened in.
 */
const routeRepoId = computed<number | null>(() => {
  const raw = route.query[REPO_QUERY_KEY]
  if (typeof raw !== 'string' || !/^[0-9]+$/.test(raw)) return null
  const id = Number(raw)
  return repos.value.some((r) => r.id === id) ? id : null
})

/**
 * Picking a repository is a history entry of its own, so Back returns to the
 * repository and archive open before it. The page follows once the route
 * lands, never before: switching first would let the explorer it unmounts
 * rewrite the entry being left.
 */
function pickRepo(event: Event): void {
  const raw = (event.target as HTMLSelectElement).value
  // Picking the repository already open starts it over, as any pick did before
  // the route carried the repository: its archive closes without a new entry.
  if (Number(raw) === selectedRepoId.value) {
    void onRepoChange()
    return
  }
  router.push({ query: { [REPO_QUERY_KEY]: raw } }).catch(logger.error)
}

watch(routeRepoId, (id) => {
  if (id === selectedRepoId.value) return
  selectedRepoId.value = id
  void onRepoChange()
})

async function onRepoChange(): Promise<void> {
  archives.value = []
  // Clearing the selection resets the browser: it watches `archive` and tears
  // down its own polling and path state.
  selectedArchive.value = null
  archivesError.value = null
  if (selectedRepoId.value === null) return
  await loadArchives()
}

// `silent` keeps the list on screen while a background refetch is in flight -
// the explorer asks for one after a delete, and blanking the panel would hide
// the row state it is refreshing.
async function loadArchives(silent = false): Promise<void> {
  const repoId = selectedRepoId.value
  if (repoId === null) return
  if (!silent) archivesLoading.value = true
  archivesError.value = null
  try {
    const list = await listRepoArchives(repoId)
    // Back and Forward can switch the repository again before this lands.
    if (repoId === selectedRepoId.value) archives.value = list
  } catch (e: unknown) {
    if (repoId === selectedRepoId.value) archivesError.value = extractError(e)
  } finally {
    if (repoId === selectedRepoId.value) archivesLoading.value = false
  }
}

const showArchiveDiff = ref(false)

const passphrase = ref<string | null>(null)
const passphraseLoading = ref(false)
const passphraseError = ref<string | null>(null)
const { copied: passphraseCopied, copy: copyToClipboard } = useClipboard()

useEscapeKey(showPassphraseDialog, () => {
  showPassphraseDialog.value = false
})

async function revealPassphrase(): Promise<void> {
  if (selectedRepoId.value === null) return
  passphraseLoading.value = true
  passphraseError.value = null
  passphrase.value = null
  passphraseCopied.value = false
  try {
    const res = await getRepoPassphrase(selectedRepoId.value)
    passphrase.value = res.passphrase
    showPassphraseDialog.value = true
  } catch (e: unknown) {
    passphraseError.value = extractError(e)
    showPassphraseDialog.value = true
  } finally {
    passphraseLoading.value = false
  }
}

// Deleting is available here too, so this page needs the same three events
// that release a row's "deleting..." marker - including the repo-idle one,
// which is the only one that fires when borg's delete failed and left the
// archive in place.
useArchiveDeletionEvents({
  target: () => explorer.value,
  repoIds: () => (selectedRepoId.value === null ? [] : [selectedRepoId.value]),
  reload: () => loadArchives(true),
})

onMounted(loadRepos)
</script>

<template>
  <div class="archives-view">
    <div class="page-header">
      <h1 class="page-title">Archives</h1>
    </div>

    <div
      v-if="reposLoading"
      class="state-msg state-msg--inline"
    >
      Loading repositories...
    </div>
    <div
      v-else-if="reposError"
      class="error-banner"
    >
      {{ reposError }}
    </div>
    <div
      v-else
      class="page-sections"
    >
      <div class="repo-selector">
        <label class="selector-label">Repository</label>
        <select
          :value="selectedRepoId"
          class="input select-input select-input--lg"
          @change="pickRepo"
        >
          <option
            :value="null"
            disabled
          >
            — select a repository —
          </option>
          <option
            v-for="repo in repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.name }}
          </option>
        </select>
        <span
          v-if="repos.length === 0"
          class="muted-hint"
          >No repositories configured yet.</span
        >
        <button
          v-if="selectedRepoId !== null"
          class="btn btn-sm btn-ghost passphrase-btn"
          :disabled="passphraseLoading"
          @click="revealPassphrase"
        >
          {{ passphraseLoading ? 'Loading...' : 'Show Passphrase' }}
        </button>
      </div>

      <!-- One explorer per repository, each with its own route keys: an archive
           name left in the URL by one repository can never open another's
           archive of the same name. -->
      <ArchiveExplorer
        v-if="selectedRepoId !== null"
        :key="selectedRepoId"
        ref="explorer"
        v-model:selected="selectedArchive"
        :archives="repoArchives"
        :loading="archivesLoading"
        :error="archivesError"
        :is-admin="isAdmin"
        :reload="loadArchives"
        :archive-query-key="`browse-${selectedRepoId}`"
        :path-query-key="`path-${selectedRepoId}`"
        empty-description="This repository has no archives yet. They appear here once a backup has run."
      >
        <template #actions>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="archives.length < 2"
            @click="showArchiveDiff = true"
          >
            Diff
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="archivesLoading"
            aria-label="Refresh archives"
            @click="loadArchives()"
          >
            <RefreshCw
              :size="14"
              :class="{ spinning: archivesLoading }"
            />
          </button>
        </template>
      </ArchiveExplorer>

      <FileSearch
        :repo-id="selectedRepoId"
        :archives="archives.map((a) => ({ name: a.name }))"
      />
    </div>

    <!-- Passphrase Dialog -->
    <BaseModal
      :open="showPassphraseDialog"
      :title="passphrase ? 'Repository Passphrase' : 'Error'"
      @close="showPassphraseDialog = false"
    >
      <template v-if="passphrase">
        <p class="passphrase-warning">Keep this passphrase secure. Do not share it.</p>
        <div class="passphrase-box">
          <code class="passphrase-text">{{ passphrase }}</code>
          <button
            class="btn btn-sm btn-ghost"
            @click="passphrase && copyToClipboard(passphrase)"
          >
            {{ passphraseCopied ? 'Copied!' : 'Copy' }}
          </button>
        </div>
      </template>
      <div
        v-else-if="passphraseError"
        class="form-error"
      >
        {{ passphraseError }}
      </div>

      <template #footer>
        <button
          class="btn btn-primary"
          @click="showPassphraseDialog = false"
        >
          Done
        </button>
      </template>
    </BaseModal>
    <!-- Archive Diff -->
    <ArchiveDiff
      :open="showArchiveDiff"
      :repo-id="selectedRepoId"
      :archives="archives"
      @close="showArchiveDiff = false"
    />
  </div>
</template>

<style scoped>
.archives-view {
  max-width: 1300px;
  color: var(--text-primary);
}

.repo-selector {
  display: flex;
  align-items: center;
  gap: var(--space-6);
}

.selector-label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  white-space: nowrap;
}

.muted-hint {
  font-size: var(--fs-sm);
  color: var(--text-muted);
}

/* Buttons */

.passphrase-btn {
  margin-left: auto;
}
</style>

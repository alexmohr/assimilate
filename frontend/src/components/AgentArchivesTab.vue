<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, useTemplateRef, watch } from 'vue'
import ArchiveExplorer from './ArchiveExplorer.vue'
import EmptyState from './EmptyState.vue'
import { listRepoArchives } from '../api/archives'
import { resolveArchiveDomain, resolveArchiveHost } from '../utils/archiveHost'
import { extractError } from '../utils/error'
import { inRepo, type RepoArchive } from '../utils/repoArchive'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import type { Repo } from '../types/repo'

/**
 * An agent's Backups tab: every archive this agent has in every repository
 * it backs up to, in one `ArchiveExplorer` - the same one the repository and
 * schedule screens render - with each row tagged with its repository.
 *
 * One list rather than one explorer per repository: an agent writing into two
 * repositories used to get two full lists and two empty file browsers stacked
 * on top of each other, which on a phone put the second repository's archives
 * screens below the first's.
 *
 * Reads straight from each repository's real archive list (`listRepoArchives`,
 * which borg itself is the source of truth for and which is never capped)
 * rather than deriving archives from the agent's paginated report history -
 * this tab should show every archive regardless of how far the Logs tab's
 * "Load more" has been clicked.
 */
const props = defineProps<{
  hostname: string
  /**
   * Disambiguates `hostname` when it is shared by more than one agent (see
   * `resolveArchiveDomain`). Two such agents can back up to the same
   * repository, so filtering archives by hostname alone would mix one
   * agent's archives into the other's tab.
   */
  domain?: string | null
  repos: Repo[]
  isAdmin: boolean
}>()

/** Each repository's archives as last fetched, kept apart so one can be refetched alone. */
const byRepo = ref<Map<number, RepoArchive[]>>(new Map())
const loading = ref(false)
/** Every repository that failed to load, named, so one bad repository does not hide the rest. */
const errors = ref<Map<number, string>>(new Map())
const selected = ref<RepoArchive | null>(null)

const explorer = useTemplateRef<InstanceType<typeof ArchiveExplorer>>('explorer')

const archives = computed<RepoArchive[]>(() =>
  props.repos.flatMap((repo) => byRepo.value.get(repo.id) ?? []),
)

const error = computed<string | null>(() => {
  if (errors.value.size === 0) return null
  return props.repos
    .filter((repo) => errors.value.has(repo.id))
    .map((repo) => `${repo.name}: ${errors.value.get(repo.id)}`)
    .join(' - ')
})

async function loadRepo(repo: Repo): Promise<void> {
  try {
    const all = await listRepoArchives(repo.id)
    const own = all.filter(
      (a) =>
        resolveArchiveHost(a) === props.hostname &&
        resolveArchiveDomain(a) === (props.domain ?? null),
    )
    byRepo.value = new Map(byRepo.value).set(repo.id, inRepo(own, repo))
    if (errors.value.has(repo.id)) {
      const next = new Map(errors.value)
      next.delete(repo.id)
      errors.value = next
    }
  } catch (e: unknown) {
    errors.value = new Map(errors.value).set(repo.id, extractError(e))
  }
}

/** Silent: keeps the list and selection while it refetches, as a background refresh must. */
function loadAll(silent: boolean): Promise<void> {
  if (!silent) loading.value = true
  return Promise.all(props.repos.map(loadRepo))
    .then(() => undefined)
    .finally(() => {
      loading.value = false
    })
}

// Keyed by id, not the `repos` array itself: `AgentDetailView`'s background
// refresh (WS DataChanged, reconnect, AgentDisconnected) reassigns `repos` to
// a fresh array on every run even when this agent's actual repositories
// haven't changed, and watching the array by reference would reload from
// scratch - resetting the loading state and clearing whatever archive is
// selected - on every one of those refreshes. The `DataChanged` handler below
// already does the silent, selection-preserving refresh for "the same repos,
// maybe new archives"; this watcher is only for "the set of repos actually
// changed".
const repoIdsKey = (): string => props.repos.map((r) => r.id).join(',')
watch(
  repoIdsKey,
  () => {
    byRepo.value = new Map()
    errors.value = new Map()
    selected.value = null
    void loadAll(false)
  },
  { immediate: true },
)

// Every screen that browses and deletes archives needs these three events -
// see useArchiveDeletionEvents. Matched against `repos` at event time, since
// which repositories this agent has isn't known yet when listeners are
// registered here at setup.
const { onMessage } = useWebSocket()

function isOurs(repoId: number): boolean {
  return props.repos.some((repo) => repo.id === repoId)
}

onMessage('ArchiveDeleted', (payload) => {
  if (isOurs(payload.repo_id)) {
    explorer.value?.onArchiveDeleted(payload.archive_name, payload.repo_id)
  }
})

onMessage('DataChanged', () => {
  loadAll(true)
    .then(() => explorer.value?.onDataChanged())
    .catch(logger.error)
})

onMessage('RepoOpChanged', (payload) => {
  if (payload.op?.kind === 'delete_archive' || payload.op?.kind === 'compact_repo') return
  if (isOurs(payload.repo_id)) explorer.value?.onRepoIdle(payload.repo_id)
})
</script>

<template>
  <EmptyState
    v-if="repos.length === 0"
    title="No repositories yet"
    description="Archives appear here once this agent has backed up to a repository."
  />
  <ArchiveExplorer
    v-else
    ref="explorer"
    v-model:selected="selected"
    :archives="archives"
    :loading="loading"
    :error="error"
    :is-admin="isAdmin"
    :reload="loadAll"
    empty-title="No archives"
    empty-description="No archives from this host in its repositories yet."
  >
    <!--
      One repository is named here, once: rows only carry a repository tag
      when the list spans several, and on an agent's page "which repository
      are these in" is not otherwise answered anywhere.
    -->
    <template
      v-if="repos.length === 1"
      #actions
    >
      <span class="muted">in {{ repos[0]?.name }}</span>
    </template>
  </ArchiveExplorer>
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch } from 'vue'
import ArchiveExplorer from './ArchiveExplorer.vue'
import EmptyState from './EmptyState.vue'
import { listRepoArchives } from '../api/archives'
import { resolveArchiveHost } from '../utils/archiveHost'
import { extractError } from '../utils/error'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { Repo } from '../types/repo'

/**
 * An agent's Backups tab: one archive browser per repository it backs up to,
 * each the same `ArchiveExplorer` the repository and schedule screens render,
 * filtered to this agent's own archives.
 *
 * Reads straight from each repository's real archive list (`listRepoArchives`,
 * which borg itself is the source of truth for and which is never capped)
 * rather than deriving archives from the agent's paginated report history -
 * this tab should show every archive regardless of how far the Logs tab's
 * "Load more" has been clicked.
 */
const props = defineProps<{
  hostname: string
  repos: Repo[]
  isAdmin: boolean
}>()

interface RepoSection {
  repo: Repo
  archives: ArchiveEntry[]
  loading: boolean
  error: string | null
  selected: ArchiveEntry | null
}

const sections = ref<RepoSection[]>([])
const explorerRefs = new Map<number, InstanceType<typeof ArchiveExplorer>>()

function setExplorerRef(repoId: number, el: unknown): void {
  if (el) {
    explorerRefs.set(repoId, el as InstanceType<typeof ArchiveExplorer>)
  } else {
    explorerRefs.delete(repoId)
  }
}

async function loadSection(section: RepoSection, silent = false): Promise<void> {
  if (!silent) section.loading = true
  section.error = null
  try {
    const all = await listRepoArchives(section.repo.id)
    section.archives = all.filter((a) => resolveArchiveHost(a) === props.hostname)
  } catch (e: unknown) {
    section.error = extractError(e)
  } finally {
    section.loading = false
  }
}

async function loadAll(): Promise<void> {
  sections.value = props.repos.map((repo) => ({
    repo,
    archives: [],
    loading: true,
    error: null,
    selected: null,
  }))
  await Promise.all(sections.value.map((section) => loadSection(section)))
}

watch(() => props.repos, loadAll, { immediate: true })

// Every screen that browses and deletes archives needs these three events -
// see useArchiveDeletionEvents, whose single-repo contract doesn't fit a tab
// that renders one explorer per repository. Looked up from `sections.value`
// at event time instead, since which repos this agent has isn't known yet
// when listeners are registered here at setup.
const { onMessage } = useWebSocket()

onMessage('ArchiveDeleted', (payload) => {
  explorerRefs.get(payload.repo_id)?.onArchiveDeleted(payload.archive_name)
})

onMessage('DataChanged', () => {
  Promise.all(sections.value.map((section) => loadSection(section, true)))
    .then(() =>
      sections.value.forEach((section) => explorerRefs.get(section.repo.id)?.onDataChanged()),
    )
    .catch(logger.error)
})

onMessage('RepoOpChanged', (payload) => {
  if (payload.op?.kind === 'delete_archive' || payload.op?.kind === 'compact_repo') return
  explorerRefs.get(payload.repo_id)?.onRepoIdle()
})
</script>

<template>
  <div class="agent-archives-tab">
    <EmptyState
      v-if="repos.length === 0"
      title="No repositories yet"
      description="Archives appear here once this agent has backed up to a repository."
    />
    <section
      v-for="section in sections"
      :key="section.repo.id"
      class="archive-section"
    >
      <h3 class="archive-section-title">{{ section.repo.name }}</h3>
      <ArchiveExplorer
        :ref="(el) => setExplorerRef(section.repo.id, el)"
        v-model:selected="section.selected"
        :repo-id="section.repo.id"
        :repo-name="section.repo.name"
        :archives="section.archives"
        :loading="section.loading"
        :error="section.error"
        :is-admin="isAdmin"
        :reload="(silent: boolean) => loadSection(section, silent)"
        empty-title="No archives"
        empty-description="No archives from this host in this repository yet."
      />
    </section>
  </div>
</template>

<style scoped>
.agent-archives-tab {
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
}

.archive-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.archive-section-title {
  font-size: var(--fs-md);
  font-weight: 600;
  margin: 0;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import QuotaPanel from './QuotaPanel.vue'
import EntityTags from './EntityTags.vue'
import RepoBorgConsole from './RepoBorgConsole.vue'
import RepoDangerZone from './RepoDangerZone.vue'
import RepoOverviewCard from './RepoOverviewCard.vue'
import RepoPowerCard from './RepoPowerCard.vue'
import SettingsRail, { type SettingsSections } from './SettingsRail.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'
import type { RepoSettingsSection } from '../utils/repoSettings'

/**
 * Everything that configures a repository, behind one tab with a sub-nav -
 * the shape agents and schedules already use.
 *
 * These were five cards stacked below the tab strip on Overview, each with the
 * same border and its own bottom-right actions, so the landing tab answered no
 * operational question at all: it opened on a form.
 */
const props = defineProps<{
  repo: RepoWithStats
  section: RepoSettingsSection
  isAdmin: boolean
  currentOp: ActiveRepoOp | null
}>()

const emit = defineEmits<{
  'update:section': [value: RepoSettingsSection]
  changed: []
  error: [value: string]
}>()

/** Everything but the repository's own fields is admin-only, so it is absent
    rather than disabled for everyone else. */
const sections = computed<SettingsSections<RepoSettingsSection>>(() => [
  { id: 'repository', label: 'Repository' },
  ...(props.isAdmin
    ? [
        { id: 'power', label: 'Power' } as const,
        { id: 'quota', label: 'Storage quota' } as const,
        { id: 'tags', label: 'Tags' } as const,
        { id: 'console', label: 'Borg console' } as const,
        { id: 'danger', label: 'Danger zone', danger: true } as const,
      ]
    : []),
])
</script>

<template>
  <SettingsRail
    v-slot="{ section: currentSection }"
    :sections="sections"
    :section="section"
    label="Repository settings sections"
    @update:section="emit('update:section', $event)"
  >
    <RepoOverviewCard
      v-if="currentSection === 'repository'"
      :repo="repo"
      :is-admin="isAdmin"
      :current-op="currentOp"
      @saved="emit('changed')"
    />

    <RepoPowerCard
      v-else-if="currentSection === 'power'"
      :repo="repo"
      :is-admin="isAdmin"
      @saved="emit('changed')"
    />

    <QuotaPanel
      v-else-if="currentSection === 'quota'"
      :repo-id="repo.id"
      :is-admin="isAdmin"
      :current-usage-bytes="repo.total_deduplicated_size"
    />

    <EntityTags
      v-else-if="currentSection === 'tags'"
      scope="repo"
      :entity-path="`/repos/${repo.id}`"
    />

    <RepoBorgConsole
      v-else-if="currentSection === 'console'"
      :repo-id="repo.id"
    />

    <RepoDangerZone
      v-else-if="currentSection === 'danger'"
      :repo="repo"
      :current-op="currentOp"
      @error="emit('error', $event)"
      @changed="emit('changed')"
    />
  </SettingsRail>
</template>

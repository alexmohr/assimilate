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

interface SectionOption {
  id: RepoSettingsSection
  label: string
  danger?: boolean
}

/** Everything but the repository's own fields is admin-only, so it is absent
    rather than disabled for everyone else. */
const sections = computed<SectionOption[]>(() => {
  const list: SectionOption[] = [{ id: 'repository', label: 'Repository' }]
  if (props.isAdmin) {
    list.push({ id: 'quota', label: 'Storage quota' })
    list.push({ id: 'tags', label: 'Tags' })
    list.push({ id: 'console', label: 'Borg console' })
    list.push({ id: 'danger', label: 'Danger zone', danger: true })
  }
  return list
})

/** Same fallback as the agent and schedule settings tabs: a section this
    viewer has no button for renders as the first one they do. */
const currentSection = computed<RepoSettingsSection>(() =>
  sections.value.some((s) => s.id === props.section) ? props.section : 'repository',
)
</script>

<template>
  <div class="settings-tab">
    <nav
      class="settings-nav"
      aria-label="Repository settings sections"
    >
      <button
        v-for="s in sections"
        :key="s.id"
        type="button"
        class="settings-nav-item"
        :class="{ 'settings-nav-item--danger': s.danger }"
        :aria-current="s.id === currentSection"
        @click="emit('update:section', s.id)"
      >
        {{ s.label }}
      </button>
    </nav>

    <div class="settings-pane">
      <RepoOverviewCard
        v-if="currentSection === 'repository'"
        :repo="repo"
        :is-admin="isAdmin"
        :current-op="currentOp"
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
    </div>
  </div>
</template>

<style scoped>
/* Base shapes live in style.css, shared with the agent and schedule settings
   tabs. Only this page's mobile collapse is its own. */
@media (max-width: 768px) {
  .settings-tab {
    flex-direction: column;
  }

  .settings-nav {
    width: 100%;
    flex-direction: row;
    flex-wrap: wrap;
    border-bottom: 1px solid var(--border);
  }

  .settings-nav-item {
    border-left: none;
    border-bottom: 2px solid transparent;
  }

  .settings-nav-item[aria-current='true'] {
    border-left-color: transparent;
    border-bottom-color: var(--accent);
  }

  .settings-nav-item--danger[aria-current='true'] {
    border-bottom-color: var(--danger);
  }
}
</style>

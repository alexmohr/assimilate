<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatDate } from '../utils/format'
import EntityTags from './EntityTags.vue'
import AgentDefaultsCard from './AgentDefaultsCard.vue'
import AgentHostnameAliases from './AgentHostnameAliases.vue'
import AgentDangerZone from './AgentDangerZone.vue'
import type { AgentRow } from '../types/agent'
import type { SettingsSection } from '../utils/agentSettings'

/**
 * Everything that configures an agent, behind one tab with a sub-nav.
 *
 * These were nine cards stacked below the agent's status on the Overview tab,
 * each with the same border and the same bottom-right Edit button, so nothing
 * on the landing tab was more prominent than the build timestamp. Grouping
 * them here empties Overview for the operational answer.
 */
const props = defineProps<{
  agent: AgentRow
  section: SettingsSection
  isAdmin: boolean
  regenLoading: boolean
}>()

const emit = defineEmits<{
  'update:section': [value: SettingsSection]
  editIdentity: []
  regenerateToken: []
  saved: [agent: AgentRow]
}>()

const aliases = ref<InstanceType<typeof AgentHostnameAliases> | null>(null)

/**
 * The view renames the agent, then offers to keep the old hostname as a
 * matching pattern; accepting that has to refresh the list in here.
 */
async function reloadAliases(hostname: string): Promise<void> {
  await aliases.value?.reload(hostname)
}

defineExpose({ reloadAliases })

const isImported = computed(() => props.agent.is_imported)

interface SectionOption {
  id: SettingsSection
  label: string
  danger?: boolean
}

/** The danger zone is admin-only, so it is absent rather than disabled. */
const sections = computed<SectionOption[]>(() => {
  const list: SectionOption[] = [
    { id: 'identity', label: 'Identity' },
    { id: 'defaults', label: 'Backup defaults' },
    { id: 'aliases', label: 'Hostname aliases' },
  ]
  if (props.isAdmin) {
    list.push({ id: 'tags', label: 'Tags' })
    list.push({ id: 'danger', label: 'Danger zone', danger: true })
  }
  return list
})
</script>

<template>
  <div class="settings-tab">
    <nav
      class="settings-nav"
      aria-label="Agent settings sections"
    >
      <button
        v-for="s in sections"
        :key="s.id"
        type="button"
        class="settings-nav-item"
        :class="{ 'settings-nav-item--danger': s.danger }"
        :aria-current="s.id === section"
        @click="emit('update:section', s.id)"
      >
        {{ s.label }}
      </button>
    </nav>

    <div class="settings-pane">
      <template v-if="section === 'identity'">
        <div class="info-card">
          <div class="info-title-row">
            <h3 class="info-title">Identity</h3>
            <button
              v-if="!isImported"
              class="btn btn-sm"
              type="button"
              @click="emit('editIdentity')"
            >
              Edit
            </button>
          </div>
          <dl class="info-grid">
            <dt>Hostname</dt>
            <dd class="mono">{{ agent.hostname }}</dd>
            <dt>Display name</dt>
            <dd>{{ agent.display_name ?? 'Not set' }}</dd>
            <template v-if="!isImported">
              <dt>Agent version</dt>
              <dd class="mono">{{ agent.agent_version ?? 'Unknown' }}</dd>
              <dt>Revision</dt>
              <dd class="mono">{{ agent.agent_git_sha ?? 'Unknown' }}</dd>
              <dt>Built</dt>
              <dd class="mono">{{ agent.agent_build_time ?? 'Unknown' }}</dd>
            </template>
            <dt>Registered</dt>
            <dd>{{ formatDate(agent.created_at ?? null, 'Never') }}</dd>
            <dt>Last seen</dt>
            <dd>{{ formatDate(agent.last_seen_at ?? null, 'Never') }}</dd>
          </dl>
        </div>

        <div
          v-if="!isImported"
          class="info-card"
        >
          <div class="info-title-row">
            <h3 class="info-title">Connection</h3>
            <button
              class="btn btn-sm"
              type="button"
              :disabled="regenLoading"
              @click="emit('regenerateToken')"
            >
              {{ regenLoading ? 'Regenerating...' : 'Regenerate token' }}
            </button>
          </div>
          <p class="field-hint">
            Regenerating invalidates the current token immediately. The agent stays disconnected
            until it is restarted with the new one.
          </p>
        </div>
      </template>

      <AgentDefaultsCard
        v-else-if="section === 'defaults'"
        :agent="agent"
        :can-edit="!isImported"
        @saved="emit('saved', $event)"
      />

      <AgentHostnameAliases
        v-else-if="section === 'aliases'"
        ref="aliases"
        :hostname="agent.hostname"
        :can-edit="!isImported"
      />

      <EntityTags
        v-else-if="section === 'tags'"
        scope="host"
        :entity-path="`/agents/${agent.hostname}`"
      />

      <AgentDangerZone
        v-else-if="section === 'danger'"
        :agent="agent"
      />
    </div>
  </div>
</template>

<style scoped>
.settings-tab {
  display: flex;
  gap: 1.25rem;
  align-items: flex-start;
}

.settings-nav {
  width: 170px;
  flex: none;
  display: flex;
  flex-direction: column;
}

.settings-nav-item {
  font: inherit;
  text-align: left;
  font-size: var(--fs-sm);
  padding: 0.35rem 0.6rem;
  border: none;
  border-left: 2px solid transparent;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.settings-nav-item:hover {
  color: var(--text-primary);
}

.settings-nav-item[aria-current='true'] {
  color: var(--accent);
  font-weight: 600;
  border-left-color: var(--accent);
  background: var(--accent-subtle);
}

.settings-nav-item--danger[aria-current='true'] {
  color: var(--danger);
  border-left-color: var(--danger);
  background: var(--danger-subtle);
}

.settings-pane {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

@media (max-width: 720px) {
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

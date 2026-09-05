<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatDate } from '../utils/format'
import { domainParams } from '../utils/agent'
import EntityTags from './EntityTags.vue'
import SettingsRail, { type SettingsSections } from './SettingsRail.vue'
import AgentDefaultsCard from './AgentDefaultsCard.vue'
import AgentHostnameAliases from './AgentHostnameAliases.vue'
import AgentPowerCard from './AgentPowerCard.vue'
import AgentVmsCard from './AgentVmsCard.vue'
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

/** The danger zone is admin-only, so it is absent rather than disabled. */
const sections = computed<SettingsSections<SettingsSection>>(() => [
  { id: 'identity', label: 'Identity' },
  { id: 'defaults', label: 'Backup defaults' },
  { id: 'aliases', label: 'Hostname aliases' },
  ...(props.isAdmin
    ? [
        { id: 'power', label: 'Power' } as const,
        { id: 'vms', label: 'Virtual machines' } as const,
        { id: 'tags', label: 'Tags' } as const,
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
    label="Agent settings sections"
    @update:section="emit('update:section', $event)"
  >
    <template v-if="currentSection === 'identity'">
      <div class="pane-head">
        <p class="pane-lede">How this host names itself, and what the server knows about it.</p>
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
        <dt>Domain</dt>
        <dd class="mono">{{ agent.domain ?? 'Not set' }}</dd>
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

      <section
        v-if="!isImported"
        class="pane-section"
      >
        <div class="pane-section-head">
          <span class="group-label">Connection</span>
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
          Regenerating invalidates the current token immediately. The agent stays disconnected until
          it is restarted with the new one.
        </p>
      </section>
    </template>

    <AgentDefaultsCard
      v-else-if="currentSection === 'defaults'"
      :agent="agent"
      :can-edit="!isImported"
      @saved="emit('saved', $event)"
    />

    <AgentHostnameAliases
      v-else-if="currentSection === 'aliases'"
      ref="aliases"
      :hostname="agent.hostname"
      :domain="agent.domain"
      :can-edit="!isImported"
    />

    <AgentPowerCard
      v-else-if="currentSection === 'power'"
      :agent="agent"
      :can-edit="!isImported"
      @saved="emit('saved', $event)"
    />

    <AgentVmsCard
      v-else-if="currentSection === 'vms'"
      :agent="agent"
      :can-edit="!isImported"
    />

    <EntityTags
      v-else-if="currentSection === 'tags'"
      scope="host"
      :entity-path="`/agents/${agent.hostname}`"
      :entity-params="domainParams(agent.domain)"
    />

    <AgentDangerZone
      v-else-if="currentSection === 'danger'"
      :agent="agent"
    />
  </SettingsRail>
</template>

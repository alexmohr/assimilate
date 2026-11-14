<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import SettingsRail, { type SettingsSections } from './SettingsRail.vue'
import AgentDefaultsCard from './AgentDefaultsCard.vue'
import AgentSourcesUsedBy from './AgentSourcesUsedBy.vue'
import AgentVmsCard from './AgentVmsCard.vue'
import AgentDatabasesCard from './AgentDatabasesCard.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow } from '../types/schedule'
import type { SourcesSection } from '../utils/agentSources'

/**
 * Everything on this host worth backing up: its files (paths, excludes, file
 * change patterns and the commands that run around them), its virtual
 * machines and its databases. A schedule takes it as it is, or sets its own
 * paths under its Settings › Sources; Used by shows which does what.
 *
 * Files used to be "Backup defaults" under Settings; they answer the same
 * question as VMs and databases, so they sit beside them.
 */
const props = defineProps<{
  agent: AgentRow
  section: SourcesSection
  isAdmin: boolean
  /** The schedules that target this host, so each section can name those taking from it. */
  schedules?: readonly ScheduleRow[]
}>()

const emit = defineEmits<{
  'update:section': [value: SourcesSection]
  saved: [agent: AgentRow]
}>()

/** VMs and databases stay admin-only, as they were; files are for everyone. */
const sections = computed<SettingsSections<SourcesSection>>(() => [
  { id: 'files', label: 'Files' },
  ...(props.isAdmin
    ? ([
        { id: 'vms', label: 'Virtual machines' },
        { id: 'databases', label: 'Databases' },
      ] as const)
    : []),
  { id: 'used-by', label: 'Used by' },
])
</script>

<template>
  <SettingsRail
    v-slot="{ section: currentSection }"
    :sections="sections"
    :section="section"
    label="Agent backup sources"
    @update:section="emit('update:section', $event)"
  >
    <AgentDefaultsCard
      v-if="currentSection === 'files'"
      :agent="agent"
      :can-edit="!agent.is_imported"
      @saved="emit('saved', $event)"
    />
    <AgentVmsCard
      v-else-if="currentSection === 'vms'"
      :agent="agent"
      :can-edit="!agent.is_imported"
    />
    <AgentDatabasesCard
      v-else-if="currentSection === 'databases'"
      :agent="agent"
      :can-edit="!agent.is_imported"
      :schedules="schedules ?? []"
    />
    <AgentSourcesUsedBy
      v-else-if="currentSection === 'used-by'"
      :agent="agent"
      :schedules="schedules ?? []"
    />
  </SettingsRail>
</template>

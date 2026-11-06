<script setup lang="ts">
import SettingsAccordion from './SettingsAccordion.vue'
import type { SettingsSections } from './SettingsRail.vue'
import AgentVmsCard from './AgentVmsCard.vue'
import AgentDatabasesCard from './AgentDatabasesCard.vue'
import type { AgentRow } from '../types/agent'

/**
 * What an agent backs up beyond plain files: its virtual machines and its
 * databases. They used to be two Settings sections; they are lists of things
 * on the host as much as configuration, so they get a tab of their own.
 */
type SourcesSection = 'vms' | 'databases'

defineProps<{
  agent: AgentRow
  section: SourcesSection
}>()

const emit = defineEmits<{ 'update:section': [value: SourcesSection] }>()

const sections: SettingsSections<SourcesSection> = [
  { id: 'vms', label: 'Virtual machines' },
  { id: 'databases', label: 'Databases' },
]
</script>

<template>
  <SettingsAccordion
    v-slot="{ section: currentSection }"
    :sections="sections"
    :section="section"
    label="Agent backup sources"
    @update:section="emit('update:section', $event)"
  >
    <AgentVmsCard
      v-if="currentSection === 'vms'"
      :agent="agent"
      :can-edit="!agent.is_imported"
    />
    <AgentDatabasesCard
      v-else-if="currentSection === 'databases'"
      :agent="agent"
      :can-edit="!agent.is_imported"
    />
  </SettingsAccordion>
</template>

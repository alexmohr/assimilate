<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { updateAgent } from '../api/agents'
import { extractError } from '../utils/error'
import { dropBlankCommands, parseLines } from '../utils/validation'
import FileSourcesSection from './FileSourcesSection.vue'
import type { AgentRow } from '../types/agent'
import type { FileSet } from '../types/fileSet'

/**
 * A host's Sources › Files: the shared Files section, saved to the agent.
 *
 * One form and one request for all of it: the agent PUT is a whole-object
 * replace, so separate cards each had to send the others' values back
 * untouched. See docs/contributing/ui-design-audit.md (F-24). A schedule's own
 * files render the same section, so the two look and behave the same.
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to push settings to. */
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [agent: AgentRow] }>()

const section = ref<InstanceType<typeof FileSourcesSection> | null>(null)
const saving = ref(false)
const error = ref<string | null>(null)

const value = computed<FileSet>(() => ({
  paths: (props.agent.default_backup_paths ?? []).join('\n'),
  excludes: (props.agent.default_exclude_patterns ?? []).join('\n'),
  includes: (props.agent.default_include_patterns ?? []).join('\n'),
  fileChangePatterns: props.agent.default_file_change_patterns_raw ?? '',
  preCommands: props.agent.default_pre_backup_commands ?? [],
  postCommands: props.agent.default_post_backup_commands ?? [],
}))

/** Holds the form open if the request fails, so nothing typed is lost. */
async function save(next: FileSet): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const res = await updateAgent(
      props.agent.hostname,
      {
        display_name: props.agent.display_name,
        domain: props.agent.domain,
        default_backup_paths: parseLines(next.paths),
        default_exclude_patterns: parseLines(next.excludes),
        // Not in the request type yet: see `AgentFileSetExtras`.
        ...{ default_include_patterns: parseLines(next.includes) },
        default_pre_backup_commands: dropBlankCommands(next.preCommands),
        default_post_backup_commands: dropBlankCommands(next.postCommands),
        default_file_change_patterns_raw: next.fileChangePatterns,
      },
      props.agent.domain,
    )
    emit('saved', res)
    section.value?.close()
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <FileSourcesSection
    ref="section"
    :value="value"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    lede="The files this host backs up. Every schedule uses them unless it sets its own files under
      its Settings › Sources."
    @save="save"
  />
</template>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { updateAgent } from '../api/agents'
import { extractError } from '../utils/error'
import { dropBlankCommands, parseLines } from '../utils/validation'
import EditableSection from './EditableSection.vue'
import FileSourcesEditor from './FileSourcesEditor.vue'
import FileSourcesView from './FileSourcesView.vue'
import type { AgentRow } from '../types/agent'
import type { HookCommand } from '../types/generated'

/**
 * An agent's backup defaults: paths, excludes, file change patterns and hook
 * commands, as one card with four sections and one Edit button.
 *
 * These were four separate cards, each with its own edit state and its own
 * save. That was never four requests' worth of independence: the agent PUT is
 * a whole-object replace, so every card already had to send the other three
 * cards' values back untouched alongside its own patch. One form means one
 * request and no way for a stale copy of a sibling field to be written back.
 * See docs/contributing/ui-design-audit.md (F-24).
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to push settings to. */
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [agent: AgentRow] }>()

const editing = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

const pathsText = ref('')
const excludesText = ref('')
const fcpText = ref('')
const preCmds = ref<HookCommand[]>([])
const postCmds = ref<HookCommand[]>([])

function startEdit(): void {
  pathsText.value = (props.agent.default_backup_paths ?? []).join('\n')
  excludesText.value = (props.agent.default_exclude_patterns ?? []).join('\n')
  fcpText.value = props.agent.default_file_change_patterns_raw ?? ''
  preCmds.value = [...(props.agent.default_pre_backup_commands ?? [])]
  postCmds.value = [...(props.agent.default_post_backup_commands ?? [])]
  error.value = null
  editing.value = true
}

/** Holds the form open if the request fails, so nothing typed is lost. */
async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const res = await updateAgent(
      props.agent.hostname,
      {
        display_name: props.agent.display_name,
        domain: props.agent.domain,
        default_backup_paths: parseLines(pathsText.value),
        default_exclude_patterns: parseLines(excludesText.value),
        default_pre_backup_commands: dropBlankCommands(preCmds.value),
        default_post_backup_commands: dropBlankCommands(postCmds.value),
        default_file_change_patterns_raw: fcpText.value,
      },
      props.agent.domain,
    )
    emit('saved', res)
    editing.value = false
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <EditableSection
    lede="The files this host backs up. Every schedule uses them unless it sets its own under its
      Settings › Sources."
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <FileSourcesView
        :paths="agent.default_backup_paths ?? []"
        :excludes="agent.default_exclude_patterns ?? []"
        :file-change-patterns-raw="agent.default_file_change_patterns_raw ?? ''"
        :pre-commands="agent.default_pre_backup_commands ?? []"
        :post-commands="agent.default_post_backup_commands ?? []"
      />
    </template>

    <template #hint>
      A schedule uses all of this as it is, sets its own files instead (with the same fields), or
      leaves files out altogether.
    </template>

    <template #edit>
      <FileSourcesEditor
        v-model:paths="pathsText"
        v-model:excludes="excludesText"
        v-model:file-change-patterns="fcpText"
        v-model:pre-commands="preCmds"
        v-model:post-commands="postCmds"
        id-prefix="host-files"
      />
    </template>
  </EditableSection>
</template>

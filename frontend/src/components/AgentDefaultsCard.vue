<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { parseLines } from '../utils/validation'
import { parseFileChangePatterns } from '../utils/fileChangePatterns'
import EditableSection from './EditableSection.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import type { AgentRow } from '../types/agent'

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
const preCmdsText = ref('')
const postCmdsText = ref('')

function startEdit(): void {
  pathsText.value = (props.agent.default_backup_paths ?? []).join('\n')
  excludesText.value = (props.agent.default_exclude_patterns ?? []).join('\n')
  fcpText.value = props.agent.default_file_change_patterns_raw ?? ''
  preCmdsText.value = (props.agent.default_pre_backup_commands ?? []).join('\n')
  postCmdsText.value = (props.agent.default_post_backup_commands ?? []).join('\n')
  error.value = null
  editing.value = true
}

/** Holds the form open if the request fails, so nothing typed is lost. */
async function save(): Promise<void> {
  saving.value = true
  error.value = null
  try {
    const res = await apiClient.put<AgentRow>(`/agents/${props.agent.hostname}`, {
      display_name: props.agent.display_name,
      default_backup_paths: parseLines(pathsText.value),
      default_exclude_patterns: parseLines(excludesText.value),
      default_pre_backup_commands: parseLines(preCmdsText.value),
      default_post_backup_commands: parseLines(postCmdsText.value),
      default_file_change_patterns_raw: fcpText.value,
    })
    emit('saved', res.data)
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
    lede="What a schedule uses for this host when it does not set its own paths, patterns or
      commands."
    :editing="editing"
    :can-edit="canEdit"
    :saving="saving"
    :error="error"
    @edit="startEdit"
    @cancel="editing = false"
    @save="save"
  >
    <template #view>
      <div class="defaults-groups">
        <div class="defaults-group">
          <span class="group-label group-label--lg">Backup paths</span>
          <div
            v-if="(agent.default_backup_paths ?? []).length > 0"
            class="paths-list"
          >
            <code
              v-for="(p, idx) in agent.default_backup_paths ?? []"
              :key="idx"
              class="path-item mono"
            >
              {{ p }}
            </code>
          </div>
          <span
            v-else
            class="muted"
            >No default paths configured.</span
          >
        </div>

        <div class="defaults-group">
          <span class="group-label group-label--lg">Exclude patterns</span>
          <div
            v-if="(agent.default_exclude_patterns ?? []).length > 0"
            class="paths-list"
          >
            <code
              v-for="(p, idx) in agent.default_exclude_patterns ?? []"
              :key="idx"
              class="path-item mono"
            >
              {{ p }}
            </code>
          </div>
          <span
            v-else
            class="muted"
            >No default excludes configured.</span
          >
        </div>

        <div class="defaults-group">
          <span class="group-label group-label--lg">File change patterns</span>
          <div
            v-if="parseFileChangePatterns(agent.default_file_change_patterns_raw ?? '').length > 0"
            class="paths-list"
          >
            <code
              v-for="(p, idx) in parseFileChangePatterns(
                agent.default_file_change_patterns_raw ?? '',
              )"
              :key="idx"
              class="path-item mono"
            >
              {{ p.path }} <span class="fcp-action-badge">{{ p.action }}</span>
            </code>
          </div>
          <span
            v-else
            class="muted"
            >No default file change patterns configured.</span
          >
        </div>

        <div class="defaults-group">
          <span class="group-label group-label--lg">Pre-backup commands</span>
          <div
            v-if="agent.default_pre_backup_commands.length > 0"
            class="paths-list"
          >
            <code
              v-for="(cmd, idx) in agent.default_pre_backup_commands"
              :key="idx"
              class="path-item mono"
            >
              {{ cmd }}
            </code>
          </div>
          <span
            v-else
            class="muted"
            >None configured.</span
          >
        </div>

        <div class="defaults-group">
          <span class="group-label group-label--lg">Post-backup commands</span>
          <div
            v-if="agent.default_post_backup_commands.length > 0"
            class="paths-list"
          >
            <code
              v-for="(cmd, idx) in agent.default_post_backup_commands"
              :key="idx"
              class="path-item mono"
            >
              {{ cmd }}
            </code>
          </div>
          <span
            v-else
            class="muted"
            >None configured.</span
          >
        </div>
      </div>
    </template>

    <template #hint>
      Inherited by every schedule targeting this host unless the schedule overrides them.
      Schedule-specific hook commands are appended after the agent-level ones (pre) or prepended
      before them (post).
    </template>

    <template #edit>
      <label
        class="group-label group-label--lg"
        for="defaults-paths"
        >Backup paths</label
      >
      <textarea
        id="defaults-paths"
        v-model="pathsText"
        class="input defaults-area"
        placeholder="Directories to back up, one per line"
        spellcheck="false"
      />

      <label
        class="group-label group-label--lg"
        for="defaults-excludes"
        >Exclude patterns</label
      >
      <textarea
        id="defaults-excludes"
        v-model="excludesText"
        class="input defaults-area"
        placeholder="Exclude patterns, one per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
        spellcheck="false"
      />

      <label class="group-label group-label--lg">File change patterns</label>
      <FileChangePatternsEditor v-model="fcpText">
        <template #hint>
          Glob patterns matched against the full warning message, with actions:
          <code>ignore</code> (no warning), <code>warn</code> (default), <code>fatal</code> (fail
          backup). Checked after schedule-level patterns, as a fallback for this host.
          <code>*</code> does not match <code>/</code> - to cover every file under a directory, end
          the pattern with <code>**</code>, e.g. <code>/data/wal/**</code>.
        </template>
      </FileChangePatternsEditor>

      <label
        class="group-label group-label--lg"
        for="defaults-pre"
        >Pre-backup commands</label
      >
      <textarea
        id="defaults-pre"
        v-model="preCmdsText"
        class="input defaults-area"
        placeholder="Commands run before each backup, one per line&#10;e.g. systemctl stop myapp"
        spellcheck="false"
      />

      <label
        class="group-label group-label--lg"
        for="defaults-post"
        >Post-backup commands</label
      >
      <textarea
        id="defaults-post"
        v-model="postCmdsText"
        class="input defaults-area"
        placeholder="Commands run after each backup, one per line&#10;e.g. systemctl start myapp"
        spellcheck="false"
      />
    </template>
  </EditableSection>
</template>

<style scoped>
.defaults-area {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.defaults-groups {
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
  margin-bottom: 0.5rem;
}

.defaults-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.fcp-action-badge {
  font-family: var(--font-sans);
  font-size: var(--fs-2xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
</style>

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
import EditableInfoCard from './EditableInfoCard.vue'
import FileChangePatternsEditor from './FileChangePatternsEditor.vue'
import type { AgentRow } from '../types/agent'

/**
 * The four "defaults" cards on a host: backup paths, exclude patterns, file
 * change patterns and hook commands. Each had its own copy of the same save
 * handler, differing only in which field it replaced. See
 * docs/contributing/ui-design-audit.md (F-24).
 */
const props = defineProps<{
  agent: AgentRow
  /** False for imported hosts, which have no agent to push settings to. */
  canEdit: boolean
}>()

const emit = defineEmits<{ saved: [agent: AgentRow] }>()

/** The subset of an agent this panel owns. */
type Defaults = Pick<
  AgentRow,
  | 'default_backup_paths'
  | 'default_exclude_patterns'
  | 'default_pre_backup_commands'
  | 'default_post_backup_commands'
  | 'default_file_change_patterns_raw'
>

/**
 * The agent PUT is a whole-object replace, so every card has to send the
 * other three cards' values back untouched alongside its own patch.
 */
async function saveDefaults(patch: Partial<Defaults>): Promise<AgentRow> {
  const res = await apiClient.put<AgentRow>(`/agents/${props.agent.hostname}`, {
    display_name: props.agent.display_name,
    default_backup_paths: props.agent.default_backup_paths,
    default_exclude_patterns: props.agent.default_exclude_patterns,
    default_pre_backup_commands: props.agent.default_pre_backup_commands,
    default_post_backup_commands: props.agent.default_post_backup_commands,
    default_file_change_patterns_raw: props.agent.default_file_change_patterns_raw,
    ...patch,
  })
  emit('saved', res.data)
  return res.data
}

interface CardState {
  editing: boolean
  saving: boolean
  error: string | null
}

function cardState(): CardState {
  return { editing: false, saving: false, error: null }
}

/** Runs a card's save, holding it in the edit state if the request fails. */
async function submit(state: CardState, patch: () => Partial<Defaults>): Promise<void> {
  state.saving = true
  state.error = null
  try {
    await saveDefaults(patch())
    state.editing = false
  } catch (e: unknown) {
    state.error = extractError(e)
  } finally {
    state.saving = false
  }
}

const pathsCard = ref<CardState>(cardState())
const pathsText = ref('')

function startEditPaths(): void {
  pathsText.value = (props.agent.default_backup_paths ?? []).join('\n')
  pathsCard.value.error = null
  pathsCard.value.editing = true
}

const excludesCard = ref<CardState>(cardState())
const excludesText = ref('')

function startEditExcludes(): void {
  excludesText.value = (props.agent.default_exclude_patterns ?? []).join('\n')
  excludesCard.value.error = null
  excludesCard.value.editing = true
}

const fcpCard = ref<CardState>(cardState())
const fcpText = ref('')

function startEditFcp(): void {
  fcpText.value = props.agent.default_file_change_patterns_raw ?? ''
  fcpCard.value.error = null
  fcpCard.value.editing = true
}

const hookCard = ref<CardState>(cardState())
const preCmdsText = ref('')
const postCmdsText = ref('')

function startEditHookCmds(): void {
  preCmdsText.value = (props.agent.default_pre_backup_commands ?? []).join('\n')
  postCmdsText.value = (props.agent.default_post_backup_commands ?? []).join('\n')
  hookCard.value.error = null
  hookCard.value.editing = true
}
</script>

<template>
  <!-- Default Backup Paths -->
  <EditableInfoCard
    title="Default Backup Paths"
    :editing="pathsCard.editing"
    :can-edit="canEdit"
    :saving="pathsCard.saving"
    :error="pathsCard.error"
    @edit="startEditPaths"
    @cancel="pathsCard.editing = false"
    @save="submit(pathsCard, () => ({ default_backup_paths: parseLines(pathsText) }))"
  >
    <template #view>
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
    </template>
    <template #hint>Schedules with empty backup paths will use these defaults.</template>
    <template #edit>
      <textarea
        v-model="pathsText"
        class="input exclude-area"
        placeholder="Directories to back up, one per line"
        spellcheck="false"
      />
    </template>
  </EditableInfoCard>

  <!-- Default Exclude Patterns -->
  <EditableInfoCard
    title="Default Exclude Patterns"
    :editing="excludesCard.editing"
    :can-edit="canEdit"
    :saving="excludesCard.saving"
    :error="excludesCard.error"
    @edit="startEditExcludes"
    @cancel="excludesCard.editing = false"
    @save="submit(excludesCard, () => ({ default_exclude_patterns: parseLines(excludesText) }))"
  >
    <template #view>
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
    </template>
    <template #hint>
      Applied to all schedules on this host (unless schedule ignores them).
    </template>
    <template #edit>
      <textarea
        v-model="excludesText"
        class="input exclude-area"
        placeholder="Exclude patterns, one per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
        spellcheck="false"
      />
    </template>
  </EditableInfoCard>

  <!-- Default File Change Patterns -->
  <EditableInfoCard
    title="Default File Change Patterns"
    :editing="fcpCard.editing"
    :can-edit="canEdit"
    :saving="fcpCard.saving"
    :error="fcpCard.error"
    @edit="startEditFcp"
    @cancel="fcpCard.editing = false"
    @save="submit(fcpCard, () => ({ default_file_change_patterns_raw: fcpText }))"
  >
    <template #view>
      <div
        v-if="parseFileChangePatterns(agent.default_file_change_patterns_raw ?? '').length > 0"
        class="paths-list"
      >
        <code
          v-for="(p, idx) in parseFileChangePatterns(agent.default_file_change_patterns_raw ?? '')"
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
    </template>
    <template #hint>
      Applied to every schedule targeting this host, as a fallback for warnings not matched by a
      schedule-level pattern.
    </template>
    <template #edit>
      <FileChangePatternsEditor v-model="fcpText">
        <template #hint>
          Glob patterns matched against the full warning message, with actions:
          <code>ignore</code> (no warning), <code>warn</code> (default), <code>fatal</code> (fail
          backup). Checked after schedule-level patterns, as a fallback for this host.
          <code>*</code> does not match <code>/</code> - to cover every file under a directory, end
          the pattern with <code>**</code>, e.g. <code>/data/wal/**</code>.
        </template>
      </FileChangePatternsEditor>
    </template>
  </EditableInfoCard>

  <!-- Default Hook Commands -->
  <EditableInfoCard
    title="Default Hook Commands"
    :editing="hookCard.editing"
    :can-edit="canEdit"
    :saving="hookCard.saving"
    :error="hookCard.error"
    @edit="startEditHookCmds"
    @cancel="hookCard.editing = false"
    @save="
      submit(hookCard, () => ({
        default_pre_backup_commands: parseLines(preCmdsText),
        default_post_backup_commands: parseLines(postCmdsText),
      }))
    "
  >
    <template #view>
      <div class="field-hint">
        Run before and after every backup on this host. Schedule-specific commands are appended
        after the agent-level ones (pre) or prepended before them (post).
      </div>
      <div class="hook-cmds-view">
        <div class="hook-cmds-group">
          <span class="hook-cmds-label">Pre-backup</span>
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
        <div class="hook-cmds-group">
          <span class="hook-cmds-label">Post-backup</span>
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
    <template #edit>
      <label class="hook-cmds-label">Pre-backup Commands</label>
      <textarea
        v-model="preCmdsText"
        class="input exclude-area"
        placeholder="Commands run before each backup, one per line&#10;e.g. systemctl stop myapp"
        spellcheck="false"
      />
      <label class="hook-cmds-label">Post-backup Commands</label>
      <textarea
        v-model="postCmdsText"
        class="input exclude-area"
        placeholder="Commands run after each backup, one per line&#10;e.g. systemctl start myapp"
        spellcheck="false"
      />
    </template>
  </EditableInfoCard>
</template>

<style scoped>
.muted {
  color: var(--text-muted);
  font-size: var(--fs-base);
}

.exclude-area {
  min-height: 80px;
  resize: vertical;
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.5;
}

.paths-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin-bottom: 0.5rem;
}

.path-item {
  font-size: var(--fs-sm);
  padding: 0.2rem 0.5rem;
  background: var(--bg-input);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}

.fcp-action-badge {
  font-family: var(--font-sans);
  font-size: var(--fs-2xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.hook-cmds-view {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  margin-bottom: 0.5rem;
}

.hook-cmds-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.hook-cmds-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-bottom: 0.25rem;
  display: block;
}
</style>

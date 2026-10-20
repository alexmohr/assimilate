<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import BaseModal from './BaseModal.vue'
import BaseSpinner from './BaseSpinner.vue'
import { listAgents } from '../api/agents'
import { restoreArchiveFiles } from '../api/archives'
import { resolveArchiveDomain, resolveArchiveHost } from '../utils/archiveHost'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import {
  destinationOf,
  folderError,
  targetPath,
  type RestoreLocation,
} from '../utils/restoreDestination'
import type { ArchiveEntryResponse, RestoreRun } from '../types/generated'
import type { AgentRow } from '../types/agent'

/**
 * Where a restore goes, asked once, right before it starts: which agent, and
 * where on it - back where every path came from, or under one folder - with
 * the exact destination of each path spelled out underneath.
 *
 * It opens on the machine the archive was backed up from, original paths,
 * which is what a restore usually is; another agent or a folder is a choice
 * made here, in the open, rather than a field filled in somewhere else.
 */
const props = defineProps<{
  open: boolean
  repoId: number
  archive: ArchiveEntryResponse
  /** Archive paths to restore; empty restores the whole archive. */
  paths: readonly string[]
}>()

const emit = defineEmits<{ close: []; restored: [run: RestoreRun] }>()

/** How many destinations the preview spells out before summing up the rest. */
const PREVIEW_LIMIT = 5

const agents = ref<AgentRow[]>([])
const agentsLoading = ref(false)
const agentsError = ref<string | null>(null)
const agentId = ref<number | null>(null)
const locationKind = ref<RestoreLocation['kind']>('original')
const folder = ref('')
const submitting = ref(false)
const error = ref<string | null>(null)

const sourceHost = computed(() => resolveArchiveHost(props.archive))
const sourceDomain = computed(() => resolveArchiveDomain(props.archive))

function isSource(agent: AgentRow): boolean {
  return (
    agent.hostname === sourceHost.value && (agent.domain ?? null) === (sourceDomain.value ?? null)
  )
}

function agentLabel(agent: AgentRow): string {
  const name = agent.display_name ?? agent.hostname
  return agent.domain ? `${name} (${agent.domain})` : name
}

/** The machine the backup came from first, then whoever is online. */
const agentChoices = computed<AgentRow[]>(() =>
  [...agents.value].sort(
    (a, b) =>
      Number(isSource(b)) - Number(isSource(a)) ||
      Number(b.is_connected) - Number(a.is_connected) ||
      agentLabel(a).localeCompare(agentLabel(b)),
  ),
)

const agent = computed<AgentRow | null>(
  () => agents.value.find((a) => a.id === agentId.value) ?? null,
)

const folderProblem = computed<string | null>(() =>
  locationKind.value === 'folder' ? folderError(folder.value) : null,
)

const location = computed<RestoreLocation>(() =>
  locationKind.value === 'folder'
    ? { kind: 'folder', path: folder.value.trim().replace(/\/+$/, '') }
    : { kind: 'original' },
)

const preview = computed<{ from: string; to: string }[]>(() =>
  props.paths.slice(0, PREVIEW_LIMIT).map((path) => ({
    from: `/${path.replace(/^\/+/, '')}`,
    to: destinationOf(path, location.value),
  })),
)

const hiddenCount = computed(() => Math.max(0, props.paths.length - PREVIEW_LIMIT))

const what = computed(() =>
  props.paths.length === 0
    ? 'The whole archive'
    : props.paths.length === 1
      ? '1 item'
      : `${props.paths.length} items`,
)

const canSubmit = computed(
  () => agent.value !== null && folderProblem.value === null && !submitting.value,
)

async function loadAgents(): Promise<void> {
  agentsLoading.value = true
  agentsError.value = null
  try {
    agents.value = await listAgents()
    agentId.value = agents.value.find(isSource)?.id ?? null
  } catch (e: unknown) {
    agentsError.value = extractError(e)
  } finally {
    agentsLoading.value = false
  }
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    locationKind.value = 'original'
    folder.value = ''
    error.value = null
    void loadAgents()
  },
  { immediate: true },
)

async function submit(): Promise<void> {
  const target = agent.value
  if (target === null || !canSubmit.value) return
  submitting.value = true
  error.value = null
  try {
    const run = await restoreArchiveFiles(props.repoId, props.archive.name, {
      paths: [...props.paths],
      target_path: targetPath(location.value),
      hostname: target.hostname,
      domain: target.domain,
    })
    emit('restored', run)
    emit('close')
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Where to restore"
    size="lg"
    form
    @close="emit('close')"
    @submit="submit"
  >
    <div class="restore-dialog">
      <p class="muted restore-dialog-what">
        {{ what }} from <span class="mono">{{ archive.name }}</span> ({{
          formatDate(archive.start)
        }}), backed up from <strong>{{ sourceHost }}</strong
        >.
      </p>

      <div class="restore-dialog-columns">
        <fieldset class="restore-dialog-group">
          <legend class="group-label">Agent</legend>
          <div
            v-if="agentsLoading"
            class="loading-row"
          >
            <BaseSpinner size="sm" />
          </div>
          <div
            v-else-if="agentsError"
            class="error-banner"
          >
            {{ agentsError }}
          </div>
          <template v-else>
            <label
              v-for="choice in agentChoices"
              :key="choice.id"
              class="restore-choice"
              :class="{ 'restore-choice--picked': choice.id === agentId }"
            >
              <input
                v-model="agentId"
                type="radio"
                name="restore-agent"
                :value="choice.id"
              />
              <span class="restore-choice-text">
                <span class="restore-choice-name">{{ agentLabel(choice) }}</span>
                <span
                  v-if="isSource(choice)"
                  class="field-hint"
                  >the backup came from here</span
                >
                <span
                  v-else-if="!choice.is_connected"
                  class="field-hint"
                  >starts when it next connects</span
                >
              </span>
              <span
                class="badge"
                :class="choice.is_connected ? 'badge--success' : 'badge--neutral'"
                ><span class="badge-dot" />{{ choice.is_connected ? 'Online' : 'Offline' }}</span
              >
            </label>
            <p
              v-if="agentChoices.length === 0"
              class="muted"
            >
              No agents are registered yet.
            </p>
          </template>
        </fieldset>

        <fieldset class="restore-dialog-group">
          <legend class="group-label">
            Location on {{ agent ? agentLabel(agent) : 'the agent' }}
          </legend>
          <label class="restore-choice">
            <input
              v-model="locationKind"
              type="radio"
              name="restore-location"
              value="original"
            />
            <span class="restore-choice-text">
              <span class="restore-choice-name">Original paths</span>
              <span class="field-hint">Files already there are replaced.</span>
            </span>
          </label>
          <label class="restore-choice">
            <input
              v-model="locationKind"
              type="radio"
              name="restore-location"
              value="folder"
            />
            <span class="restore-choice-text">
              <span class="restore-choice-name">Into a folder</span>
              <span class="field-hint">Each path keeps its full path inside it.</span>
            </span>
          </label>
          <div
            v-if="locationKind === 'folder'"
            class="field restore-dialog-folder"
          >
            <label
              class="field-label"
              for="restore-folder"
              >Folder</label
            >
            <input
              id="restore-folder"
              v-model="folder"
              class="input mono"
              type="text"
              placeholder="/srv/restore"
              spellcheck="false"
              autocomplete="off"
              :aria-invalid="folderProblem !== null && folder !== ''"
            />
            <p
              v-if="folderProblem && folder !== ''"
              class="form-error"
            >
              {{ folderProblem }}
            </p>
          </div>
        </fieldset>
      </div>

      <div
        v-if="agent"
        class="restore-dialog-preview"
        data-testid="restore-preview"
      >
        <span class="group-label">Written on {{ agentLabel(agent) }} as</span>
        <p
          v-if="paths.length === 0"
          class="mono restore-dialog-path"
        >
          {{
            location.kind === 'original'
              ? 'every path back where it was backed up from'
              : `everything under ${location.path}/`
          }}
        </p>
        <ul
          v-else
          class="restore-dialog-paths"
        >
          <li
            v-for="item in preview"
            :key="item.from"
            class="mono restore-dialog-path"
          >
            {{ item.to }}
          </li>
          <li
            v-if="hiddenCount > 0"
            class="muted"
          >
            and {{ hiddenCount }} more
          </li>
        </ul>
      </div>

      <p
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </p>
    </div>

    <template #footer>
      <button
        class="btn btn-ghost"
        type="button"
        :disabled="submitting"
        @click="emit('close')"
      >
        Cancel
      </button>
      <button
        class="btn btn-primary"
        type="submit"
        :disabled="!canSubmit"
        data-testid="restore-submit"
      >
        {{
          submitting ? 'Starting...' : agent ? `Restore onto ${agentLabel(agent)}` : 'Pick an agent'
        }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.restore-dialog {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.restore-dialog-what {
  margin: 0;
  overflow-wrap: anywhere;
}

.restore-dialog-columns {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
  gap: var(--space-6);
}

.restore-dialog-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
}

.restore-choice {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  min-height: 2.75rem;
  padding: var(--space-3) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  cursor: pointer;
}

.restore-choice--picked {
  border-color: var(--accent);
  background: var(--accent-subtle);
}

.restore-choice-text {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.restore-choice-name {
  overflow-wrap: anywhere;
}

.restore-dialog-folder {
  margin: 0;
}

.restore-dialog-preview {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
}

.restore-dialog-paths {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin: 0;
  padding: 0;
  list-style: none;
}

.restore-dialog-path {
  margin: 0;
  font-size: var(--fs-xs);
  overflow-wrap: anywhere;
}
</style>

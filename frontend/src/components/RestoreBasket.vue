<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Download, File, Folder, RotateCcw, X } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import BaseHostLink from './BaseHostLink.vue'
import HelpHint from './HelpHint.vue'
import { archiveFilesDownloadUrl, restoreArchiveFiles } from '../api/archives'
import { useRestoreRun } from '../composables/useRestoreRun'
import { useAsyncAction } from '../composables/useAsyncAction'
import { isDirEntry } from '../composables/useRestoreTree'
import { badgeClass } from '../utils/badge'
import { formatBytes } from '../utils/format'
import { resolveArchiveDomain, resolveArchiveHost } from '../utils/archiveHost'
import { checkDestination, restoredLocation } from '../utils/restoreDestination'
import { restoreStatusLabel, restoreStatusTone } from '../utils/restoreRun'
import type {
  ArchiveEntryResponse as ArchiveEntry,
  ContentEntryResponse as ContentEntry,
  PathTotals,
} from '../types/generated'

/** How many destination previews the confirmation lists before summarising. */
const PREVIEW_COUNT = 3

const props = defineProps<{
  repoId: number
  archive: ArchiveEntry
  /** The basket: ticked paths, a ticked folder being one entry. */
  paths: string[]
  entryOf: (path: string) => ContentEntry | null
  totals: Record<string, PathTotals>
  isAdmin: boolean
}>()

const emit = defineEmits<{
  remove: [path: string]
  clear: []
}>()

const destinationInput = ref('')
const destinationCheck = computed(() => checkDestination(destinationInput.value))
const destination = computed(() =>
  destinationCheck.value.ok ? destinationCheck.value.destination : null,
)
const destinationError = computed(() =>
  destinationCheck.value.ok ? null : destinationCheck.value.error,
)

const host = computed(() => resolveArchiveHost(props.archive))
const hostIsAgent = computed(() => props.archive.matched === true)

const confirmOpen = ref(false)
const { loading: starting, error: restoreError, run } = useAsyncAction()
const { run: restoreRun, follow } = useRestoreRun()

// Another archive is another restore: what was started for the last one is
// no longer this basket's.
watch(
  () => props.archive.name,
  () => {
    restoreRun.value = null
    restoreError.value = null
  },
)

interface BasketItem {
  path: string
  isDir: boolean
  files: number | null
  bytes: number | null
}

const items = computed<BasketItem[]>(() =>
  props.paths.map((path) => {
    const entry = props.entryOf(path)
    const isDir = entry === null || isDirEntry(entry)
    const total = props.totals[path]
    if (total !== undefined) return { path, isDir, files: total.files, bytes: total.bytes }
    if (entry !== null && !isDir) return { path, isDir, files: 1, bytes: entry.size }
    return { path, isDir, files: null, bytes: null }
  }),
)

const summary = computed(() => {
  const counted = items.value.every((item) => item.files !== null)
  const files = items.value.reduce((sum, item) => sum + (item.files ?? 0), 0)
  const bytes = items.value.reduce((sum, item) => sum + (item.bytes ?? 0), 0)
  return { counted, files, bytes }
})

const summaryText = computed(() => {
  const { counted, files, bytes } = summary.value
  if (!counted) return 'Counting...'
  return `${files} ${files === 1 ? 'file' : 'files'}, ${formatBytes(bytes)}`
})

const downloadUrl = computed(() =>
  archiveFilesDownloadUrl(props.repoId, props.archive.name, props.paths),
)

const canAct = computed(() => props.paths.length > 0 && destinationError.value === null)

const destinationLabel = computed(() => destination.value ?? 'Original locations')

const previews = computed(() =>
  props.paths
    .slice(0, PREVIEW_COUNT)
    .map((path) => ({ path, to: restoredLocation(destination.value, path) })),
)

function itemSize(item: BasketItem): string {
  if (item.files === null || item.bytes === null) return ''
  return item.isDir
    ? `${item.files} ${item.files === 1 ? 'file' : 'files'}, ${formatBytes(item.bytes)}`
    : formatBytes(item.bytes)
}

/** A recorded restore's target, saying so when it is the original locations. */
// A run's target is null only for a database restore, which the basket never
// starts: the files it restores always land somewhere, `/` by default.
function describeTarget(runTarget: string | null): string {
  const target = runTarget ?? '/'
  const check = checkDestination(target)
  return check.ok && check.destination === null ? 'original locations' : target
}

async function startRestore(): Promise<void> {
  await run(async () => {
    const started = await restoreArchiveFiles(props.repoId, props.archive.name, {
      paths: props.paths,
      target_path: destination.value ?? '/',
      hostname: host.value,
      domain: resolveArchiveDomain(props.archive),
    })
    follow(started)
  })
  if (restoreError.value === null) confirmOpen.value = false
}
</script>

<template>
  <section
    class="panel panel--sectioned"
    data-testid="restore-basket"
    aria-labelledby="restore-basket-title"
  >
    <div class="panel-header">
      <h2
        id="restore-basket-title"
        class="panel-title"
      >
        Restore basket
      </h2>
      <button
        v-if="paths.length > 0"
        type="button"
        class="btn btn-xs btn-ghost"
        @click="emit('clear')"
      >
        Clear
      </button>
    </div>
    <div class="panel-body basket-body">
      <p
        v-if="paths.length === 0"
        class="muted"
      >
        Tick files or folders in the tree to add them here.
      </p>
      <ul
        v-else
        class="basket-list"
      >
        <li
          v-for="item in items"
          :key="item.path"
          class="basket-item"
          :data-path="item.path"
        >
          <Folder
            v-if="item.isDir"
            :size="14"
            class="basket-icon"
          />
          <File
            v-else
            :size="14"
            class="basket-icon"
          />
          <span
            class="basket-path mono"
            :title="item.path"
            >{{ item.path }}</span
          >
          <span class="basket-size">{{ itemSize(item) }}</span>
          <button
            type="button"
            class="btn btn-xs btn-ghost"
            :aria-label="`Remove ${item.path}`"
            @click="emit('remove', item.path)"
          >
            <X :size="14" />
          </button>
        </li>
      </ul>

      <div
        class="basket-total"
        data-testid="basket-total"
      >
        <span class="group-label">Total</span>
        <span>{{ paths.length === 0 ? 'Nothing selected' : summaryText }}</span>
      </div>

      <div class="field">
        <div class="field-label-row field-label-row--tight">
          <label
            class="field-label"
            for="restore-destination"
            >Destination</label
          >
          <HelpHint label="About the destination">
            Every path is restored under this directory with its full archive path, so
            <code>etc/nginx/nginx.conf</code> restored to <code>/srv/restore</code> becomes
            <code>/srv/restore/etc/nginx/nginx.conf</code>. Leave it empty to put every path back
            where it was backed up from.
          </HelpHint>
        </div>
        <input
          id="restore-destination"
          v-model="destinationInput"
          class="input"
          type="text"
          placeholder="Original locations"
          spellcheck="false"
          autocomplete="off"
          :aria-invalid="destinationError !== null"
        />
        <p
          v-if="destinationError"
          class="form-error"
        >
          {{ destinationError }}
        </p>
        <p
          v-else
          class="field-hint"
        >
          Leave empty to restore to the original paths.
        </p>
      </div>

      <dl class="info-grid">
        <dt>Host</dt>
        <dd>
          <BaseHostLink :hostname="host" />
        </dd>
      </dl>
      <p
        v-if="isAdmin && !hostIsAgent"
        class="field-hint"
      >
        No agent claims this archive's host, so it can only be downloaded.
      </p>

      <div
        v-if="restoreRun"
        class="basket-status"
        data-testid="basket-restore-status"
      >
        <span
          class="badge"
          :class="badgeClass(restoreStatusTone(restoreRun.status))"
          >{{ restoreStatusLabel(restoreRun.status) }}</span
        >
        <span
          v-if="restoreRun.status === 'failed'"
          class="form-error"
          >{{ restoreRun.error_message ?? 'The restore failed.' }}</span
        >
        <span
          v-else
          class="muted"
          >{{ restoreRun.paths.length }} {{ restoreRun.paths.length === 1 ? 'path' : 'paths' }} onto
          {{ restoreRun.hostname }}, {{ describeTarget(restoreRun.target_path) }}</span
        >
      </div>

      <div class="basket-actions">
        <a
          class="btn btn-sm btn-ghost"
          :class="{ 'basket-action-disabled': !canAct }"
          :href="canAct ? downloadUrl : undefined"
          :aria-disabled="!canAct"
          download
          data-testid="basket-download"
        >
          <Download :size="14" />
          Download
        </a>
        <button
          v-if="isAdmin"
          type="button"
          class="btn btn-sm btn-primary"
          :disabled="!canAct || !hostIsAgent || starting"
          @click="confirmOpen = true"
        >
          <RotateCcw :size="14" />
          Restore to host
        </button>
      </div>
    </div>

    <BaseModal
      :open="confirmOpen"
      title="Restore to host"
      @close="confirmOpen = false"
    >
      <p>
        Restore {{ paths.length }} {{ paths.length === 1 ? 'path' : 'paths' }} ({{ summaryText }})
        from <span class="mono">{{ archive.name }}</span> onto <strong>{{ host }}</strong
        >.
      </p>
      <dl class="info-grid confirm-grid">
        <dt>Destination</dt>
        <dd data-testid="restore-confirm-destination">{{ destinationLabel }}</dd>
        <template
          v-for="preview in previews"
          :key="preview.path"
        >
          <dt class="mono">{{ preview.path }}</dt>
          <dd class="mono">{{ preview.to }}</dd>
        </template>
      </dl>
      <p
        v-if="paths.length > previews.length"
        class="muted"
      >
        and {{ paths.length - previews.length }} more.
      </p>
      <p class="token-warning">
        {{
          destination === null
            ? 'Files at their original locations are overwritten without another prompt.'
            : `Existing files under ${destination} are overwritten without another prompt.`
        }}
      </p>
      <p
        v-if="restoreError"
        class="form-error"
      >
        {{ restoreError }}
      </p>
      <template #footer>
        <button
          type="button"
          class="btn btn-ghost"
          @click="confirmOpen = false"
        >
          Cancel
        </button>
        <button
          type="button"
          class="btn btn-primary"
          :disabled="starting"
          @click="startRestore"
        >
          {{ starting ? 'Starting...' : 'Restore' }}
        </button>
      </template>
    </BaseModal>
  </section>
</template>

<style scoped>
.basket-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.basket-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  max-height: 40vh;
  overflow-y: auto;
}

.basket-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--fs-sm);
}

.basket-icon {
  flex-shrink: 0;
  color: var(--text-muted);
}

.basket-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.basket-size {
  flex-shrink: 0;
  color: var(--text-muted);
  white-space: nowrap;
}

.basket-total {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
  font-variant-numeric: tabular-nums;
}

.basket-status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}

.basket-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--space-4);
}

.basket-action-disabled {
  opacity: 0.5;
  pointer-events: none;
}

.confirm-grid {
  margin: var(--space-5) 0;
}
</style>

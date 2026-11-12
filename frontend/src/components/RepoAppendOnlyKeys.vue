<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { Copy } from '@lucide/vue'
import { relativeTime } from '../utils/format'
import { useClipboard } from '../composables/useClipboard'
import type { AppendOnlyStatusResponse } from '../types/generated'

/**
 * The keys Assimilate keeps on the repository host: which are installed, a
 * way to write them again, and the exact lines for a host managed by hand.
 */
const props = defineProps<{
  status: AppendOnlyStatusResponse
  busy: boolean
}>()

const emit = defineEmits<{ reapply: [] }>()

const showLines = ref(false)
const { copied: copiedKey, copy } = useClipboard()
const copiedWhich = ref<'backup' | 'maintenance' | null>(null)

async function copyLine(which: 'backup' | 'maintenance', line: string): Promise<void> {
  await copy(line)
  copiedWhich.value = which
}

function lineCopied(which: 'backup' | 'maintenance'): boolean {
  return copiedKey.value && copiedWhich.value === which
}
</script>

<template>
  <div class="panel">
    <div class="pane-section-head">
      <h3 class="panel-title">Set up by Assimilate</h3>
      <span
        v-if="status.installed_at"
        class="badge badge--success"
      >
        Installed {{ relativeTime(status.installed_at) }}
      </span>
      <span
        v-else
        class="badge badge--neutral"
      >
        Not installed
      </span>
    </div>
    <p class="field-hint keys-lede">
      The key agents use is restricted to
      <span class="mono">borg serve --append-only</span>. A second, full-access maintenance key
      stays on the server.
      <template v-if="status.maintenance_key">
        Maintenance key
        <span class="mono">{{ status.maintenance_key.fingerprint }}</span
        >.
      </template>
    </p>
    <div class="panel-actions">
      <button
        type="button"
        class="btn btn-sm"
        :disabled="busy"
        @click="emit('reapply')"
      >
        {{ busy ? 'Working...' : 'Reapply' }}
      </button>
      <button
        type="button"
        class="btn btn-sm btn-ghost"
        :aria-expanded="showLines"
        @click="showLines = !showLines"
      >
        {{ showLines ? 'Hide the lines' : 'Show the lines' }}
      </button>
    </div>

    <div
      v-if="showLines"
      class="keys-lines"
    >
      <p class="field-hint">
        The lines Assimilate keeps in the host's <span class="mono">authorized_keys</span>, for
        hosts you manage by hand. Paste them in place of the server key's line.
      </p>
      <div class="keys-block">
        <div class="pane-section-head">
          <span class="field-label">Backup key</span>
          <span class="badge badge--violet">Append-only</span>
          <span class="muted mono">{{ props.status.backup_key.fingerprint }}</span>
        </div>
        <pre class="detail-pre">{{ status.backup_key.authorized_keys_line }}</pre>
        <button
          type="button"
          class="btn btn-sm"
          @click="copyLine('backup', status.backup_key.authorized_keys_line)"
        >
          <Copy :size="14" />
          {{ lineCopied('backup') ? 'Copied' : 'Copy line' }}
        </button>
      </div>
      <div
        v-if="status.maintenance_key"
        class="keys-block"
      >
        <div class="pane-section-head">
          <span class="field-label">Maintenance key</span>
          <span class="badge badge--warning">Full access</span>
          <span class="muted mono">{{ status.maintenance_key.fingerprint }}</span>
        </div>
        <pre class="detail-pre">{{ status.maintenance_key.authorized_keys_line }}</pre>
        <button
          type="button"
          class="btn btn-sm"
          @click="copyLine('maintenance', status.maintenance_key.authorized_keys_line)"
        >
          <Copy :size="14" />
          {{ lineCopied('maintenance') ? 'Copied' : 'Copy line' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.keys-lede {
  margin: var(--space-4) 0;
}

.keys-lines {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
  margin-top: var(--space-6);
}

.keys-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-4);
}
</style>

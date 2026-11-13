<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { Check, X } from '@lucide/vue'
import { relativeTime } from '../utils/format'
import type { AppendOnlyCheckResponse } from '../types/generated'

/**
 * What connecting to the host with each key showed. The restriction line is
 * read from the host's `authorized_keys`: borg cannot be asked whether a
 * connection is append-only, so none of this proves borg refuses a deletion.
 */
const props = defineProps<{
  check: AppendOnlyCheckResponse | null
  busy: boolean
}>()

const emit = defineEmits<{ check: [] }>()

interface CheckRow {
  key: string
  ok: boolean
  label: string
}

const rows = computed<CheckRow[]>(() =>
  props.check === null
    ? []
    : [
        { key: 'backup', ok: props.check.backup_key_ok, label: 'Backup key reaches borg' },
        {
          key: 'maintenance',
          ok: props.check.maintenance_key_ok,
          label: 'Maintenance key reaches the host',
        },
        {
          key: 'restriction',
          ok: props.check.restriction_in_place,
          label: "Host's authorized_keys restricts the backup key to append-only",
        },
      ],
)
</script>

<template>
  <div class="panel">
    <div class="pane-section-head">
      <h3 class="panel-title">Connection check</h3>
      <span
        v-if="check"
        class="muted"
      >
        checked {{ relativeTime(check.checked_at) }}
      </span>
      <button
        type="button"
        class="btn btn-sm"
        :disabled="busy"
        @click="emit('check')"
      >
        {{ busy ? 'Checking...' : 'Check now' }}
      </button>
    </div>
    <p
      v-if="check === null"
      class="muted"
    >
      Not checked yet.
    </p>
    <ul
      v-else
      class="check-list"
    >
      <li
        v-for="row in rows"
        :key="row.key"
        :class="row.ok ? 'check-ok' : 'check-bad'"
      >
        <Check
          v-if="row.ok"
          :size="16"
        />
        <X
          v-else
          :size="16"
        />
        <span>{{ row.label }}</span>
      </li>
    </ul>
    <div
      v-if="check?.error"
      class="form-error"
    >
      {{ check.error }}
    </div>
    <p class="field-hint check-note">
      The restriction line is read from the host's authorized_keys. borg cannot report whether a
      connection is append-only, so this does not prove borg refuses deletions. Each maintenance run
      compares borg's archive list with Assimilate's and stops if anything vanished.
    </p>
  </div>
</template>

<style scoped>
.check-list {
  list-style: none;
  margin: var(--space-4) 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  font-size: var(--fs-sm);
}

.check-list li {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.check-ok {
  color: var(--success);
}

.check-bad {
  color: var(--danger);
}

.check-note {
  margin: var(--space-4) 0 0;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
/**
 * What a group's members may do on each repository. Every member holds these
 * permissions on top of their own grants, so this is where a team's access is
 * set once instead of user by user.
 */
import { ref, watch } from 'vue'
import { listGroupPermissions, setGroupPermission } from '../api/groups'
import type { Group } from '../api/groups'
import { listRepos } from '../api/repos'
import type { RepoPermission } from '../types/generated'
import { extractError } from '../utils/error'
import BaseModal from './BaseModal.vue'
import BaseSpinner from './BaseSpinner.vue'
import ModalFormActions from './ModalFormActions.vue'

const props = defineProps<{
  open: boolean
  group: Group | null
}>()

const emit = defineEmits<{ close: [] }>()

const PERMISSION_COLUMNS: readonly { key: RepoPermission; label: string }[] = [
  { key: 'view', label: 'View' },
  { key: 'backup', label: 'Backup' },
  { key: 'modify_schedules', label: 'Schedules' },
  { key: 'extract', label: 'Extract' },
  { key: 'delete', label: 'Delete' },
]

interface Row {
  repoId: number
  repoName: string
  /** What the group holds now, as saved. */
  saved: RepoPermission[]
  /** What the dialog will save. */
  draft: RepoPermission[]
}

const rows = ref<Row[]>([])
const loading = ref(false)
const saving = ref(false)
const error = ref<string | null>(null)

async function load(group: Group): Promise<void> {
  loading.value = true
  error.value = null
  rows.value = []
  try {
    const [repos, grants] = await Promise.all([listRepos(), listGroupPermissions(group.id)])
    rows.value = repos.map((repo) => {
      const saved = grants.find((grant) => grant.repo_id === repo.id)?.permissions ?? []
      return { repoId: repo.id, repoName: repo.name, saved, draft: [...saved] }
    })
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to load the group permissions')
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.open, props.group] as const,
  ([open, group]) => {
    if (open && group) void load(group)
  },
  { immediate: true },
)

function toggle(row: Row, permission: RepoPermission): void {
  row.draft = row.draft.includes(permission)
    ? row.draft.filter((held) => held !== permission)
    : [...row.draft, permission]
}

function changed(row: Row): boolean {
  return (
    row.draft.length !== row.saved.length || row.draft.some((held) => !row.saved.includes(held))
  )
}

async function save(): Promise<void> {
  const group = props.group
  if (!group) return
  saving.value = true
  error.value = null
  try {
    for (const row of rows.value.filter(changed)) {
      const result = await setGroupPermission(group.id, row.repoId, row.draft)
      row.saved = result.permissions
      row.draft = [...result.permissions]
    }
    emit('close')
  } catch (e: unknown) {
    error.value = extractError(e, 'Failed to save the group permissions')
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <BaseModal
    :open="open"
    size="lg"
    title="Group repository permissions"
    @close="emit('close')"
  >
    <p class="field-hint">
      Every member of <strong>{{ group?.name }}</strong> may do this on each repository, on top of
      their own permissions.
    </p>
    <BaseSpinner
      v-if="loading"
      size="sm"
    />
    <p
      v-else-if="rows.length === 0"
      class="field-hint"
    >
      No repositories configured yet.
    </p>
    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table data-table--compact">
        <thead>
          <tr>
            <th>Repository</th>
            <th
              v-for="c in PERMISSION_COLUMNS"
              :key="c.key"
            >
              {{ c.label }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="row in rows"
            :key="row.repoId"
            :data-repo="row.repoName"
          >
            <td class="cell-mono">{{ row.repoName }}</td>
            <td
              v-for="c in PERMISSION_COLUMNS"
              :key="c.key"
            >
              <input
                type="checkbox"
                :checked="row.draft.includes(c.key)"
                :aria-label="`${c.label} on ${row.repoName}`"
                :disabled="saving"
                @change="toggle(row, c.key)"
              />
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <template #footer>
      <ModalFormActions
        type="button"
        :submitting="saving"
        :error="error"
        submit-label="Save permissions"
        submitting-label="Saving..."
        @cancel="emit('close')"
        @confirm="save"
      />
    </template>
  </BaseModal>
</template>

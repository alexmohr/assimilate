<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Plus, Trash2, ShieldCheck } from '@lucide/vue'
import { apiClient } from '../api/client'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from '../components/BaseSpinner.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import BaseModal from '../components/BaseModal.vue'
import EmptyState from '../components/EmptyState.vue'

interface Role {
  id: number
  name: string
  is_seeded: boolean
  can_create_agent: boolean
  can_delete_agent: boolean
  can_delete_own_agent: boolean
  can_create_repo: boolean
  can_delete_repo: boolean
  can_delete_own_repo: boolean
  can_create_schedule: boolean
  can_delete_schedule: boolean
  can_delete_own_schedule: boolean
  can_manage_tags: boolean
  can_view_all_repos: boolean
  can_manage_tunnels: boolean
  can_upgrade_agent: boolean
}

type PermissionKey =
  | 'can_create_agent'
  | 'can_delete_agent'
  | 'can_delete_own_agent'
  | 'can_create_repo'
  | 'can_delete_repo'
  | 'can_delete_own_repo'
  | 'can_create_schedule'
  | 'can_delete_schedule'
  | 'can_delete_own_schedule'
  | 'can_manage_tags'
  | 'can_view_all_repos'
  | 'can_manage_tunnels'
  | 'can_upgrade_agent'

const PERMISSION_LABELS: { key: PermissionKey; label: string }[] = [
  { key: 'can_create_agent', label: 'Create Agent' },
  { key: 'can_delete_agent', label: 'Delete Agent' },
  { key: 'can_delete_own_agent', label: 'Delete Own Agent' },
  { key: 'can_create_repo', label: 'Create Repo' },
  { key: 'can_delete_repo', label: 'Delete Repo' },
  { key: 'can_delete_own_repo', label: 'Delete Own Repo' },
  { key: 'can_create_schedule', label: 'Create Schedule' },
  { key: 'can_delete_schedule', label: 'Delete Schedule' },
  { key: 'can_delete_own_schedule', label: 'Delete Own Schedule' },
  { key: 'can_manage_tags', label: 'Manage Tags' },
  { key: 'can_view_all_repos', label: 'View All Repos' },
  { key: 'can_manage_tunnels', label: 'Manage Tunnels' },
  { key: 'can_upgrade_agent', label: 'Upgrade Agent' },
]

const SEEDED_ROLES = new Set(['admin', 'operator', 'viewer'])

const roles = ref<Role[]>([])
const { loading, error, run } = useAsyncAction('Failed to load roles')
loading.value = true

function emptyPerms(): Record<PermissionKey, boolean> {
  return {
    can_create_agent: false,
    can_delete_agent: false,
    can_delete_own_agent: false,
    can_create_repo: false,
    can_delete_repo: false,
    can_delete_own_repo: false,
    can_create_schedule: false,
    can_delete_schedule: false,
    can_delete_own_schedule: false,
    can_manage_tags: false,
    can_view_all_repos: false,
    can_manage_tunnels: false,
    can_upgrade_agent: false,
  }
}

const showCreateModal = ref(false)
const createForm = ref<{ name: string } & Record<PermissionKey, boolean>>({
  name: '',
  ...emptyPerms(),
})
const {
  loading: createSubmitting,
  error: createError,
  run: runCreate,
} = useAsyncAction('Failed to create role')

const showEditModal = ref(false)
const editTarget = ref<Role | null>(null)
const editForm = ref<Record<PermissionKey, boolean>>(emptyPerms())
const {
  loading: editSubmitting,
  error: editError,
  run: runEdit,
} = useAsyncAction('Failed to update role')

const showDeleteModal = ref(false)
const deleteTarget = ref<Role | null>(null)
const {
  loading: deleteSubmitting,
  error: deleteError,
  run: runDelete,
} = useAsyncAction('Failed to delete role')

const filterText = ref('')

const filteredRoles = computed((): Role[] => {
  if (!filterText.value.trim()) return roles.value
  const q = filterText.value.toLowerCase()
  return roles.value.filter((r) => r.name.toLowerCase().includes(q))
})

function isSeeded(role: Role): boolean {
  return role.is_seeded || SEEDED_ROLES.has(role.name)
}

function permissionCount(role: Role): number {
  return PERMISSION_LABELS.filter((p) => role[p.key]).length
}

async function fetchRoles(): Promise<void> {
  await run(async () => {
    const res = await apiClient.get<Role[]>('/roles')
    roles.value = res.data
  })
}

function openCreate(): void {
  createForm.value = { name: '', ...emptyPerms() }
  createError.value = null
  showCreateModal.value = true
}

async function submitCreate(): Promise<void> {
  if (!createForm.value.name.trim()) {
    createError.value = 'Name is required'
    return
  }
  await runCreate(async () => {
    await apiClient.post('/roles', {
      name: createForm.value.name.trim(),
      can_create_agent: createForm.value.can_create_agent,
      can_delete_agent: createForm.value.can_delete_agent,
      can_delete_own_agent: createForm.value.can_delete_own_agent,
      can_create_repo: createForm.value.can_create_repo,
      can_delete_repo: createForm.value.can_delete_repo,
      can_delete_own_repo: createForm.value.can_delete_own_repo,
      can_create_schedule: createForm.value.can_create_schedule,
      can_delete_schedule: createForm.value.can_delete_schedule,
      can_delete_own_schedule: createForm.value.can_delete_own_schedule,
      can_manage_tags: createForm.value.can_manage_tags,
      can_view_all_repos: createForm.value.can_view_all_repos,
      can_manage_tunnels: createForm.value.can_manage_tunnels,
      can_upgrade_agent: createForm.value.can_upgrade_agent,
    })
    showCreateModal.value = false
    await fetchRoles()
  })
}

function openEdit(role: Role): void {
  editTarget.value = role
  const perms = emptyPerms()
  for (const key of Object.keys(perms) as PermissionKey[]) {
    perms[key] = role[key]
  }
  editForm.value = perms
  editError.value = null
  showEditModal.value = true
}

async function submitEdit(): Promise<void> {
  const target = editTarget.value
  if (!target) return
  await runEdit(async () => {
    await apiClient.put(`/roles/${target.id}`, editForm.value)
    showEditModal.value = false
    await fetchRoles()
  })
}

function openDelete(role: Role): void {
  deleteTarget.value = role
  deleteError.value = null
  showDeleteModal.value = true
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value
  if (!target) return
  await runDelete(async () => {
    await apiClient.delete(`/roles/${target.id}`)
    showDeleteModal.value = false
    await fetchRoles()
  })
}

onMounted((): void => {
  void fetchRoles()
})
</script>

<template>
  <div class="roles-page">
    <div class="page-header">
      <h1 class="page-title">Roles</h1>
      <div class="header-actions">
        <button
          class="btn btn-primary"
          @click="openCreate"
        >
          <Plus :size="14" />
          New
        </button>
      </div>
    </div>

    <p class="page-description">
      Roles define sets of system-wide permissions that control what actions a user can perform.
      Assign a role to a user to grant capabilities like creating agents, managing repositories, or
      configuring schedules. Unlike groups, roles do not control access to specific repositories.
    </p>

    <div class="toolbar">
      <input
        v-model="filterText"
        class="input search-input"
        placeholder="Filter roles..."
      />
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="roles.length === 0"
      :icon="ShieldCheck"
      title="No roles yet"
      description="Roles bundle permissions so they can be granted to users and groups."
      action="New role"
      @action="showCreateModal = true"
    />

    <!-- Permission Matrix -->
    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table data-table--compact">
        <thead>
          <tr>
            <th class="role-name-col">Role</th>
            <th
              v-for="perm in PERMISSION_LABELS"
              :key="perm.key"
              class="perm-col"
            >
              <span class="perm-header">{{ perm.label }}</span>
            </th>
            <th class="actions-col">Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="role in filteredRoles"
            :key="role.id"
            :class="{ seeded: isSeeded(role) }"
          >
            <td class="role-name-cell">
              <span class="role-name">{{ role.name }}</span>
              <span
                v-if="isSeeded(role)"
                class="badge badge--neutral"
                >built-in</span
              >
              <span class="perm-count"
                >{{ permissionCount(role) }}/{{ PERMISSION_LABELS.length }}</span
              >
            </td>
            <td
              v-for="perm in PERMISSION_LABELS"
              :key="perm.key"
              class="perm-cell"
            >
              <span
                class="perm-indicator"
                :class="role[perm.key] ? 'perm-yes' : 'perm-no'"
              >
                {{ role[perm.key] ? '\u2713' : '\u2715' }}
              </span>
            </td>
            <td class="actions-cell">
              <button
                class="btn btn-sm btn-ghost"
                @click="openEdit(role)"
              >
                Edit
              </button>
              <button
                class="btn btn-sm btn-ghost btn-danger-text"
                :disabled="isSeeded(role)"
                :title="isSeeded(role) ? 'Cannot delete built-in role' : 'Delete role'"
                @click="openDelete(role)"
              >
                <Trash2 :size="14" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- Create Role Modal -->
    <BaseModal
      :open="showCreateModal"
      size="lg"
      title="New role"
      form
      @close="showCreateModal = false"
      @submit="submitCreate"
    >
      <div class="field">
        <label
          class="field-label"
          for="create-role-name"
          >Name <span class="required">*</span></label
        >
        <input
          id="create-role-name"
          v-model="createForm.name"
          type="text"
          class="input"
          required
        />
      </div>
      <div class="permissions-grid">
        <label
          v-for="perm in PERMISSION_LABELS"
          :key="perm.key"
          class="perm-checkbox"
        >
          <input
            v-model="createForm[perm.key]"
            type="checkbox"
          />
          <span>{{ perm.label }}</span>
        </label>
      </div>

      <template #footer>
        <ModalFormActions
          :submitting="createSubmitting"
          :disabled="!createForm.name.trim()"
          :error="createError"
          submit-label="Create"
          submitting-label="Creating..."
          @cancel="showCreateModal = false"
        />
      </template>
    </BaseModal>

    <!-- Edit Role Modal -->
    <BaseModal
      :open="showEditModal"
      size="lg"
      form
      @close="showEditModal = false"
      @submit="submitEdit"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Edit Role &mdash; {{ editTarget?.name }}
        </h2>
      </template>
      <div class="permissions-grid">
        <label
          v-for="perm in PERMISSION_LABELS"
          :key="perm.key"
          class="perm-checkbox"
        >
          <input
            v-model="editForm[perm.key]"
            type="checkbox"
          />
          <span>{{ perm.label }}</span>
        </label>
      </div>

      <template #footer>
        <ModalFormActions
          :submitting="editSubmitting"
          :error="editError"
          submit-label="Save"
          submitting-label="Saving..."
          @cancel="showEditModal = false"
        />
      </template>
    </BaseModal>

    <!-- Delete Role Modal -->
    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete role"
      :submitting="deleteSubmitting"
      :error="deleteError"
      @cancel="showDeleteModal = false"
      @confirm="confirmDelete"
    >
      Are you sure you want to delete the role <strong>{{ deleteTarget?.name }}</strong
      >? Users assigned this role will lose its permissions.
    </ConfirmDeleteDialog>
  </div>
</template>

<style scoped>
.roles-page {
  max-width: 1200px;
}

@media (max-width: 768px) {
  .page-description {
    display: none;
  }
}

.role-name-col {
  text-align: left !important;
  min-width: 140px;
}

.perm-col {
  width: 70px;
  min-width: 60px;
}

.perm-header {
  display: block;
  writing-mode: vertical-rl;
  text-orientation: mixed;
  transform: rotate(180deg);
  white-space: nowrap;
  padding: var(--space-2) 0;
}

.actions-col {
  text-align: right !important;
  min-width: 120px;
}

.data-table tr.seeded {
  background: var(--bg-hover);
}

.role-name-cell {
  text-align: left !important;
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.role-name {
  font-weight: 600;
  font-size: var(--fs-base);
}

.perm-count {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  margin-left: auto;
}

.perm-cell {
  text-align: center;
}

.perm-indicator {
  font-size: var(--fs-xs);
  font-weight: 700;
}

.perm-yes {
  color: var(--success);
}

.perm-no {
  color: var(--text-muted);
  opacity: 0.4;
}

.actions-cell {
  justify-content: flex-end;
}

.permissions-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--space-4);
  padding: var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
}

.perm-checkbox {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--fs-sm);
  cursor: pointer;
  color: var(--text-primary);
}

.perm-checkbox input[type='checkbox'] {
  accent-color: var(--accent);
  cursor: pointer;
}
</style>

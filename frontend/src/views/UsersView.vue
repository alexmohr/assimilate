<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useAuthStore } from '../stores/auth'
import { formatDate } from '../utils/format'
import { extractError } from '../utils/error'
import { Plus, Pencil, Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import type { Repo } from '../types/repo'
import BaseModal from '../components/BaseModal.vue'
import BaseTabs, { type TabOption } from '../components/BaseTabs.vue'

interface User {
  id: number
  username: string
  role: 'admin' | 'user'
  created_at: string
  last_login_at: string | null
}

interface RepoPermission {
  user_id: number
  repo_id: number
  can_view: boolean
  can_backup: boolean
  can_modify_schedules: boolean
  can_extract: boolean
  can_delete: boolean
}

interface RoleRow {
  id: number
  name: string
}

interface GroupRow {
  id: number
  name: string
}

const authStore = useAuthStore()
const users = ref<User[]>([])
const loading = ref(true)

const showCreateModal = ref(false)
const createForm = ref({ username: '', password: '', role: 'user' as 'admin' | 'user' })
const createError = ref('')
const createSubmitting = ref(false)

const showDeleteModal = ref(false)
const deleteTarget = ref<User | null>(null)
const deleteSubmitting = ref(false)

const showEditModal = ref(false)
const editUser = ref<User | null>(null)
type EditTab = 'general' | 'password' | 'roles' | 'permissions'

const editTabs: TabOption<EditTab>[] = [
  { id: 'general', label: 'General' },
  { id: 'password', label: 'Password' },
  { id: 'roles', label: 'Roles & Groups' },
  { id: 'permissions', label: 'Permissions' },
]

const editTab = ref<EditTab>('general')
const editRole = ref<'admin' | 'user'>('user')
const editRoleSubmitting = ref(false)
const editRoleError = ref('')

const editPassword = ref('')
const editPasswordSubmitting = ref(false)
const editPasswordError = ref('')
const editPasswordSuccess = ref(false)

const editRolesLoading = ref(false)
const allRoles = ref<RoleRow[]>([])
const allGroups = ref<GroupRow[]>([])
const userRoleIds = ref<number[]>([])
const userGroupIds = ref<number[]>([])
const editRolesSubmitting = ref(false)
const editRolesError = ref('')

const editPermsLoading = ref(false)
const permissionsRepos = ref<Repo[]>([])
const permissionsData = ref<RepoPermission[]>([])

async function fetchUsers(): Promise<void> {
  loading.value = true
  try {
    const res = await apiClient.get<User[]>('/users')
    users.value = res.data
  } finally {
    loading.value = false
  }
}

function openCreate(): void {
  createForm.value = { username: '', password: '', role: 'user' }
  createError.value = ''
  showCreateModal.value = true
}

async function submitCreate(): Promise<void> {
  createError.value = ''
  createSubmitting.value = true
  try {
    await apiClient.post('/users', createForm.value)
    showCreateModal.value = false
    await fetchUsers()
  } catch (e: unknown) {
    createError.value = extractError(e, 'Failed to create user')
  } finally {
    createSubmitting.value = false
  }
}

function openEdit(user: User): void {
  editUser.value = user
  editTab.value = 'general'
  editRole.value = user.role
  editRoleError.value = ''
  editPassword.value = ''
  editPasswordError.value = ''
  editPasswordSuccess.value = false
  editRolesError.value = ''
  showEditModal.value = true
  void loadEditRoles(user)
  void loadEditPermissions(user)
}

async function loadEditRoles(user: User): Promise<void> {
  editRolesLoading.value = true
  try {
    const [rolesRes, groupsRes, userRolesRes, userGroupsRes] = await Promise.all([
      apiClient.get<RoleRow[]>('/roles'),
      apiClient.get<GroupRow[]>('/groups'),
      apiClient.get<RoleRow[]>(`/users/${user.id}/roles`),
      apiClient.get<GroupRow[]>(`/users/${user.id}/groups`),
    ])
    allRoles.value = rolesRes.data
    allGroups.value = groupsRes.data
    userRoleIds.value = userRolesRes.data.map((r) => r.id)
    userGroupIds.value = userGroupsRes.data.map((g) => g.id)
  } finally {
    editRolesLoading.value = false
  }
}

async function loadEditPermissions(user: User): Promise<void> {
  editPermsLoading.value = true
  try {
    const [reposRes, permsRes] = await Promise.all([
      apiClient.get<Repo[]>('/repos'),
      apiClient.get<RepoPermission[]>(`/users/${user.id}/permissions`),
    ])
    permissionsRepos.value = reposRes.data
    permissionsData.value = permsRes.data
  } finally {
    editPermsLoading.value = false
  }
}

async function saveRole(): Promise<void> {
  if (!editUser.value) return
  editRoleSubmitting.value = true
  editRoleError.value = ''
  try {
    await apiClient.put(`/users/${editUser.value.id}/role`, { role: editRole.value })
    const idx = users.value.findIndex((u) => u.id === editUser.value!.id)
    if (idx !== -1) {
      users.value[idx] = { ...users.value[idx], role: editRole.value }
    }
    editUser.value = { ...editUser.value, role: editRole.value }
  } catch (e: unknown) {
    editRoleError.value = extractError(e, 'Failed to update role')
  } finally {
    editRoleSubmitting.value = false
  }
}

async function savePassword(): Promise<void> {
  if (!editUser.value) return
  editPasswordSubmitting.value = true
  editPasswordError.value = ''
  editPasswordSuccess.value = false
  try {
    await apiClient.put(`/users/${editUser.value.id}/password`, {
      password: editPassword.value,
    })
    editPassword.value = ''
    editPasswordSuccess.value = true
  } catch (e: unknown) {
    editPasswordError.value = extractError(e, 'Failed to reset password')
  } finally {
    editPasswordSubmitting.value = false
  }
}

function toggleUserRole(roleId: number): void {
  const idx = userRoleIds.value.indexOf(roleId)
  if (idx === -1) {
    userRoleIds.value = [...userRoleIds.value, roleId]
  } else {
    userRoleIds.value = userRoleIds.value.filter((id) => id !== roleId)
  }
}

function toggleUserGroup(groupId: number): void {
  const idx = userGroupIds.value.indexOf(groupId)
  if (idx === -1) {
    userGroupIds.value = [...userGroupIds.value, groupId]
  } else {
    userGroupIds.value = userGroupIds.value.filter((id) => id !== groupId)
  }
}

async function saveRolesGroups(): Promise<void> {
  if (!editUser.value) return
  editRolesSubmitting.value = true
  editRolesError.value = ''
  try {
    await Promise.all([
      apiClient.put(`/users/${editUser.value.id}/roles`, { role_ids: userRoleIds.value }),
      apiClient.put(`/users/${editUser.value.id}/groups`, { group_ids: userGroupIds.value }),
    ])
  } catch (e: unknown) {
    editRolesError.value = extractError(e, 'Failed to save assignments')
  } finally {
    editRolesSubmitting.value = false
  }
}

function getPermission(repoId: number): RepoPermission {
  const existing = permissionsData.value.find((p) => p.repo_id === repoId)
  if (existing) return existing
  return {
    user_id: editUser.value?.id ?? 0,
    repo_id: repoId,
    can_view: false,
    can_backup: false,
    can_modify_schedules: false,
    can_extract: false,
    can_delete: false,
  }
}

async function togglePermission(repoId: number, field: keyof RepoPermission): Promise<void> {
  if (!editUser.value) return
  const perm = getPermission(repoId)
  const updated = { ...perm, [field]: !perm[field] }
  await apiClient.put(`/repos/${repoId}/permissions/${editUser.value.id}`, {
    can_view: updated.can_view,
    can_backup: updated.can_backup,
    can_modify_schedules: updated.can_modify_schedules,
    can_extract: updated.can_extract,
    can_delete: updated.can_delete,
  })
  const idx = permissionsData.value.findIndex((p) => p.repo_id === repoId)
  if (idx !== -1) {
    permissionsData.value[idx] = updated
  } else {
    permissionsData.value.push(updated)
  }
}

function openDelete(user: User): void {
  deleteTarget.value = user
  showDeleteModal.value = true
}

async function confirmDelete(): Promise<void> {
  if (!deleteTarget.value) return
  deleteSubmitting.value = true
  try {
    await apiClient.delete(`/users/${deleteTarget.value.id}`)
    showDeleteModal.value = false
    deleteTarget.value = null
    await fetchUsers()
  } finally {
    deleteSubmitting.value = false
  }
}

function isSelf(user: User): boolean {
  return user.id === authStore.user?.id
}

onMounted(fetchUsers)
</script>

<template>
  <div class="users-page">
    <div class="page-header">
      <h1 class="page-title">Users</h1>
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

    <BaseSpinner
      v-if="loading"
      size="lg"
    />

    <table
      v-else
      class="data-table"
    >
      <thead>
        <tr>
          <th>Username</th>
          <th>Role</th>
          <th class="col-date">Created</th>
          <th class="col-date">Last login</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="user in users"
          :key="user.id"
        >
          <td>
            <span class="user-cell">
              {{ user.username }}
              <span
                v-if="isSelf(user)"
                class="badge badge--accent"
                >you</span
              >
            </span>
          </td>
          <td>
            <span
              class="badge badge--neutral"
              :class="user.role"
              >{{ user.role }}</span
            >
          </td>
          <td class="date-cell col-date">
            {{ formatDate(user.created_at) }}
          </td>
          <td class="date-cell col-date">
            {{ formatDate(user.last_login_at, 'Never') }}
          </td>
          <td>
            <div
              v-if="!isSelf(user)"
              class="actions-cell"
            >
              <button
                class="btn btn-sm btn-ghost"
                @click="openEdit(user)"
              >
                <Pencil :size="14" />
                Edit
              </button>
              <button
                class="btn btn-sm btn-ghost btn-danger-text"
                @click="openDelete(user)"
              >
                <Trash2 :size="14" />
              </button>
            </div>
          </td>
        </tr>
      </tbody>
    </table>

    <!-- Create User Modal -->
    <BaseModal
      :open="showCreateModal"
      title="Add user"
      form
      @close="showCreateModal = false"
      @submit="submitCreate"
    >
      <div class="field">
        <label for="new-username">Username</label>
        <input
          id="new-username"
          v-model="createForm.username"
          class="input"
          type="text"
          required
        />
      </div>
      <div class="field">
        <label for="new-password">Password</label>
        <input
          id="new-password"
          v-model="createForm.password"
          class="input"
          type="password"
          required
          minlength="8"
        />
      </div>
      <div class="field">
        <label for="new-role">Role</label>
        <select
          id="new-role"
          v-model="createForm.role"
          class="input"
        >
          <option value="user">User</option>
          <option value="admin">Admin</option>
        </select>
      </div>
      <div
        v-if="createError"
        class="form-error"
      >
        {{ createError }}
      </div>
      <div class="modal-actions">
        <button
          type="button"
          class="btn btn-ghost"
          @click="showCreateModal = false"
        >
          Cancel
        </button>
        <button
          type="submit"
          class="btn btn-primary"
          :disabled="createSubmitting || !createForm.username.trim() || !createForm.password"
        >
          Create
        </button>
      </div>
    </BaseModal>

    <!-- Edit User Modal -->
    <BaseModal
      :open="showEditModal"
      size="lg"
      @close="showEditModal = false"
    >
      <template #header="{ titleId }">
        <h2
          :id="titleId"
          class="modal-title"
        >
          Edit User — {{ editUser?.username }}
        </h2>
      </template>
      <BaseTabs
        v-model="editTab"
        :tabs="editTabs"
        label="User settings sections"
      />
      <div class="tab-scroll">
        <!-- General Tab -->
        <div
          v-if="editTab === 'general'"
          class="tab-content"
        >
          <div class="field">
            <label for="edit-role">Role</label>
            <select
              id="edit-role"
              v-model="editRole"
              class="input"
            >
              <option value="user">User</option>
              <option value="admin">Admin</option>
            </select>
          </div>
          <div
            v-if="editRoleError"
            class="form-error"
          >
            {{ editRoleError }}
          </div>
          <div class="modal-actions">
            <button
              class="btn btn-primary"
              :disabled="editRoleSubmitting || editRole === editUser?.role"
              @click="saveRole"
            >
              {{ editRoleSubmitting ? 'Saving...' : 'Save Role' }}
            </button>
          </div>
        </div>

        <!-- Password Tab -->
        <div
          v-if="editTab === 'password'"
          class="tab-content"
        >
          <div class="field">
            <label for="edit-password">New password</label>
            <input
              id="edit-password"
              v-model="editPassword"
              class="input"
              type="password"
              minlength="8"
              placeholder="Enter new password"
            />
          </div>
          <div
            v-if="editPasswordError"
            class="form-error"
          >
            {{ editPasswordError }}
          </div>
          <div
            v-if="editPasswordSuccess"
            class="form-success"
          >
            Password updated successfully.
          </div>
          <div class="modal-actions">
            <button
              class="btn btn-primary"
              :disabled="editPasswordSubmitting || !editPassword"
              @click="savePassword"
            >
              {{ editPasswordSubmitting ? 'Resetting...' : 'Reset Password' }}
            </button>
          </div>
        </div>

        <!-- Roles & Groups Tab -->
        <div
          v-if="editTab === 'roles'"
          class="tab-content"
        >
          <BaseSpinner
            v-if="editRolesLoading"
            size="sm"
          />
          <template v-else>
            <div class="rg-section">
              <h3 class="rg-heading">Roles</h3>
              <div
                v-if="allRoles.length === 0"
                class="rg-empty"
              >
                No roles available.
              </div>
              <div
                v-else
                class="rg-list"
              >
                <label
                  v-for="role in allRoles"
                  :key="role.id"
                  class="rg-item"
                >
                  <input
                    type="checkbox"
                    :checked="userRoleIds.includes(role.id)"
                    @change="toggleUserRole(role.id)"
                  />
                  <span class="rg-item-name">{{ role.name }}</span>
                </label>
              </div>
            </div>
            <div
              v-if="allGroups.length > 0"
              class="rg-section"
            >
              <h3 class="rg-heading">Groups</h3>
              <div class="rg-list">
                <label
                  v-for="group in allGroups"
                  :key="group.id"
                  class="rg-item"
                >
                  <input
                    type="checkbox"
                    :checked="userGroupIds.includes(group.id)"
                    @change="toggleUserGroup(group.id)"
                  />
                  <span class="rg-item-name">{{ group.name }}</span>
                </label>
              </div>
            </div>
          </template>
          <div
            v-if="editRolesError"
            class="form-error"
          >
            {{ editRolesError }}
          </div>
          <div class="modal-actions">
            <button
              class="btn btn-primary"
              :disabled="editRolesSubmitting || editRolesLoading"
              @click="saveRolesGroups"
            >
              {{ editRolesSubmitting ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </div>

        <!-- Permissions Tab -->
        <div
          v-if="editTab === 'permissions'"
          class="tab-content"
        >
          <BaseSpinner
            v-if="editPermsLoading"
            size="sm"
          />
          <div
            v-else-if="permissionsRepos.length === 0"
            class="rg-empty"
          >
            No repositories configured yet.
          </div>
          <div
            v-else
            class="permissions-scroll"
          >
            <table class="data-table data-table--compact">
              <thead>
                <tr>
                  <th>Repository</th>
                  <th>View</th>
                  <th>Backup</th>
                  <th>Schedules</th>
                  <th>Extract</th>
                  <th>Delete</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="repo in permissionsRepos"
                  :key="repo.id"
                >
                  <td class="perm-repo-cell">{{ repo.name }}</td>
                  <td class="perm-check-cell">
                    <input
                      type="checkbox"
                      :checked="getPermission(repo.id).can_view"
                      @change="togglePermission(repo.id, 'can_view')"
                    />
                  </td>
                  <td class="perm-check-cell">
                    <input
                      type="checkbox"
                      :checked="getPermission(repo.id).can_backup"
                      @change="togglePermission(repo.id, 'can_backup')"
                    />
                  </td>
                  <td class="perm-check-cell">
                    <input
                      type="checkbox"
                      :checked="getPermission(repo.id).can_modify_schedules"
                      @change="togglePermission(repo.id, 'can_modify_schedules')"
                    />
                  </td>
                  <td class="perm-check-cell">
                    <input
                      type="checkbox"
                      :checked="getPermission(repo.id).can_extract"
                      @change="togglePermission(repo.id, 'can_extract')"
                    />
                  </td>
                  <td class="perm-check-cell">
                    <input
                      type="checkbox"
                      :checked="getPermission(repo.id).can_delete"
                      @change="togglePermission(repo.id, 'can_delete')"
                    />
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </BaseModal>

    <!-- Delete User Modal -->
    <BaseModal
      :open="showDeleteModal"
      title="Delete user"
      @close="showDeleteModal = false"
    >
      <p class="confirm-text">
        Delete <strong>{{ deleteTarget?.username }}</strong
        >? This action cannot be undone.
      </p>
      <div class="modal-actions">
        <button
          class="btn btn-ghost"
          @click="showDeleteModal = false"
        >
          Cancel
        </button>
        <button
          class="btn btn-danger"
          :disabled="deleteSubmitting"
          @click="confirmDelete"
        >
          {{ deleteSubmitting ? 'Deleting...' : 'Delete' }}
        </button>
      </div>
    </BaseModal>
  </div>
</template>

<style scoped>
.users-page {
  max-width: 900px;
}

.loading {
  color: var(--text-muted);
  padding: 2rem 0;
}

.user-cell {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.date-cell {
  color: var(--text-secondary);
  font-size: var(--fs-sm);
}

.col-date {
  @media (max-width: 768px) {
    display: none;
  }
}

.tab-scroll {
  padding: 0.25rem 0 0;
  overflow-y: auto;
  flex: 1;
  min-height: 0;
}

.tab-content {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.field label {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-secondary);
}

.field input,
.field select {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  color: var(--text-primary);
  font-size: var(--fs-base);
}

.field input:focus,
.field select:focus {
  outline: none;
  border-color: var(--accent);
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.5rem;
}

.rg-section {
  margin-bottom: 1rem;
}

.rg-heading {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin: 0 0 0.5rem;
}

.rg-empty {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  padding: 0.5rem 0;
}

.rg-list {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem;
  max-height: 200px;
  overflow-y: auto;
}

.rg-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.4rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-fast);
}

.rg-item:hover {
  background: var(--bg-hover);
}

.rg-item input[type='checkbox'] {
  accent-color: var(--accent);
  cursor: pointer;
}

.rg-item-name {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-primary);
}

.permissions-scroll {
  max-height: 350px;
  overflow-y: auto;
}

.perm-repo-cell {
  font-family: var(--mono);
  font-size: var(--fs-xs);
}

.perm-check-cell {
  text-align: center;
}

.perm-check-cell input[type='checkbox'] {
  accent-color: var(--accent);
  cursor: pointer;
}
</style>

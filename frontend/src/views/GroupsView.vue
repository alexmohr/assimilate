<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useAsyncAction } from '../composables/useAsyncAction'
import { Plus, Trash2 } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'

interface Group {
  id: number
  name: string
  description: string | null
  created_at: string
}

interface UserRow {
  id: number
  username: string
  role: string
}

interface GroupMember {
  user_id: number
}

const groups = ref<Group[]>([])
const allUsers = ref<UserRow[]>([])
const { loading, error, run } = useAsyncAction('Failed to load groups')
loading.value = true

const showCreateModal = ref(false)
const createForm = ref({ name: '', description: '' })
const {
  loading: createSubmitting,
  error: createError,
  run: runCreate,
} = useAsyncAction('Failed to create group')

const showEditModal = ref(false)
const editTarget = ref<Group | null>(null)
const editForm = ref({ name: '', description: '' })
const {
  loading: editSubmitting,
  error: editError,
  run: runEdit,
} = useAsyncAction('Failed to update group')

const showDeleteModal = ref(false)
const deleteTarget = ref<Group | null>(null)
const {
  loading: deleteSubmitting,
  error: deleteError,
  run: runDelete,
} = useAsyncAction('Failed to delete group')

const showMembersModal = ref(false)
const membersTarget = ref<Group | null>(null)
const membersLoading = ref(false)
const memberUserIds = ref<number[]>([])
const membersSubmitting = ref(false)
const membersError = ref<string | null>(null)

const memberCounts = ref<Record<number, number>>({})

const filterText = ref('')

const filteredGroups = computed((): Group[] => {
  if (!filterText.value.trim()) return groups.value
  const q = filterText.value.toLowerCase()
  return groups.value.filter(
    (g) => g.name.toLowerCase().includes(q) || (g.description?.toLowerCase().includes(q) ?? false),
  )
})

async function fetchGroups(): Promise<void> {
  await run(async () => {
    const res = await apiClient.get<Group[]>('/groups')
    groups.value = res.data
    const counts: Record<number, number> = {}
    await Promise.all(
      res.data.map(async (g) => {
        try {
          const membersRes = await apiClient.get<GroupMember[]>(`/groups/${g.id}/members`)
          counts[g.id] = membersRes.data.length
        } catch (e: unknown) {
          logger.error(`fetchGroups: failed to load members for group ${g.id}`, e)
          counts[g.id] = 0
        }
      }),
    )
    memberCounts.value = counts
  })
}

async function fetchUsers(): Promise<void> {
  try {
    const res = await apiClient.get<UserRow[]>('/users')
    allUsers.value = res.data
  } catch (e: unknown) {
    logger.error('fetchUsers failed', e)
    allUsers.value = []
  }
}

function openCreate(): void {
  createForm.value = { name: '', description: '' }
  createError.value = null
  showCreateModal.value = true
}

async function submitCreate(): Promise<void> {
  if (!createForm.value.name.trim()) {
    createError.value = 'Name is required'
    return
  }
  await runCreate(async () => {
    await apiClient.post('/groups', {
      name: createForm.value.name.trim(),
      description: createForm.value.description.trim() || null,
    })
    showCreateModal.value = false
    await fetchGroups()
  })
}

function openEdit(group: Group): void {
  editTarget.value = group
  editForm.value = { name: group.name, description: group.description ?? '' }
  editError.value = null
  showEditModal.value = true
}

async function submitEdit(): Promise<void> {
  const target = editTarget.value
  if (!target) return
  if (!editForm.value.name.trim()) {
    editError.value = 'Name is required'
    return
  }
  await runEdit(async () => {
    await apiClient.put(`/groups/${target.id}`, {
      name: editForm.value.name.trim(),
      description: editForm.value.description.trim() || null,
    })
    showEditModal.value = false
    await fetchGroups()
  })
}

function openDelete(group: Group): void {
  deleteTarget.value = group
  deleteError.value = null
  showDeleteModal.value = true
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value
  if (!target) return
  await runDelete(async () => {
    await apiClient.delete(`/groups/${target.id}`)
    showDeleteModal.value = false
    await fetchGroups()
  })
}

async function openMembers(group: Group): Promise<void> {
  membersTarget.value = group
  membersLoading.value = true
  membersError.value = null
  memberUserIds.value = []
  showMembersModal.value = true
  try {
    const res = await apiClient.get<GroupMember[]>(`/groups/${group.id}/members`)
    memberUserIds.value = res.data.map((m) => m.user_id)
  } catch (e: unknown) {
    membersError.value = extractError(e, 'Failed to load members')
  } finally {
    membersLoading.value = false
  }
}

function toggleMember(userId: number): void {
  const idx = memberUserIds.value.indexOf(userId)
  if (idx === -1) {
    memberUserIds.value = [...memberUserIds.value, userId]
  } else {
    memberUserIds.value = memberUserIds.value.filter((id) => id !== userId)
  }
}

async function saveMembers(): Promise<void> {
  if (!membersTarget.value) return
  membersSubmitting.value = true
  membersError.value = null
  try {
    await apiClient.put(`/groups/${membersTarget.value.id}/members`, {
      user_ids: memberUserIds.value,
    })
    showMembersModal.value = false
    await fetchGroups()
  } catch (e: unknown) {
    membersError.value = extractError(e, 'Failed to save members')
  } finally {
    membersSubmitting.value = false
  }
}

onMounted(async () => {
  await Promise.all([fetchGroups(), fetchUsers()])
})
</script>

<template>
  <div class="groups-page">
    <div class="page-header">
      <h1 class="page-title">Groups</h1>
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
      Groups organize users into collections for shared repository access. Assign per-repository
      permissions to a group and all its members inherit them. Use groups when multiple users need
      identical access to the same set of repositories.
    </p>

    <div class="toolbar">
      <input
        v-model="filterText"
        class="input search-input"
        placeholder="Filter groups..."
      />
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="state-msg state-error"
    >
      {{ error }}
    </div>
    <div
      v-else-if="groups.length === 0"
      class="state-msg"
    >
      No groups created yet.
    </div>
    <div
      v-else-if="filteredGroups.length === 0"
      class="state-msg"
    >
      No groups match the filter.
    </div>

    <table
      v-else
      class="data-table"
    >
      <thead>
        <tr>
          <th>Name</th>
          <th>Description</th>
          <th>Members</th>
          <th>Actions</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="group in filteredGroups"
          :key="group.id"
        >
          <td class="name-cell">{{ group.name }}</td>
          <td class="desc-cell">{{ group.description ?? '\u2014' }}</td>
          <td class="count-cell">{{ memberCounts[group.id] ?? 0 }}</td>
          <td class="actions-cell">
            <button
              class="btn btn-sm btn-ghost"
              @click="openMembers(group)"
            >
              Members
            </button>
            <button
              class="btn btn-sm btn-ghost"
              @click="openEdit(group)"
            >
              Edit
            </button>
            <button
              class="btn btn-sm btn-ghost btn-danger-text"
              title="Delete"
              @click="openDelete(group)"
            >
              <Trash2 :size="14" />
            </button>
          </td>
        </tr>
      </tbody>
    </table>

    <!-- Create Group Modal -->
    <div
      v-if="showCreateModal"
      class="overlay"
      @click.self="showCreateModal = false"
    >
      <div class="dialog">
        <div class="dialog-header">
          <h2 class="dialog-title">Create Group</h2>
          <button
            class="close-btn"
            @click="showCreateModal = false"
          >
            &times;
          </button>
        </div>
        <form @submit.prevent="submitCreate">
          <div class="dialog-body">
            <div class="field">
              <label
                class="field-label"
                for="create-name"
                >Name <span class="required">*</span></label
              >
              <input
                id="create-name"
                v-model="createForm.name"
                type="text"
                class="input"
                required
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="create-desc"
                >Description</label
              >
              <input
                id="create-desc"
                v-model="createForm.description"
                type="text"
                class="input"
                placeholder="Optional description"
              />
            </div>
          </div>
          <ModalFormActions
            :submitting="createSubmitting"
            :disabled="!createForm.name.trim()"
            :error="createError"
            submit-label="Create"
            submitting-label="Creating..."
            @cancel="showCreateModal = false"
          />
        </form>
      </div>
    </div>

    <!-- Edit Group Modal -->
    <div
      v-if="showEditModal"
      class="overlay"
      @click.self="showEditModal = false"
    >
      <div class="dialog">
        <div class="dialog-header">
          <h2 class="dialog-title">Edit Group</h2>
          <button
            class="close-btn"
            @click="showEditModal = false"
          >
            &times;
          </button>
        </div>
        <form @submit.prevent="submitEdit">
          <div class="dialog-body">
            <div class="field">
              <label
                class="field-label"
                for="edit-name"
                >Name <span class="required">*</span></label
              >
              <input
                id="edit-name"
                v-model="editForm.name"
                type="text"
                class="input"
                required
              />
            </div>
            <div class="field">
              <label
                class="field-label"
                for="edit-desc"
                >Description</label
              >
              <input
                id="edit-desc"
                v-model="editForm.description"
                type="text"
                class="input"
                placeholder="Optional description"
              />
            </div>
          </div>
          <ModalFormActions
            :submitting="editSubmitting"
            :disabled="!editForm.name.trim()"
            :error="editError"
            submit-label="Save"
            submitting-label="Saving..."
            @cancel="showEditModal = false"
          />
        </form>
      </div>
    </div>

    <!-- Delete Group Modal -->
    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete Group"
      :submitting="deleteSubmitting"
      :error="deleteError"
      @cancel="showDeleteModal = false"
      @confirm="confirmDelete"
    >
      Are you sure you want to delete <strong>{{ deleteTarget?.name }}</strong
      >? Members will be removed from this group.
    </ConfirmDeleteDialog>

    <!-- Members Modal -->
    <div
      v-if="showMembersModal"
      class="overlay"
      @click.self="showMembersModal = false"
    >
      <div class="dialog dialog-lg">
        <div class="dialog-header">
          <h2 class="dialog-title">Group Members</h2>
          <button
            class="close-btn"
            @click="showMembersModal = false"
          >
            &times;
          </button>
        </div>
        <div class="dialog-body">
          <p class="modal-subtitle">
            Manage members of <strong>{{ membersTarget?.name }}</strong>
          </p>
          <BaseSpinner
            v-if="membersLoading"
            size="sm"
          />
          <div
            v-else-if="allUsers.length === 0"
            class="state-msg"
          >
            No users found.
          </div>
          <div
            v-else
            class="members-list"
          >
            <label
              v-for="user in allUsers"
              :key="user.id"
              class="member-item"
            >
              <input
                type="checkbox"
                :checked="memberUserIds.includes(user.id)"
                @change="toggleMember(user.id)"
              />
              <span class="member-name">{{ user.username }}</span>
              <span class="member-role">{{ user.role }}</span>
            </label>
          </div>
        </div>
        <ModalFormActions
          type="button"
          :submitting="membersSubmitting"
          :error="membersError"
          submit-label="Save Members"
          submitting-label="Saving..."
          @cancel="showMembersModal = false"
          @confirm="saveMembers"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.groups-page {
  max-width: 900px;
}

@media (max-width: 768px) {
  .page-description {
    display: none;
  }
}

.dialog-lg {
  width: 550px;
}

.desc-cell {
  color: var(--text-secondary);
  font-size: 0.8125rem;
}

.count-cell {
  font-weight: 500;
}

.actions-cell {
  display: flex;
  gap: 0.375rem;
}

.modal-subtitle {
  font-size: 0.8125rem;
  color: var(--text-secondary);
  margin: -0.25rem 0 1rem;
}

.members-list {
  max-height: 300px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  margin-bottom: 1rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.5rem;
}

.member-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.1s;
}

.member-item:hover {
  background: var(--bg-hover);
}

.member-item input[type='checkbox'] {
  accent-color: var(--accent);
  cursor: pointer;
}

.member-name {
  font-size: 0.8125rem;
  font-weight: 500;
  color: var(--text-primary);
}

.member-role {
  font-size: 0.6875rem;
  color: var(--text-muted);
  text-transform: uppercase;
  margin-left: auto;
}
</style>

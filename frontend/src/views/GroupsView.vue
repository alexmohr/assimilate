<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import {
  createGroup,
  deleteGroup,
  listGroupMembers,
  listGroups,
  updateGroup,
  updateGroupMembers,
} from '../api/groups'
import type { Group } from '../api/groups'
import { listUsers } from '../api/users'
import type { User } from '../api/users'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import { useAsyncAction } from '../composables/useAsyncAction'
import { Plus, Trash2, Users } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import ModalFormActions from '../components/ModalFormActions.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import BaseModal from '../components/BaseModal.vue'
import EmptyState from '../components/EmptyState.vue'

const groups = ref<Group[]>([])
const allUsers = ref<User[]>([])
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
    const fetchedGroups = await listGroups()
    groups.value = fetchedGroups
    const counts: Record<number, number> = {}
    await Promise.all(
      fetchedGroups.map(async (g) => {
        try {
          const members = await listGroupMembers(g.id)
          counts[g.id] = members.length
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
    allUsers.value = await listUsers()
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
    await createGroup({
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
    await updateGroup(target.id, {
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
    await deleteGroup(target.id)
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
    memberUserIds.value = await listGroupMembers(group.id)
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
    await updateGroupMembers(membersTarget.value.id, {
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
      class="error-banner"
    >
      {{ error }}
    </div>
    <EmptyState
      v-else-if="groups.length === 0"
      :icon="Users"
      title="No groups yet"
      description="Groups let you grant a set of roles to several users at once."
      action="New group"
      @action="openCreate"
    />
    <div
      v-else-if="filteredGroups.length === 0"
      class="state-msg"
    >
      No groups match the filter.
    </div>

    <div
      v-else
      class="table-wrap table-wrap--framed"
    >
      <table class="data-table">
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
    </div>

    <!-- Create Group Modal -->
    <BaseModal
      :open="showCreateModal"
      title="New group"
      form
      @close="showCreateModal = false"
      @submit="submitCreate"
    >
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

    <!-- Edit Group Modal -->
    <BaseModal
      :open="showEditModal"
      title="Edit group"
      form
      @close="showEditModal = false"
      @submit="submitEdit"
    >
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

      <template #footer>
        <ModalFormActions
          :submitting="editSubmitting"
          :disabled="!editForm.name.trim()"
          :error="editError"
          submit-label="Save"
          submitting-label="Saving..."
          @cancel="showEditModal = false"
        />
      </template>
    </BaseModal>

    <!-- Delete Group Modal -->
    <ConfirmDeleteDialog
      :show="showDeleteModal"
      title="Delete group"
      :submitting="deleteSubmitting"
      :error="deleteError"
      @cancel="showDeleteModal = false"
      @confirm="confirmDelete"
    >
      Are you sure you want to delete <strong>{{ deleteTarget?.name }}</strong
      >? Members will be removed from this group.
    </ConfirmDeleteDialog>

    <!-- Members Modal -->
    <BaseModal
      :open="showMembersModal"
      size="lg"
      title="Group members"
      @close="showMembersModal = false"
    >
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

      <template #footer>
        <ModalFormActions
          type="button"
          :submitting="membersSubmitting"
          :error="membersError"
          submit-label="Save Members"
          submitting-label="Saving..."
          @cancel="showMembersModal = false"
          @confirm="saveMembers"
        />
      </template>
    </BaseModal>
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

.desc-cell {
  color: var(--text-secondary);
  font-size: var(--fs-sm);
}

.count-cell {
  font-weight: 500;
}

.modal-subtitle {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  margin: calc(var(--space-2) * -1) 0 var(--space-6);
}

.members-list {
  max-height: 300px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  margin-bottom: var(--space-6);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: var(--space-4);
}

.member-item {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--duration-fast);
}

.member-item:hover {
  background: var(--bg-hover);
}

.member-item input[type='checkbox'] {
  accent-color: var(--accent);
  cursor: pointer;
}

.member-name {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-primary);
}

.member-role {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  text-transform: uppercase;
  margin-left: auto;
}
</style>

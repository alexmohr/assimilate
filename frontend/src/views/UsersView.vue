<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { createUser, listUsers } from '../api/users'
import type { User } from '../api/users'
import { useAuthStore } from '../stores/auth'
import { formatDate, relativeTime } from '../utils/format'
import { extractError } from '../utils/error'
import { badgeClass } from '../utils/badge'
import {
  BASE_ROLES,
  MIN_PASSWORD_LENGTH,
  baseRoleLabel,
  roleBadgeTone,
  type BaseRole,
} from '../utils/userSettings'
import { ChevronRight, Plus, Users } from '@lucide/vue'
import AsyncSection from '../components/AsyncSection.vue'
import EmptyState from '../components/EmptyState.vue'
import BaseModal from '../components/BaseModal.vue'

/**
 * Every account that can sign in, as one panel of rows. A row opens the
 * user's own page, where the role, roles and groups, repository permissions
 * and password are changed and the account is deleted. Only creating one
 * stays a dialog.
 */
const authStore = useAuthStore()
const users = ref<User[]>([])
const loading = ref(true)
const loadError = ref('')

const showCreateModal = ref(false)
const createForm = ref<{ username: string; password: string; role: BaseRole }>({
  username: '',
  password: '',
  role: 'user',
})
const createError = ref('')
const createSubmitting = ref(false)

async function fetchUsers(): Promise<void> {
  loading.value = true
  loadError.value = ''
  try {
    users.value = await listUsers()
  } catch (e: unknown) {
    loadError.value = extractError(e, 'Failed to load users')
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
    await createUser(createForm.value)
    showCreateModal.value = false
    await fetchUsers()
  } catch (e: unknown) {
    createError.value = extractError(e, 'Failed to create user')
  } finally {
    createSubmitting.value = false
  }
}

function isSelf(user: User): boolean {
  return user.id === authStore.user?.id
}

function lastLogin(user: User): string {
  return user.last_login_at ? relativeTime(user.last_login_at) : 'Never'
}

onMounted(fetchUsers)
</script>

<template>
  <div>
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

    <AsyncSection
      :loading="loading"
      :error="loadError"
      :empty="users.length === 0"
    >
      <template #empty>
        <EmptyState
          :icon="Users"
          title="No users yet"
          description="Every person who signs in has an account here."
          action="New user"
          @action="openCreate"
        />
      </template>
      <div class="list-stack users-list">
        <section class="list-group">
          <div class="list-group-header">
            <h2 class="list-group-title">Users</h2>
            <span class="list-group-count">{{ users.length }}</span>
          </div>
          <div
            class="list-cols"
            aria-hidden="true"
          >
            <span>Username</span>
            <span>Role</span>
            <span>Created</span>
            <span>Last login</span>
            <span></span>
          </div>
          <RouterLink
            v-for="user in users"
            :key="user.id"
            :to="`/users/${user.id}`"
            class="list-row"
            :data-user="user.username"
          >
            <div class="list-row-main">
              <div class="list-row-text">
                <span class="list-row-name">
                  {{ user.username }}
                  <span
                    v-if="isSelf(user)"
                    class="badge badge--info"
                    >You</span
                  >
                </span>
                <span
                  v-if="user.must_change_password"
                  class="list-row-sub"
                >
                  Must change password
                </span>
              </div>
            </div>
            <span class="list-row-chips">
              <span
                class="badge"
                :class="badgeClass(roleBadgeTone(user.role))"
                >{{ user.role }}</span
              >
            </span>
            <span class="list-row-value list-wide-only user-date">
              {{ formatDate(user.created_at) }}
            </span>
            <span class="list-row-value user-date">
              <span class="list-narrow-only">Last login </span>{{ lastLogin(user) }}
            </span>
            <ChevronRight
              :size="16"
              class="list-row-chevron"
            />
          </RouterLink>
        </section>
      </div>
    </AsyncSection>

    <BaseModal
      :open="showCreateModal"
      title="New user"
      form
      @close="showCreateModal = false"
      @submit="submitCreate"
    >
      <div class="field">
        <label
          class="field-label"
          for="new-username"
          >Username</label
        >
        <input
          id="new-username"
          v-model="createForm.username"
          class="input"
          type="text"
          required
        />
      </div>
      <div class="field">
        <label
          class="field-label"
          for="new-password"
          >Password</label
        >
        <input
          id="new-password"
          v-model="createForm.password"
          class="input"
          type="password"
          required
          :minlength="MIN_PASSWORD_LENGTH"
        />
      </div>
      <div class="field">
        <label
          class="field-label"
          for="new-role"
          >Role</label
        >
        <select
          id="new-role"
          v-model="createForm.role"
          class="input"
        >
          <option
            v-for="r in BASE_ROLES"
            :key="r"
            :value="r"
          >
            {{ baseRoleLabel(r) }}
          </option>
        </select>
      </div>
      <div
        v-if="createError"
        class="form-error"
      >
        {{ createError }}
      </div>
      <template #footer>
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
          {{ createSubmitting ? 'Creating...' : 'Create' }}
        </button>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.users-list .list-group {
  --list-cols: minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1.4fr) minmax(0, 1fr) 2.5rem;
}

.user-date {
  color: var(--text-secondary);
}
</style>

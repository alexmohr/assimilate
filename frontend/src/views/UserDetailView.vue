<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Check } from '@lucide/vue'
import {
  changeUserPassword,
  deleteUser,
  getUserGroups,
  getUserPermissions,
  getUserRoles,
  listUsers,
  updateUserGroups,
  updateUserRole,
  updateUserRoles,
  type RepoPermission,
  type User,
} from '../api/users'
import { listRoles, type Role } from '../api/roles'
import { listGroups, type Group } from '../api/groups'
import { listRepos, updateRepoPermission } from '../api/repos'
import type { Repo } from '../types/repo'
import { useAsyncAction } from '../composables/useAsyncAction'
import { useToast } from '../composables/useToast'
import { useAuthStore } from '../stores/auth'
import { badgeClass } from '../utils/badge'
import { extractError } from '../utils/error'
import { formatDate } from '../utils/format'
import {
  BASE_ROLES,
  MIN_PASSWORD_LENGTH,
  baseRoleLabel,
  baseRoleOf,
  isUserSection,
  roleBadgeTone,
  type BaseRole,
  type UserSection,
} from '../utils/userSettings'
import BaseSpinner from '../components/BaseSpinner.vue'
import ConfirmDeleteDialog from '../components/ConfirmDeleteDialog.vue'
import DetailHeader from '../components/DetailHeader.vue'
import EditableSection from '../components/EditableSection.vue'
import OverflowMenu from '../components/OverflowMenu.vue'
import PaneRow from '../components/PaneRow.vue'
import SettingsRail, { type SettingsSections } from '../components/SettingsRail.vue'

/**
 * One user account: its base role, the RBAC roles and groups it belongs to,
 * what it may do on each repository, and a password reset. Admin only, like
 * the list it is opened from. Your own account is shown read-only - you change
 * your own password on your profile, and cannot demote or delete yourself.
 */
const props = defineProps<{ id: string }>()
const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()
const toast = useToast()

const userId = computed(() => Number(props.id))
const user = ref<User | null>(null)
const { loading, error, run } = useAsyncAction()

const isSelf = computed(() => user.value !== null && user.value.id === authStore.user?.id)
const canEdit = computed(() => !isSelf.value)

const section = computed<UserSection>({
  get() {
    const s = route.query.section
    return isUserSection(s) ? s : 'role'
  },
  set(val: UserSection) {
    void router.replace({ query: { ...route.query, section: val } })
  },
})

/** Deleting the account lives in the header's overflow menu. */
const sections: SettingsSections<UserSection> = [
  { id: 'role', label: 'Role' },
  { id: 'access', label: 'Roles & groups' },
  { id: 'permissions', label: 'Repository permissions' },
  { id: 'password', label: 'Password' },
]

const baseRole = computed((): BaseRole => baseRoleOf(user.value?.role ?? ''))

// --- loading -------------------------------------------------------------------------------

const allRoles = ref<Role[]>([])
const allGroups = ref<Group[]>([])
const userRoleIds = ref<number[]>([])
const userGroupIds = ref<number[]>([])
const repos = ref<Repo[]>([])
const permissions = ref<RepoPermission[]>([])

async function fetchAll(): Promise<void> {
  const id = userId.value
  const [users, roles, groups, heldRoles, heldGroups, repoList, perms] = await Promise.all([
    listUsers(),
    listRoles(),
    listGroups(),
    getUserRoles(id),
    getUserGroups(id),
    listRepos(),
    getUserPermissions(id),
  ])
  if (userId.value !== id) return
  const found = users.find((u) => u.id === id)
  if (!found) throw new Error('User not found')
  user.value = found
  allRoles.value = roles
  allGroups.value = groups
  userRoleIds.value = heldRoles.map((r) => r.id)
  userGroupIds.value = heldGroups.map((g) => g.id)
  repos.value = repoList
  permissions.value = perms
}

async function load(): Promise<void> {
  await run(fetchAll)
}

watch(
  () => props.id,
  async () => {
    user.value = null
    await load()
  },
)
onMounted(load)

// --- role ----------------------------------------------------------------------------------

const roleEditing = ref(false)
const roleSaving = ref(false)
const roleError = ref<string | null>(null)
const roleDraft = ref<BaseRole>('user')

function startRoleEdit(): void {
  roleDraft.value = baseRole.value
  roleError.value = null
  roleEditing.value = true
}

async function saveRole(): Promise<void> {
  if (!user.value) return
  if (roleDraft.value === baseRole.value) {
    roleEditing.value = false
    return
  }
  roleSaving.value = true
  roleError.value = null
  try {
    await updateUserRole(user.value.id, { role: roleDraft.value })
    user.value = { ...user.value, role: roleDraft.value }
    roleEditing.value = false
    toast.success(`${user.value.username} is now ${baseRoleLabel(roleDraft.value).toLowerCase()}`)
  } catch (e: unknown) {
    roleError.value = extractError(e, 'Failed to update role')
  } finally {
    roleSaving.value = false
  }
}

// --- roles & groups ------------------------------------------------------------------------

const accessEditing = ref(false)
const accessSaving = ref(false)
const accessError = ref<string | null>(null)
const roleIdsDraft = ref<number[]>([])
const groupIdsDraft = ref<number[]>([])

const heldRoleNames = computed((): string[] =>
  allRoles.value.filter((r) => userRoleIds.value.includes(r.id)).map((r) => r.name),
)
const heldGroupNames = computed((): string[] =>
  allGroups.value.filter((g) => userGroupIds.value.includes(g.id)).map((g) => g.name),
)

function startAccessEdit(): void {
  roleIdsDraft.value = [...userRoleIds.value]
  groupIdsDraft.value = [...userGroupIds.value]
  accessError.value = null
  accessEditing.value = true
}

function toggleId(list: number[], id: number): number[] {
  return list.includes(id) ? list.filter((x) => x !== id) : [...list, id]
}

async function saveAccess(): Promise<void> {
  if (!user.value) return
  accessSaving.value = true
  accessError.value = null
  try {
    await Promise.all([
      updateUserRoles(user.value.id, { role_ids: roleIdsDraft.value }),
      updateUserGroups(user.value.id, { group_ids: groupIdsDraft.value }),
    ])
    userRoleIds.value = [...roleIdsDraft.value]
    userGroupIds.value = [...groupIdsDraft.value]
    accessEditing.value = false
    toast.success('Roles and groups saved')
  } catch (e: unknown) {
    accessError.value = extractError(e, 'Failed to save assignments')
  } finally {
    accessSaving.value = false
  }
}

// --- repository permissions ----------------------------------------------------------------

type PermissionKey = Exclude<keyof RepoPermission, 'user_id' | 'repo_id'>

const PERMISSION_COLUMNS: readonly { key: PermissionKey; label: string }[] = [
  { key: 'can_view', label: 'View' },
  { key: 'can_backup', label: 'Backup' },
  { key: 'can_modify_schedules', label: 'Schedules' },
  { key: 'can_extract', label: 'Extract' },
  { key: 'can_delete', label: 'Delete' },
]

const permsEditing = ref(false)
const permsSaving = ref(false)
const permsError = ref<string | null>(null)
const permsDraft = ref<RepoPermission[]>([])

function emptyPermission(repoId: number): RepoPermission {
  return {
    user_id: userId.value,
    repo_id: repoId,
    can_view: false,
    can_backup: false,
    can_modify_schedules: false,
    can_extract: false,
    can_delete: false,
  }
}

function permissionIn(list: readonly RepoPermission[], repoId: number): RepoPermission {
  return list.find((p) => p.repo_id === repoId) ?? emptyPermission(repoId)
}

function samePermission(a: RepoPermission, b: RepoPermission): boolean {
  return PERMISSION_COLUMNS.every((c) => a[c.key] === b[c.key])
}

function startPermsEdit(): void {
  permsDraft.value = repos.value.map((r) => ({ ...permissionIn(permissions.value, r.id) }))
  permsError.value = null
  permsEditing.value = true
}

function toggleDraftPermission(repoId: number, key: PermissionKey): void {
  permsDraft.value = permsDraft.value.map((p) =>
    p.repo_id === repoId ? { ...p, [key]: !p[key] } : p,
  )
}

async function savePerms(): Promise<void> {
  if (!user.value) return
  const id = user.value.id
  const changed = permsDraft.value.filter(
    (p) => !samePermission(p, permissionIn(permissions.value, p.repo_id)),
  )
  permsSaving.value = true
  permsError.value = null
  try {
    await Promise.all(
      changed.map((p) =>
        updateRepoPermission(p.repo_id, id, {
          can_view: p.can_view,
          can_backup: p.can_backup,
          can_modify_schedules: p.can_modify_schedules,
          can_extract: p.can_extract,
          can_delete: p.can_delete,
        }),
      ),
    )
    permissions.value = permsDraft.value.map((p) => ({ ...p }))
    permsEditing.value = false
    if (changed.length > 0) toast.success('Permissions saved')
  } catch (e: unknown) {
    permsError.value = extractError(e, 'Failed to save permissions')
  } finally {
    permsSaving.value = false
  }
}

// --- password ------------------------------------------------------------------------------

const pwEditing = ref(false)
const pwSaving = ref(false)
const pwError = ref<string | null>(null)
const pwDraft = ref('')

function startPwEdit(): void {
  pwDraft.value = ''
  pwError.value = null
  pwEditing.value = true
}

async function savePassword(): Promise<void> {
  if (!user.value) return
  if (pwDraft.value.length < MIN_PASSWORD_LENGTH) {
    pwError.value = `The password needs at least ${MIN_PASSWORD_LENGTH} characters.`
    return
  }
  pwSaving.value = true
  pwError.value = null
  try {
    await changeUserPassword(user.value.id, { password: pwDraft.value })
    pwDraft.value = ''
    pwEditing.value = false
    toast.success(`Password for ${user.value.username} reset`)
  } catch (e: unknown) {
    pwError.value = extractError(e, 'Failed to reset password')
  } finally {
    pwSaving.value = false
  }
}

// --- delete --------------------------------------------------------------------------------

const showDelete = ref(false)
const deleting = ref(false)
const deleteError = ref<string | null>(null)

async function confirmDelete(): Promise<void> {
  if (!user.value) return
  deleting.value = true
  deleteError.value = null
  try {
    await deleteUser(user.value.id)
    toast.success(`Deleted ${user.value.username}`)
    showDelete.value = false
    await router.push('/users')
  } catch (e: unknown) {
    deleteError.value = extractError(e, 'Failed to delete user')
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div class="user-detail">
    <nav class="detail-breadcrumb">
      <RouterLink
        to="/users"
        class="crumb-link"
      >
        Users
      </RouterLink>
      <span class="crumb-sep">/</span>
      <span class="crumb-current mono">{{ user?.username ?? '...' }}</span>
    </nav>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error && !user"
      class="error-banner"
    >
      {{ error }}
    </div>

    <template v-else-if="user">
      <DetailHeader
        :name="user.username"
        mono
      >
        <template #badges>
          <span
            class="badge"
            :class="badgeClass(roleBadgeTone(user.role))"
          >
            {{ user.role }}
          </span>
          <span
            v-if="isSelf"
            class="badge badge--info"
          >
            You
          </span>
          <span
            v-if="user.must_change_password"
            class="badge badge--warning"
          >
            Must change password
          </span>
        </template>
        <template #meta>
          <span>Created {{ formatDate(user.created_at) }}</span>
          <span>Last login {{ formatDate(user.last_login_at, 'never') }}</span>
        </template>
        <template
          v-if="canEdit"
          #actions
        >
          <OverflowMenu
            v-slot="{ run: runAction }"
            label="More user actions"
          >
            <button
              class="overflow-menu-item overflow-menu-item--danger"
              role="menuitem"
              type="button"
              @click="runAction(() => (showDelete = true))"
            >
              Delete user
            </button>
          </OverflowMenu>
        </template>
      </DetailHeader>

      <p
        v-if="isSelf"
        class="field-hint user-self-hint"
      >
        This is your own account. Change your password on your
        <RouterLink to="/profile">profile</RouterLink>; another admin changes your role.
      </p>

      <div class="tab-content fade-in">
        <SettingsRail
          v-slot="{ section: current }"
          :sections="sections"
          :section="section"
          label="User sections"
          @update:section="section = $event"
        >
          <EditableSection
            v-if="current === 'role'"
            lede="Admins manage everything, including other users. Every other user sees only what their roles, groups and repository permissions allow."
            :editing="roleEditing"
            :can-edit="canEdit"
            :saving="roleSaving"
            :error="roleError"
            @edit="startRoleEdit"
            @cancel="roleEditing = false"
            @save="saveRole"
          >
            <template #view>
              <dl class="info-grid">
                <dt>Role</dt>
                <dd>{{ baseRoleLabel(baseRole) }}</dd>
              </dl>
            </template>
            <template #edit>
              <div class="pane-rows">
                <PaneRow
                  title="Role"
                  label-for="user-edit-role"
                >
                  <select
                    id="user-edit-role"
                    v-model="roleDraft"
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
                </PaneRow>
              </div>
            </template>
          </EditableSection>

          <EditableSection
            v-else-if="current === 'access'"
            lede="Roles grant permissions on every repository; groups bundle users so a role or permission can be given to all of them at once."
            :editing="accessEditing"
            :can-edit="canEdit"
            :saving="accessSaving"
            :error="accessError"
            @edit="startAccessEdit"
            @cancel="accessEditing = false"
            @save="saveAccess"
          >
            <template #view>
              <dl class="info-grid">
                <dt>Roles</dt>
                <dd :class="{ muted: heldRoleNames.length === 0 }">
                  {{ heldRoleNames.length > 0 ? heldRoleNames.join(', ') : 'None' }}
                </dd>
                <dt>Groups</dt>
                <dd :class="{ muted: heldGroupNames.length === 0 }">
                  {{ heldGroupNames.length > 0 ? heldGroupNames.join(', ') : 'None' }}
                </dd>
              </dl>
            </template>
            <template #edit>
              <div class="pane-rows">
                <PaneRow
                  title="Roles"
                  stack
                >
                  <p
                    v-if="allRoles.length === 0"
                    class="field-hint"
                  >
                    No roles exist yet. Create one under Roles.
                  </p>
                  <div
                    v-else
                    class="user-choice-list"
                  >
                    <label
                      v-for="role in allRoles"
                      :key="role.id"
                      class="user-choice"
                    >
                      <input
                        type="checkbox"
                        :checked="roleIdsDraft.includes(role.id)"
                        @change="roleIdsDraft = toggleId(roleIdsDraft, role.id)"
                      />
                      <span>{{ role.name }}</span>
                    </label>
                  </div>
                </PaneRow>
                <PaneRow
                  title="Groups"
                  stack
                >
                  <p
                    v-if="allGroups.length === 0"
                    class="field-hint"
                  >
                    No groups exist yet. Create one under Groups.
                  </p>
                  <div
                    v-else
                    class="user-choice-list"
                  >
                    <label
                      v-for="group in allGroups"
                      :key="group.id"
                      class="user-choice"
                    >
                      <input
                        type="checkbox"
                        :checked="groupIdsDraft.includes(group.id)"
                        @change="groupIdsDraft = toggleId(groupIdsDraft, group.id)"
                      />
                      <span>{{ group.name }}</span>
                    </label>
                  </div>
                </PaneRow>
              </div>
            </template>
          </EditableSection>

          <EditableSection
            v-else-if="current === 'permissions'"
            lede="What this user may do on each repository, on top of what their roles and groups already allow."
            :editing="permsEditing"
            :can-edit="canEdit && repos.length > 0"
            :saving="permsSaving"
            :error="permsError"
            @edit="startPermsEdit"
            @cancel="permsEditing = false"
            @save="savePerms"
          >
            <template #view>
              <p
                v-if="repos.length === 0"
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
                        class="user-perm-cell"
                      >
                        {{ c.label }}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr
                      v-for="repo in repos"
                      :key="repo.id"
                    >
                      <td class="cell-mono">{{ repo.name }}</td>
                      <td
                        v-for="c in PERMISSION_COLUMNS"
                        :key="c.key"
                        class="user-perm-cell"
                      >
                        <Check
                          v-if="permissionIn(permissions, repo.id)[c.key]"
                          :size="14"
                          class="user-perm-granted"
                          :aria-label="`${c.label} allowed`"
                        />
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </template>
            <template #edit>
              <div class="table-wrap">
                <table class="data-table data-table--compact">
                  <thead>
                    <tr>
                      <th>Repository</th>
                      <th
                        v-for="c in PERMISSION_COLUMNS"
                        :key="c.key"
                        class="user-perm-cell"
                      >
                        {{ c.label }}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr
                      v-for="p in permsDraft"
                      :key="p.repo_id"
                    >
                      <td class="cell-mono">
                        {{ repos.find((r) => r.id === p.repo_id)?.name ?? p.repo_id }}
                      </td>
                      <td
                        v-for="c in PERMISSION_COLUMNS"
                        :key="c.key"
                        class="user-perm-cell"
                      >
                        <input
                          type="checkbox"
                          :checked="p[c.key]"
                          :aria-label="`${c.label} on ${repos.find((r) => r.id === p.repo_id)?.name ?? p.repo_id}`"
                          @change="toggleDraftPermission(p.repo_id, c.key)"
                        />
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </template>
          </EditableSection>

          <EditableSection
            v-else-if="current === 'password'"
            lede="Sets a new password for this account. Assimilate does not send it anywhere, so pass it on yourself."
            :editing="pwEditing"
            :can-edit="canEdit"
            :saving="pwSaving"
            :error="pwError"
            @edit="startPwEdit"
            @cancel="pwEditing = false"
            @save="savePassword"
          >
            <template #view>
              <dl class="info-grid">
                <dt>Password</dt>
                <dd>Set</dd>
                <dt>At next sign-in</dt>
                <dd>
                  {{ user.must_change_password ? 'Must choose a new one' : 'No change required' }}
                </dd>
              </dl>
            </template>
            <template #edit>
              <div class="pane-rows">
                <PaneRow
                  title="New password"
                  label-for="user-edit-password"
                  :hint="`At least ${MIN_PASSWORD_LENGTH} characters.`"
                >
                  <input
                    id="user-edit-password"
                    v-model="pwDraft"
                    class="input"
                    type="password"
                    autocomplete="new-password"
                    :minlength="MIN_PASSWORD_LENGTH"
                  />
                </PaneRow>
              </div>
            </template>
          </EditableSection>
        </SettingsRail>
      </div>

      <ConfirmDeleteDialog
        :show="showDelete"
        title="Delete user"
        :submitting="deleting"
        :error="deleteError"
        confirm-label="Delete"
        submitting-label="Deleting..."
        @cancel="showDelete = false"
        @confirm="confirmDelete"
      >
        Delete <span class="mono">{{ user.username }}</span
        >? They can no longer sign in, and this cannot be undone.
      </ConfirmDeleteDialog>
    </template>
  </div>
</template>

<style scoped>
.user-detail {
  max-width: 1200px;
}

.user-self-hint {
  margin-bottom: var(--space-6);
}

.user-choice-list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3) var(--space-7);
}

.user-choice {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.user-perm-cell {
  text-align: center;
}

.user-perm-granted {
  color: var(--success);
}
</style>

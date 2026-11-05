<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { Info, Lock } from '@lucide/vue'
import {
  checkAppendOnly,
  getAppendOnlyStatus,
  reapplyAppendOnly,
  setAppendOnly,
} from '../api/appendOnly'
import { extractError } from '../utils/error'
import { useToast } from '../composables/useToast'
import { useWebSocket } from '../composables/useWebSocket'
import { logger } from '../utils/logger'
import HelpHint from './HelpHint.vue'
import PaneRow from './PaneRow.vue'
import RepoAppendOnlyCheck from './RepoAppendOnlyCheck.vue'
import RepoAppendOnlyKeys from './RepoAppendOnlyKeys.vue'
import RepoAppendOnlyPassword from './RepoAppendOnlyPassword.vue'
import RepoMaintenancePanel from './RepoMaintenancePanel.vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { ActiveRepoOp, RepoWithStats } from '../types/repo'
import type { AppendOnlyStatusResponse } from '../types/generated'

/**
 * Append-only for a repository: enforced by the host, through the keys the
 * server installs in its `authorized_keys`. Agents get a key that may add
 * archives but not delete them; only the server holds the key that can.
 *
 * It covers the whole SSH login (host and user), not one path, so every
 * repository sharing the login switches together.
 */
const props = defineProps<{
  repo: RepoWithStats
  currentOp: ActiveRepoOp | null
}>()

const emit = defineEmits<{ changed: [] }>()

/** What a one-time password is retried for. */
type SetupAction = 'enable' | 'disable' | 'reapply'

const { success: toastSuccess } = useToast()
const { onMessage } = useWebSocket()

const status = ref<AppendOnlyStatusResponse | null>(null)
const loading = ref(false)
const loadError = ref<string | null>(null)
const busy = ref(false)
const checking = ref(false)
const actionError = ref<string | null>(null)
const retryAction = ref<SetupAction>('enable')

const maintenanceRunning = computed<boolean>(() => props.currentOp?.kind === 'maintenance')

const needsPassword = computed<boolean>(() => status.value?.setup_failure?.needs_password === true)

// A reload that was started before an action finished may resolve after it
// and would put the state from before the action back, so an action's result
// invalidates every reload still in flight.
let generation = 0

async function load(silent: boolean): Promise<void> {
  const mine = ++generation
  if (!silent) loading.value = true
  loadError.value = null
  try {
    const loaded = await getAppendOnlyStatus(props.repo.id)
    if (mine === generation) status.value = loaded
  } catch (e: unknown) {
    if (!silent) loadError.value = extractError(e)
    else logger.error(e)
  } finally {
    loading.value = false
  }
}

async function perform(action: SetupAction, password: string | null): Promise<void> {
  busy.value = true
  actionError.value = null
  retryAction.value = action
  try {
    const next =
      action === 'reapply'
        ? await reapplyAppendOnly(props.repo.id, { password })
        : await setAppendOnly(props.repo.id, { enabled: action === 'enable', password })
    status.value = next
    generation++
    if (next.setup_failure === null) {
      toastSuccess(
        action === 'reapply'
          ? 'Keys written again.'
          : action === 'enable'
            ? 'Append-only is on.'
            : 'Append-only is off.',
      )
    }
    // A failed setup changed nothing on the host, so the parent has nothing to refetch.
    if (next.setup_failure === null) emit('changed')
  } catch (e: unknown) {
    // A 409 lands here too: turning it off while deletions are queued.
    actionError.value = extractError(e)
  } finally {
    busy.value = false
  }
}

function toggle(enabled: boolean): Promise<void> {
  return perform(enabled ? 'enable' : 'disable', null)
}

async function runCheck(): Promise<void> {
  checking.value = true
  actionError.value = null
  try {
    const checked = await checkAppendOnly(props.repo.id)
    status.value = checked
    generation++
  } catch (e: unknown) {
    actionError.value = extractError(e)
  } finally {
    checking.value = false
  }
}

onMounted(() => load(false))
watch(
  () => props.repo.id,
  () => load(false),
)

// The maintenance run is a background job and queued deletions arrive from
// other tabs, so the pane follows the server's events rather than polling.
onMessage('DataChanged', () => {
  void load(true)
})
onMessage('RepoOpChanged', (payload) => {
  if (payload.repo_id === props.repo.id) void load(true)
})
</script>

<template>
  <div class="panel-stack">
    <div class="pane-head">
      <p class="pane-lede">
        Agents can add archives but cannot delete or overwrite them. Only the Assimilate server
        deletes, during maintenance, with a separate full-access key. A compromised backup machine
        cannot destroy its own history.
      </p>
    </div>

    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading append-only status...
    </div>
    <div
      v-else-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </div>

    <template v-else-if="status">
      <div class="pane-rows">
        <PaneRow
          title="Append-only"
          :hint="`For every repository ${status.login.ssh_user}@${status.login.ssh_host} holds`"
        >
          <ToggleSwitch
            :model-value="status.enabled"
            :disabled="busy"
            label="Append-only"
            @update:model-value="toggle"
          />
        </PaneRow>
      </div>
      <div
        v-if="actionError"
        class="form-error"
        role="alert"
      >
        {{ actionError }}
      </div>

      <div class="callout callout--info">
        <Info :size="16" />
        <p>
          sshd gives each key one command per login, so append-only covers the whole login, not one
          path.
          <template v-if="status.login.sibling_repos.length > 0">
            These repositories share
            <span class="mono">{{ status.login.ssh_user }}@{{ status.login.ssh_host }}</span>
            and switch with this one:
            <template
              v-for="(sibling, index) in status.login.sibling_repos"
              :key="sibling.id"
            >
              <RouterLink
                class="mono"
                :to="`/repos/${sibling.id}`"
                >{{ sibling.name }}</RouterLink
              ><template v-if="index < status.login.sibling_repos.length - 1">, </template
              ><template v-else>.</template>
            </template>
          </template>
          To keep a repository deletable by agents, give it its own SSH user.
        </p>
      </div>

      <RepoAppendOnlyPassword
        v-if="status.setup_failure && needsPassword"
        :failure="status.setup_failure"
        :login="status.login"
        :busy="busy"
        @submit="perform(retryAction, $event)"
      />
      <div
        v-else-if="status.setup_failure"
        class="error-banner"
        role="alert"
      >
        {{ status.setup_failure.message }}
      </div>

      <template v-if="status.enabled">
        <div class="group-label">Keys on {{ status.login.ssh_host }}</div>
        <RepoAppendOnlyKeys
          :status="status"
          :busy="busy"
          @reapply="perform('reapply', null)"
        />
        <RepoAppendOnlyCheck
          :check="status.check"
          :busy="checking"
          @check="runCheck"
        />

        <div class="panel">
          <h3 class="panel-title">
            What changes for this repository
            <HelpHint label="what append-only changes">
              Space is freed only when maintenance runs, so the quota counts queued deletions as
              used.
            </HelpHint>
          </h3>
          <ul class="effects">
            <li>
              <Lock :size="14" />
              Backups no longer prune or compact. Retention runs during maintenance instead.
            </li>
            <li>
              <Lock :size="14" />
              Deleting an archive in the UI queues it. It disappears at the next maintenance.
            </li>
          </ul>
        </div>

        <div class="group-label">Maintenance</div>
        <RepoMaintenancePanel
          v-if="status.maintenance"
          :repo-id="repo.id"
          :status="status"
          :maintenance="status.maintenance"
          :running="maintenanceRunning"
          @status="status = $event"
          @changed="emit('changed')"
        />
      </template>
    </template>
  </div>
</template>

<style scoped>
.effects {
  list-style: none;
  margin: var(--space-4) 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.effects li {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.effects svg {
  color: var(--violet);
  flex: none;
}
</style>

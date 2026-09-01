<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatDate, relativeTime } from '../utils/format'
import { badgeClass, type AgentPowerPhase } from '../utils/badge'
import DetailHeader from './DetailHeader.vue'
import OverflowMenu from './OverflowMenu.vue'
import type { AgentRow } from '../types/agent'

/**
 * The agent detail page's identity block, shown above the tab strip and so
 * present on every tab. Actions are graded rather than listed: one accented
 * slot for the thing that is actionable right now, everything else - including
 * navigation like Activity log - behind the overflow menu. Before this the
 * same eight controls rendered as eight identical ghost buttons in one row,
 * with Activity Log indistinguishable from Restart Agent.
 *
 * The block itself is `DetailHeader`, shared with the schedule and repository
 * pages; this file is only what an agent puts in it.
 */
const props = defineProps<{
  agent: AgentRow
  /**
   * The transient wake/start/shutdown phase to show instead of the usual
   * Online/Offline badge, derived from this run's live event stream. `null`
   * outside of a wake-enabled run, which is when this is unchanged from
   * today.
   */
  powerPhase: AgentPowerPhase | null
  /** Non-null when a newer agent build is available and may be deployed. */
  deployLabel: string | null
  /** True once the agent has been deployed at least once and the caller may redeploy it. */
  canRedeploy: boolean
  restartLoading: boolean
  regenLoading: boolean
  restartError: string | null
  /** Only admins may bulk-delete failed report history, matching the other danger-zone actions. */
  isAdmin: boolean
  /** How many of this agent's backup runs currently show as failed. */
  failedReportCount: number
}>()

const emit = defineEmits<{
  adopt: []
  merge: []
  deploy: []
  redeploy: []
  activityLog: []
  editIdentity: []
  deploySshKey: []
  regenerateToken: []
  restart: []
  cleanFailedReports: []
}>()

const isImported = computed(() => props.agent.is_imported)
const isOnline = computed(() => props.agent.is_connected ?? false)

/**
 * Restart is offered only where it can actually work. The agent reports
 * whether its supervisor supports it; an offline agent cannot be reached at
 * all, and an imported host has no agent to reach.
 */
const canRestart = computed(
  () => props.agent.supports_restart && !isImported.value && isOnline.value,
)
</script>

<template>
  <DetailHeader
    :name="agent.hostname"
    mono
    mono-meta
    :subtitle="agent.display_name"
  >
    <template #badges>
      <span
        v-if="isImported"
        class="badge badge--neutral"
        >Imported</span
      >
      <span
        v-else-if="powerPhase"
        class="badge badge--pulse"
        :class="badgeClass(powerPhase.tone)"
      >
        <span class="badge-dot" />
        {{ powerPhase.label }}
      </span>
      <span
        v-else
        class="badge"
        :class="isOnline ? 'badge--success' : 'badge--neutral'"
      >
        <span class="badge-dot" />
        {{ isOnline ? 'Online' : 'Offline' }}
      </span>
      <span
        v-if="deployLabel === 'Upgrade'"
        class="badge badge--info"
        >Upgrade available</span
      >
    </template>

    <!--
      An imported host has no agent binary, so version, revision and build
      time do not exist for it. Rendering them as em dashes would imply the
      agent is there but silent, which is the opposite of the truth.
    -->
    <template #meta>
      <template v-if="!isImported">
        <span>
          agent <b>{{ agent.agent_version ?? 'unknown' }}</b>
        </span>
        <span v-if="agent.agent_git_sha">
          rev <b>{{ agent.agent_git_sha }}</b>
        </span>
        <span v-if="agent.agent_build_time">
          built <b>{{ agent.agent_build_time }}</b>
        </span>
      </template>
      <span>
        added <b>{{ formatDate(agent.created_at ?? null, 'unknown') }}</b>
      </span>
      <span v-if="agent.last_seen_at">
        seen <b>{{ relativeTime(agent.last_seen_at) }}</b>
      </span>
    </template>

    <template #actions>
      <!--
        Imported hosts get the adoption pair in the primary slot: an imported
        host has exactly one job, which is to stop being one.
      -->
      <template v-if="isImported">
        <button
          class="btn btn-sm btn-primary"
          @click="emit('adopt')"
        >
          Adopt
        </button>
        <button
          class="btn btn-sm"
          @click="emit('merge')"
        >
          Merge into...
        </button>
      </template>
      <template v-else>
        <button
          v-if="deployLabel"
          class="btn btn-sm btn-primary"
          @click="emit('deploy')"
        >
          {{ deployLabel }} agent
        </button>
      </template>

      <OverflowMenu
        v-slot="{ run }"
        label="More agent actions"
      >
        <button
          class="overflow-menu-item"
          role="menuitem"
          type="button"
          @click="run(() => emit('activityLog'))"
        >
          Activity log
        </button>
        <button
          v-if="isAdmin && failedReportCount > 0"
          class="overflow-menu-item overflow-menu-item--danger"
          role="menuitem"
          type="button"
          @click="run(() => emit('cleanFailedReports'))"
        >
          Clean up failed backups ({{ failedReportCount }})
        </button>
        <template v-if="!isImported">
          <button
            class="overflow-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => emit('editIdentity'))"
          >
            Edit identity
          </button>
          <button
            class="overflow-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => emit('deploySshKey'))"
          >
            Deploy SSH key
          </button>
          <button
            v-if="canRedeploy"
            class="overflow-menu-item"
            role="menuitem"
            type="button"
            @click="run(() => emit('redeploy'))"
          >
            Redeploy agent
          </button>
          <button
            class="overflow-menu-item"
            role="menuitem"
            type="button"
            :disabled="regenLoading"
            @click="run(() => emit('regenerateToken'))"
          >
            {{ regenLoading ? 'Regenerating...' : 'Regenerate token' }}
          </button>
          <button
            v-if="canRestart"
            class="overflow-menu-item overflow-menu-item--danger"
            role="menuitem"
            type="button"
            :disabled="restartLoading"
            @click="run(() => emit('restart'))"
          >
            {{ restartLoading ? 'Restarting...' : 'Restart agent' }}
          </button>
          <span
            v-else-if="isOnline && agent.restart_unavailable_reason"
            class="overflow-menu-note"
          >
            {{ agent.restart_unavailable_reason }}
          </span>
        </template>
      </OverflowMenu>
    </template>

    <template #footer>
      <p
        v-if="restartError"
        class="form-error detail-header-error"
      >
        {{ restartError }}
      </p>
    </template>
  </DetailHeader>
</template>

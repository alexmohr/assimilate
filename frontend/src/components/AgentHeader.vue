<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { MoreHorizontal } from '@lucide/vue'
import { formatDate, relativeTime } from '../utils/format'
import { useOverflowMenu } from '../composables/useOverflowMenu'
import type { AgentRow } from '../types/agent'

/**
 * The agent detail page's identity block, shown above the tab strip and so
 * present on every tab. Actions are graded rather than listed: one accented
 * slot for the thing that is actionable right now, everything else - including
 * navigation like Activity log - behind the overflow menu. Before this the
 * same eight controls rendered as eight identical ghost buttons in one row,
 * with Activity Log indistinguishable from Restart Agent.
 */
const props = defineProps<{
  agent: AgentRow
  /** Non-null when a newer agent build is available and may be deployed. */
  deployLabel: string | null
  restartLoading: boolean
  regenLoading: boolean
  restartError: string | null
}>()

const emit = defineEmits<{
  adopt: []
  merge: []
  deploy: []
  activityLog: []
  editIdentity: []
  deploySshKey: []
  regenerateToken: []
  restart: []
}>()

const isImported = computed(() => props.agent.is_imported)
const isOnline = computed(() => props.agent.is_connected ?? false)

const menuRoot = ref<HTMLElement | null>(null)
const { menuOpen, runAndClose: fromMenu } = useOverflowMenu(menuRoot)

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
  <header class="agent-header">
    <div class="agent-identity">
      <div class="agent-title-row">
        <h1 class="agent-hostname mono">{{ agent.hostname }}</h1>
        <span
          v-if="isImported"
          class="badge badge--neutral"
          >Imported</span
        >
        <span
          v-else
          class="badge badge-dot"
          :class="isOnline ? 'badge--success' : 'badge--neutral'"
        >
          {{ isOnline ? 'Online' : 'Offline' }}
        </span>
        <span
          v-if="deployLabel === 'Upgrade'"
          class="badge badge--info"
          >Upgrade available</span
        >
      </div>
      <p
        v-if="agent.display_name"
        class="agent-subtitle"
      >
        {{ agent.display_name }}
      </p>
      <!--
        An imported host has no agent binary, so version, revision and build
        time do not exist for it. Rendering them as em dashes would imply the
        agent is there but silent, which is the opposite of the truth.
      -->
      <div class="agent-meta mono">
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
      </div>
    </div>

    <div
      ref="menuRoot"
      class="agent-actions"
    >
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

      <button
        class="btn btn-sm btn-ghost agent-menu-toggle"
        type="button"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        aria-label="More agent actions"
        @click="menuOpen = !menuOpen"
      >
        <MoreHorizontal :size="14" />
      </button>

      <div
        v-if="menuOpen"
        class="agent-menu"
        role="menu"
      >
        <button
          class="agent-menu-item"
          role="menuitem"
          type="button"
          @click="fromMenu(() => emit('activityLog'))"
        >
          Activity log
        </button>
        <template v-if="!isImported">
          <button
            class="agent-menu-item"
            role="menuitem"
            type="button"
            @click="fromMenu(() => emit('editIdentity'))"
          >
            Edit identity
          </button>
          <button
            class="agent-menu-item"
            role="menuitem"
            type="button"
            @click="fromMenu(() => emit('deploySshKey'))"
          >
            Deploy SSH key
          </button>
          <button
            class="agent-menu-item"
            role="menuitem"
            type="button"
            :disabled="regenLoading"
            @click="fromMenu(() => emit('regenerateToken'))"
          >
            {{ regenLoading ? 'Regenerating...' : 'Regenerate token' }}
          </button>
          <button
            v-if="canRestart"
            class="agent-menu-item agent-menu-item--danger"
            role="menuitem"
            type="button"
            :disabled="restartLoading"
            @click="fromMenu(() => emit('restart'))"
          >
            {{ restartLoading ? 'Restarting...' : 'Restart agent' }}
          </button>
          <span
            v-else-if="isOnline && agent.restart_unavailable_reason"
            class="agent-menu-note"
          >
            {{ agent.restart_unavailable_reason }}
          </span>
        </template>
      </div>
    </div>

    <p
      v-if="restartError"
      class="form-error agent-header-error"
    >
      {{ restartError }}
    </p>
  </header>
</template>

<style scoped>
.agent-header {
  display: flex;
  align-items: flex-start;
  gap: 1rem;
  flex-wrap: wrap;
  margin-bottom: 1rem;
}

.agent-identity {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  min-width: 0;
}

.agent-title-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  flex-wrap: wrap;
}

.agent-hostname {
  font-size: var(--fs-xl);
  font-weight: 650;
  letter-spacing: -0.02em;
  margin: 0;
}

.agent-subtitle {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  margin: 0;
}

.agent-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem 0.75rem;
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

.agent-meta b {
  font-weight: 500;
  color: var(--text-secondary);
}

.agent-actions {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  position: relative;
}

.agent-menu-toggle {
  padding-inline: 0.45rem;
}

.agent-menu {
  position: absolute;
  top: calc(100% + 0.35rem);
  right: 0;
  z-index: 20;
  min-width: 200px;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  padding: 0.3rem;
  display: flex;
  flex-direction: column;
}

.agent-menu-item {
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  padding: 0.35rem 0.5rem;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
}

.agent-menu-item:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.agent-menu-item:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.agent-menu-item--danger {
  color: var(--danger);
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius-sm) var(--radius-sm);
  margin-top: 0.25rem;
  padding-top: 0.45rem;
}

.agent-menu-note {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  padding: 0.35rem 0.5rem;
  font-style: italic;
}

.agent-header-error {
  flex-basis: 100%;
}
</style>

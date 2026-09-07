<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { ArrowDown, ArrowUp, Plus, TriangleAlert, X } from '@lucide/vue'
import ToggleSwitch from './ToggleSwitch.vue'
import type { ScheduleRepoTarget } from '../api/schedules'
import type { Repo } from '../types/repo'

/**
 * The ordered list of repositories a schedule writes into.
 *
 * A schedule used to own exactly one repository, so this used to be a
 * single dropdown.
 * Several copies from one schedule is the point of the feature - a local copy
 * that restores fast and an offsite one that survives the building - so the
 * list carries write order and, per target, whether its failure is the run's
 * failure. Each target is still its own borg run over the source.
 */
const props = defineProps<{
  repos: readonly Repo[]
  /** Marks the whole editor read-only while a save is in flight. */
  disabled?: boolean
}>()

const model = defineModel<ScheduleRepoTarget[]>({ required: true })

const repoById = computed(() => {
  const map = new Map<number, Repo>()
  props.repos.forEach((r) => map.set(r.id, r))
  return map
})

const chosenIds = computed(() => model.value.map((t) => t.repo_id))

const unusedRepos = computed(() => props.repos.filter((r) => !chosenIds.value.includes(r.id)))

/**
 * Two targets on one storage host are two copies that die together, which is
 * the one mistake this screen can actually catch for the operator.
 */
const sharedHosts = computed(() => {
  const seen = new Set<string>()
  const shared = new Set<string>()
  for (const target of model.value) {
    const host = repoById.value.get(target.repo_id)?.ssh_host
    if (!host) continue
    if (seen.has(host)) shared.add(host)
    seen.add(host)
  }
  return shared
})

function sharesHost(target: ScheduleRepoTarget): boolean {
  const host = repoById.value.get(target.repo_id)?.ssh_host
  return host !== undefined && sharedHosts.value.has(host)
}

function repoAddress(repoId: number): string {
  const repo = repoById.value.get(repoId)
  if (!repo) return ''
  return `${repo.ssh_user}@${repo.ssh_host}:${repo.repo_path}`
}

function isTakenElsewhere(repoId: number, index: number): boolean {
  return model.value.some((t, i) => i !== index && t.repo_id === repoId)
}

function setRepo(index: number, repoId: number): void {
  model.value = model.value.map((t, i) => (i === index ? { ...t, repo_id: repoId } : t))
}

function setRequired(index: number, required: boolean): void {
  model.value = model.value.map((t, i) => (i === index ? { ...t, required } : t))
}

function addTarget(): void {
  const next = unusedRepos.value[0]
  if (!next) return
  model.value = [...model.value, { repo_id: next.id, required: false }]
}

function removeTarget(index: number): void {
  model.value = model.value.filter((_, i) => i !== index)
}

function move(index: number, delta: number): void {
  const to = index + delta
  if (to < 0 || to >= model.value.length) return
  const next = [...model.value]
  ;[next[index], next[to]] = [next[to], next[index]]
  model.value = next
}

const requiredCount = computed(() => model.value.filter((t) => t.required).length)

const canAdd = computed(() => !props.disabled && unusedRepos.value.length > 0)
</script>

<template>
  <div class="field">
    <label class="field-label">
      Repositories
      <span class="required">*</span>
    </label>

    <div class="order-list">
      <div
        v-for="(target, idx) in model"
        :key="idx"
        class="order-item repo-target"
        :class="{ 'repo-target--warned': sharesHost(target) }"
      >
        <span class="order-index">{{ idx + 1 }}</span>
        <div class="repo-target-body">
          <div class="repo-target-head">
            <select
              class="input"
              :disabled="disabled"
              :aria-label="`Repository for target ${idx + 1}`"
              :value="target.repo_id"
              @change="setRepo(idx, Number(($event.target as HTMLSelectElement).value))"
            >
              <option
                v-for="r in repos"
                :key="r.id"
                :value="r.id"
                :disabled="isTakenElsewhere(r.id, idx)"
              >
                {{ r.name }}{{ isTakenElsewhere(r.id, idx) ? ' - already a target' : '' }}
              </option>
            </select>
            <span class="badge">{{ target.required ? 'Required' : 'Best effort' }}</span>
          </div>
          <span class="repo-target-address mono muted">{{ repoAddress(target.repo_id) }}</span>
          <div class="toggle-row">
            <span class="toggle-row-label">A failure here fails the run</span>
            <ToggleSwitch
              :model-value="target.required"
              :disabled="disabled"
              :label="`Target ${idx + 1} failure fails the run`"
              @update:model-value="setRequired(idx, $event)"
            />
          </div>
          <p
            v-if="sharesHost(target)"
            class="repo-target-warning"
          >
            <TriangleAlert :size="12" />
            Shares a storage host with another target - one host outage takes out both copies.
          </p>
        </div>
        <div class="order-actions">
          <button
            type="button"
            class="order-btn"
            :disabled="disabled || idx === 0"
            title="Move repository up"
            aria-label="Move repository up"
            @click="move(idx, -1)"
          >
            <ArrowUp :size="12" />
          </button>
          <button
            type="button"
            class="order-btn"
            :disabled="disabled || idx === model.length - 1"
            title="Move repository down"
            aria-label="Move repository down"
            @click="move(idx, 1)"
          >
            <ArrowDown :size="12" />
          </button>
          <button
            type="button"
            class="order-btn"
            :disabled="disabled || model.length === 1"
            title="Remove repository"
            aria-label="Remove repository"
            @click="removeTarget(idx)"
          >
            <X :size="12" />
          </button>
        </div>
      </div>
    </div>

    <button
      type="button"
      class="btn btn-sm btn-ghost repo-target-add"
      :disabled="!canAdd"
      @click="addTarget"
    >
      <Plus :size="14" />
      {{ unusedRepos.length > 0 ? 'Add repository' : 'Every repository is already a target' }}
    </button>

    <span class="field-hint">
      Written in this order, one after another, each as its own run over the source.
      {{ requiredCount }} of {{ model.length }} required.
    </span>
  </div>
</template>

<style scoped>
.repo-target {
  align-items: flex-start;
  padding: var(--space-5);
}

.repo-target--warned {
  border-color: var(--warning);
}

.repo-target-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  flex: 1;
  min-width: 0;
}

.repo-target-head {
  display: flex;
  align-items: center;
  gap: var(--space-5);
}

.repo-target-address {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.repo-target-warning {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin: 0;
  color: var(--warning);
  font-size: var(--fs-xs);
}

.repo-target-add {
  align-self: flex-start;
}
</style>

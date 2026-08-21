<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { X } from '@lucide/vue'
import { createTag, listEntityTags, listTags, setEntityTags } from '../api/tags'
import type { TagScope } from '../api/tags'
import { logger } from '../utils/logger'
import type { TagRow } from '../types/tag'

/**
 * Tag editor for a repository or an agent. Both had a verbatim copy of this
 * markup and these four handlers, differing only in the endpoint and the tag
 * scope.
 */
const props = defineProps<{
  /** Which tag namespace to list and create in. */
  scope: TagScope
  /** Collection path for this entity's tags, e.g. `/repos/12` or `/agents/web-01`. */
  entityPath: string
}>()

const DEFAULT_TAG_COLOR = '#6b7280'

const allTags = ref<TagRow[]>([])
const assignedIds = ref<number[]>([])
const newTagName = ref('')
const newTagColor = ref(DEFAULT_TAG_COLOR)
const createTagLoading = ref(false)

const assigned = computed<TagRow[]>(() =>
  allTags.value.filter((t) => assignedIds.value.includes(t.id)),
)

const available = computed<TagRow[]>(() =>
  allTags.value.filter((t) => !assignedIds.value.includes(t.id)),
)

async function load(): Promise<void> {
  try {
    const [tags, ownTags] = await Promise.all([
      listTags(props.scope),
      // A failure here is not fatal: the picker still works, it just starts
      // with nothing assigned rather than blanking the whole panel.
      listEntityTags(props.entityPath).catch((e: unknown) => {
        logger.error('load tags failed', e)
        return [] as TagRow[]
      }),
    ])
    allTags.value = tags
    assignedIds.value = ownTags.map((t) => t.id)
  } catch (e: unknown) {
    logger.error('loadTags failed', e)
  }
}

async function save(updated: number[]): Promise<void> {
  await setEntityTags(props.entityPath, updated)
  assignedIds.value = updated
}

async function addTag(tagId: number): Promise<void> {
  try {
    await save([...assignedIds.value, tagId])
  } catch (e: unknown) {
    logger.error('addTag failed', e)
  }
}

async function removeTag(tagId: number): Promise<void> {
  try {
    await save(assignedIds.value.filter((id) => id !== tagId))
  } catch (e: unknown) {
    logger.error('removeTag failed', e)
  }
}

async function createAndAddTag(): Promise<void> {
  if (!newTagName.value.trim()) return
  createTagLoading.value = true
  try {
    const created = await createTag(newTagName.value.trim(), newTagColor.value, props.scope)
    allTags.value.push(created)
    await addTag(created.id)
    newTagName.value = ''
    newTagColor.value = DEFAULT_TAG_COLOR
  } catch (e: unknown) {
    logger.error('createAndAddTag failed', e)
  } finally {
    createTagLoading.value = false
  }
}

function onSelectExisting(e: Event): void {
  const select = e.target as HTMLSelectElement
  const id = Number(select.value)
  if (id) void addTag(id)
  select.value = ''
}

watch(() => props.entityPath, load)
onMounted(load)

defineExpose({ reload: load })
</script>

<template>
  <div class="tags-section">
    <div
      v-if="assigned.length > 0"
      class="tag-list"
    >
      <span
        v-for="tag in assigned"
        :key="tag.id"
        class="tag-pill"
        :style="{
          background: tag.color + '22',
          color: tag.color,
          borderColor: tag.color + '44',
        }"
      >
        {{ tag.name }}
        <button
          class="tag-remove"
          :aria-label="`Remove tag ${tag.name}`"
          @click="removeTag(tag.id)"
        >
          <X :size="12" />
        </button>
      </span>
    </div>
    <span
      v-else
      class="muted"
      >No tags assigned.</span
    >

    <div class="tag-add-row">
      <select
        v-if="available.length > 0"
        class="input input-sm"
        aria-label="Add existing tag"
        @change="onSelectExisting"
      >
        <option value="">Add existing tag...</option>
        <option
          v-for="t in available"
          :key="t.id"
          :value="t.id"
        >
          {{ t.name }}
        </option>
      </select>
      <div class="tag-create-inline">
        <input
          v-model="newTagName"
          class="input input-sm"
          placeholder="New tag name"
          aria-label="New tag name"
        />
        <input
          v-model="newTagColor"
          class="input color-input"
          type="color"
          aria-label="New tag colour"
        />
        <button
          class="btn btn-sm btn-ghost"
          :disabled="!newTagName.trim() || createTagLoading"
          @click="createAndAddTag"
        >
          {{ createTagLoading ? '...' : '+ Create' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.color-input {
  width: 2.5rem;
  padding: var(--space-1);
  cursor: pointer;
}
</style>

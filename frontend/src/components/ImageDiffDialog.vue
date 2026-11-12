<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowRight } from '@lucide/vue'
import BaseModal from './BaseModal.vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import BaseSpinner from './BaseSpinner.vue'
import { getArchiveFile } from '../api/archives'
import { extractBlobError } from '../utils/error'
import { formatBytes, formatDate } from '../utils/format'
import type { ArchiveEntryResponse } from '../types/generated'

/**
 * An image that changed between two archives, compared the way a code review
 * compares one: both side by side (2-up), the new one wiped across the old
 * (Swipe), or faded over it (Onion skin). The older archive is "before".
 */
const props = defineProps<{
  open: boolean
  repoId: number
  /** The archive the change was found in: the "after" side. */
  archive: ArchiveEntryResponse
  /** The archive it is compared against: the "before" side. */
  base: ArchiveEntryResponse
  path: string
}>()

const emit = defineEmits<{ close: [] }>()

type Mode = 'two_up' | 'swipe' | 'onion'

const MODES: readonly SegmentedOption<Mode>[] = [
  { value: 'two_up', label: '2-up' },
  { value: 'swipe', label: 'Swipe' },
  { value: 'onion', label: 'Onion skin' },
]

interface Side {
  url: string
  bytes: number
  width: number
  height: number
}

const mode = ref<Mode>('two_up')
/** Swipe: how far across the new image reaches; onion skin: its opacity. Both 0-100. */
const swipe = ref(50)
const onion = ref(50)
const before = ref<Side | null>(null)
const after = ref<Side | null>(null)
const loading = ref(false)
const error = ref<string | null>(null)

const fileName = computed(() => props.path.split('/').pop() ?? props.path)

/** The stage both stacked modes draw on: big enough for the larger of the two. */
const stage = computed(() => ({
  width: Math.max(before.value?.width ?? 0, after.value?.width ?? 0),
  height: Math.max(before.value?.height ?? 0, after.value?.height ?? 0),
}))

function dimensions(side: Side): string {
  return `${side.width} x ${side.height}`
}

/** What changed in size, for the line under the images. */
const sizeChange = computed<string | null>(() => {
  if (!before.value || !after.value) return null
  const delta = after.value.bytes - before.value.bytes
  if (delta === 0) return null
  return `${delta > 0 ? '+' : '−'}${formatBytes(Math.abs(delta))}`
})

function measure(url: string): Promise<{ width: number; height: number }> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight })
    img.onerror = () => reject(new Error('The file could not be shown as an image.'))
    img.src = url
  })
}

async function loadSide(archiveName: string): Promise<Side> {
  const blob = await getArchiveFile(props.repoId, archiveName, props.path)
  const url = URL.createObjectURL(blob)
  try {
    const { width, height } = await measure(url)
    return { url, bytes: blob.size, width, height }
  } catch (e: unknown) {
    URL.revokeObjectURL(url)
    throw e
  }
}

function release(): void {
  if (before.value) URL.revokeObjectURL(before.value.url)
  if (after.value) URL.revokeObjectURL(after.value.url)
  before.value = null
  after.value = null
}

let seq = 0

async function load(): Promise<void> {
  const mine = ++seq
  release()
  loading.value = true
  error.value = null
  const [b, a] = await Promise.allSettled([loadSide(props.base.name), loadSide(props.archive.name)])
  if (mine !== seq || b.status === 'rejected' || a.status === 'rejected') {
    // Nothing is shown, so neither side's image may outlive this load.
    if (b.status === 'fulfilled') URL.revokeObjectURL(b.value.url)
    if (a.status === 'fulfilled') URL.revokeObjectURL(a.value.url)
    if (mine !== seq) return
    // The file is fetched as a blob, so a server error arrives as one too.
    const reason: unknown =
      b.status === 'rejected' ? b.reason : a.status === 'rejected' ? a.reason : null
    const message = await extractBlobError(reason)
    if (mine === seq) {
      error.value = message
      loading.value = false
    }
    return
  }
  before.value = b.value
  after.value = a.value
  loading.value = false
}

watch(
  () => [props.open, props.path, props.archive.name, props.base.name] as const,
  ([open]) => {
    if (!open) {
      release()
      return
    }
    mode.value = 'two_up'
    swipe.value = 50
    onion.value = 50
    void load()
  },
  { immediate: true },
)

onBeforeUnmount(release)
</script>

<template>
  <BaseModal
    :open="open"
    :title="`Compare ${fileName}`"
    size="lg"
    @close="emit('close')"
  >
    <div class="image-diff">
      <div class="image-diff-bar">
        <BaseSegmented
          v-model="mode"
          :options="MODES"
          label="How to compare"
        />
        <span class="image-diff-path mono">{{ path }}</span>
      </div>

      <BaseSpinner
        v-if="loading"
        size="lg"
      />
      <p
        v-else-if="error"
        class="form-error"
      >
        {{ error }}
      </p>

      <template v-else-if="before && after">
        <!-- 2-up: each image at its own size, labelled with what it is. -->
        <div
          v-if="mode === 'two_up'"
          class="image-diff-two"
        >
          <figure class="image-diff-side image-diff-side--before">
            <div class="image-diff-frame">
              <img
                :src="before.url"
                :alt="`${fileName} before`"
              />
            </div>
            <figcaption>
              <span class="image-diff-label">Before</span>
              <span :title="formatDate(base.start)">{{ base.name }}</span>
              <span>{{ dimensions(before) }} · {{ formatBytes(before.bytes) }}</span>
            </figcaption>
          </figure>
          <figure class="image-diff-side image-diff-side--after">
            <div class="image-diff-frame">
              <img
                :src="after.url"
                :alt="`${fileName} after`"
              />
            </div>
            <figcaption>
              <span class="image-diff-label">After</span>
              <span :title="formatDate(archive.start)">{{ archive.name }}</span>
              <span>{{ dimensions(after) }} · {{ formatBytes(after.bytes) }}</span>
            </figcaption>
          </figure>
        </div>

        <!-- Swipe and onion skin: the two stacked on one stage. -->
        <template v-else>
          <div
            class="image-diff-stage"
            :style="{
              aspectRatio: `${stage.width} / ${stage.height}`,
              maxWidth: `${stage.width}px`,
            }"
          >
            <img
              class="image-diff-layer"
              :src="before.url"
              :alt="`${fileName} before`"
            />
            <div
              v-if="mode === 'swipe'"
              class="image-diff-layer image-diff-wipe"
              :style="{ clipPath: `inset(0 ${100 - swipe}% 0 0)` }"
            >
              <img
                :src="after.url"
                :alt="`${fileName} after`"
              />
            </div>
            <span
              v-if="mode === 'swipe'"
              class="image-diff-handle"
              :style="{ left: `${swipe}%` }"
              aria-hidden="true"
            />
            <img
              v-if="mode === 'onion'"
              class="image-diff-layer"
              :src="after.url"
              :alt="`${fileName} after`"
              :style="{ opacity: onion / 100 }"
            />
          </div>
          <label class="image-diff-slider">
            <span class="image-diff-label image-diff-label--before">Before</span>
            <input
              v-if="mode === 'swipe'"
              v-model.number="swipe"
              class="input image-diff-range"
              type="range"
              min="0"
              max="100"
              aria-label="How much of the new image to show"
            />
            <input
              v-else
              v-model.number="onion"
              class="input image-diff-range"
              type="range"
              min="0"
              max="100"
              aria-label="Opacity of the new image"
            />
            <span class="image-diff-label image-diff-label--after">After</span>
          </label>
        </template>

        <p class="image-diff-summary">
          <span class="image-diff-change"
            >{{ dimensions(before) }}
            <ArrowRight
              :size="12"
              aria-label="to"
            />
            {{ dimensions(after) }}</span
          >
          <span class="image-diff-change"
            >{{ formatBytes(before.bytes) }}
            <ArrowRight
              :size="12"
              aria-label="to"
            />
            {{ formatBytes(after.bytes) }}</span
          >
          <span v-if="sizeChange">{{ sizeChange }}</span>
        </p>
      </template>
    </div>
  </BaseModal>
</template>

<style scoped>
.image-diff {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
  min-width: 0;
}

.image-diff-bar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-5);
}

.image-diff-path {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  overflow-wrap: anywhere;
  min-width: 0;
}

/* Transparent pixels show as a checkerboard rather than the card colour. */
.image-diff-frame,
.image-diff-stage {
  background-color: var(--bg-base);
  background-image:
    linear-gradient(45deg, var(--bg-hover) 25%, transparent 25%),
    linear-gradient(-45deg, var(--bg-hover) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, var(--bg-hover) 75%),
    linear-gradient(-45deg, transparent 75%, var(--bg-hover) 75%);
  background-size: 16px 16px;
  background-position:
    0 0,
    0 8px,
    8px -8px,
    -8px 0;
}

.image-diff-two {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-6);
}

.image-diff-side {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  margin: 0;
  min-width: 0;
}

/* Red for what was there, green for what is: the colours of a text diff. */
.image-diff-side--before .image-diff-frame {
  border: 2px solid var(--danger);
}

.image-diff-side--after .image-diff-frame {
  border: 2px solid var(--success);
}

.image-diff-frame {
  display: grid;
  place-items: center;
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.image-diff-frame img {
  display: block;
  max-width: 100%;
  height: auto;
}

.image-diff-side figcaption {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  min-width: 0;
  overflow-wrap: anywhere;
}

.image-diff-label {
  font-size: var(--fs-2xs);
  font-weight: 700;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.image-diff-side--before .image-diff-label,
.image-diff-label--before {
  color: var(--danger);
}

.image-diff-side--after .image-diff-label,
.image-diff-label--after {
  color: var(--success);
}

.image-diff-stage {
  position: relative;
  width: 100%;
  margin: 0 auto;
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.image-diff-layer {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.image-diff-wipe img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.image-diff-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  margin-left: -1px;
  background: var(--accent);
  pointer-events: none;
}

.image-diff-slider {
  display: flex;
  align-items: center;
  gap: var(--space-5);
}

/* A slider takes the shared .input class like every control, but none of a
   text field's fill, border or padding: the track is the field. */
.image-diff-range,
.image-diff-range:hover,
.image-diff-range:focus {
  flex: 1;
  min-width: 0;
  width: auto;
  padding: 0;
  border: none;
  background: transparent;
  box-shadow: none;
  accent-color: var(--accent);
}

.image-diff-range:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.image-diff-change {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}

.image-diff-summary {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-6);
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}

@media (max-width: 640px) {
  .image-diff-two {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>

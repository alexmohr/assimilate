<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ChevronRight, Eye, RotateCcw, Tag } from '@lucide/vue'
import { previewTemplate, updateChannel } from '../api/notifications'
import { extractError } from '../utils/error'
import { logger } from '../utils/logger'
import {
  DEFAULT_BODY_TEMPLATE,
  DEFAULT_PUSH_BODY_TEMPLATE,
  DEFAULT_TITLE_TEMPLATE,
  TEMPLATE_PLACEHOLDERS,
} from '../utils/notificationTemplate'
import { withTemplates } from '../utils/channelConfig'
import type { EventType, NotificationChannelResponse } from '../types/generated'

/**
 * Every channel keeps its own title/message template - editing one channel's content never
 * touches another's. Always shown pre-filled (with the shared default, dedup size included on
 * a successful backup) rather than gated behind a "customize" toggle: there's nothing to turn
 * on, the fields are just there to edit.
 */
const props = defineProps<{ channel: NotificationChannelResponse }>()
const emit = defineEmits<{ updated: [channel: NotificationChannelResponse] }>()

// A channel created before per-channel content existed has no template of its own yet, and
// shows (and delivers) the shared default until one is saved. A channel that arrives without
// a config at all (a partial fixture, or a response that never matched its type) falls back
// to the defaults too rather than throwing, so every other channel card keeps rendering.
// Web push gets its own short default body -- a push toast is typically clipped to one or two
// lines by the browser, so the multi-line email/webhook default would just get cut off. See
// the matching default in notificationTemplate.ts for the full reasoning.
const defaultBody = computed((): string =>
  props.channel.channel_type === 'web_push' ? DEFAULT_PUSH_BODY_TEMPLATE : DEFAULT_BODY_TEMPLATE,
)
const savedTitle = computed(
  (): string => props.channel.config?.title_template ?? DEFAULT_TITLE_TEMPLATE,
)
const savedBody = computed((): string => props.channel.config?.body_template ?? defaultBody.value)

const expanded = ref(false)
const title = ref(savedTitle.value)
const body = ref(savedBody.value)
const sampleEvent = ref<EventType>('backup_success')
const saving = ref(false)
const error = ref('')
const lastFocused = ref<HTMLInputElement | HTMLTextAreaElement | null>(null)
const titleInput = ref<HTMLInputElement | null>(null)
const bodyInput = ref<HTMLTextAreaElement | null>(null)

const dirty = computed((): boolean => {
  return title.value !== savedTitle.value || body.value !== savedBody.value
})

// A blank title/body isn't just an empty preview: `Some("")` is a different value than
// "no template" to every delivery path, which all treat "template present" (even if blank)
// as "don't fall back to the built-in default" -- so saving one would silently blank this
// channel's real notifications. The server rejects it too, but catching it here means the
// button itself tells the admin why they can't save it instead of surfacing a server error
// after the fact.
const isBlank = computed((): boolean => {
  return title.value.trim() === '' || body.value.trim() === ''
})

// The preview is rendered by the server with the renderer a real delivery uses, against its
// sample payload for the chosen event, so it shows exactly what this channel would send.
const renderedTitle = ref('')
const renderedBody = ref('')
const showsDedupSize = computed((): boolean => {
  return /Dedup:\s+\S/.test(renderedBody.value)
})

/** How long typing has to pause before the preview is re-rendered. */
const PREVIEW_DELAY_MS = 300

let previewTimer: ReturnType<typeof setTimeout> | undefined
let previewRequest = 0

async function refreshPreview(): Promise<void> {
  previewRequest += 1
  const request = previewRequest
  try {
    const preview = await previewTemplate({
      title_template: title.value,
      body_template: body.value,
      event_type: sampleEvent.value,
    })
    if (request !== previewRequest) return
    renderedTitle.value = preview.title
    renderedBody.value = preview.body
  } catch (e: unknown) {
    logger.debug('notification preview failed', e)
  }
}

function schedulePreview(): void {
  clearTimeout(previewTimer)
  previewTimer = setTimeout(() => {
    void refreshPreview()
  }, PREVIEW_DELAY_MS)
}

// Only while the editor is open: a collapsed card has no preview to show. Opening it or
// switching the sample renders at once; typing waits for a pause.
watch(expanded, (open) => {
  if (open) void refreshPreview()
})
watch(sampleEvent, () => {
  if (expanded.value) void refreshPreview()
})
watch([title, body], () => {
  if (expanded.value) schedulePreview()
})
onBeforeUnmount(() => clearTimeout(previewTimer))

function toggle(): void {
  expanded.value = !expanded.value
}

function trackFocus(el: HTMLInputElement | HTMLTextAreaElement): void {
  lastFocused.value = el
}

function insertPlaceholder(key: string): void {
  const el = lastFocused.value ?? bodyInput.value
  if (!el) return
  el.focus()
  const start = el.selectionStart ?? el.value.length
  const end = el.selectionEnd ?? el.value.length
  const text = `{{${key}}}`
  const target = el === titleInput.value ? title : body
  target.value = el.value.slice(0, start) + text + el.value.slice(end)
  requestAnimationFrame(() => {
    el.focus()
    el.setSelectionRange(start + text.length, start + text.length)
  })
}

function placeholderToken(key: string): string {
  return `{{${key}}}`
}

function resetToDefault(): void {
  title.value = DEFAULT_TITLE_TEMPLATE
  body.value = defaultBody.value
}

async function save(): Promise<void> {
  saving.value = true
  error.value = ''
  try {
    const updated = await updateChannel(
      props.channel.id,
      withTemplates(props.channel, title.value, body.value),
    )
    emit('updated', updated)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    saving.value = false
  }
}

defineExpose({ expanded })
</script>

<template>
  <div class="content-editor">
    <button
      class="btn btn-sm btn-ghost content-toggle"
      :class="{ 'content-toggle--open': expanded }"
      @click="toggle"
    >
      <ChevronRight
        class="content-toggle-chevron"
        :size="12"
      />
      {{ expanded ? 'Hide content' : 'Edit content' }}
    </button>

    <div
      v-if="expanded"
      class="content-panel"
    >
      <p class="content-intro">
        Pre-filled with Assimilate's default content, including the deduplicated size on a
        successful backup. Changes here apply only to <strong>{{ channel.name }}</strong
        >.
      </p>

      <div class="field">
        <label
          class="field-label"
          :for="`content-title-${channel.id}`"
          >Title</label
        >
        <input
          :id="`content-title-${channel.id}`"
          ref="titleInput"
          v-model="title"
          class="input mono"
          type="text"
          @focus="trackFocus(titleInput!)"
        />
      </div>

      <div class="field">
        <label
          class="field-label"
          :for="`content-body-${channel.id}`"
          >Message</label
        >
        <textarea
          :id="`content-body-${channel.id}`"
          ref="bodyInput"
          v-model="body"
          class="input mono content-textarea"
          spellcheck="false"
          @focus="trackFocus(bodyInput!)"
        ></textarea>
      </div>

      <div class="content-var-picker">
        <span class="content-var-picker-label">
          <Tag :size="12" />
          Click a variable to insert it at your cursor
        </span>
        <div class="content-chips">
          <button
            v-for="p in TEMPLATE_PLACEHOLDERS"
            :key="p.key"
            type="button"
            class="content-chip"
            :class="{ 'content-chip--dedup': p.dedup }"
            :title="p.description"
            @click="insertPlaceholder(p.key)"
          >
            {{ placeholderToken(p.key) }}
          </button>
        </div>
      </div>

      <button
        class="content-reset-link"
        type="button"
        @click="resetToDefault"
      >
        <RotateCcw :size="12" />
        Reset to default content
      </button>

      <div class="content-preview">
        <div class="content-preview-toolbar">
          <div class="content-preview-toolbar-left">
            <span class="content-preview-label">
              <Eye :size="12" />
              Live preview
            </span>
            <span
              v-if="showsDedupSize"
              class="content-dedup-tag"
            >
              Dedup size shown by default
            </span>
          </div>
          <select
            v-model="sampleEvent"
            class="input content-preview-select"
          >
            <option value="backup_success">Sample: Backup succeeded</option>
            <option value="backup_warning">Sample: Backup warning</option>
            <option value="backup_failed">Sample: Backup failed</option>
            <option value="agent_connected">Sample: Agent connected</option>
          </select>
        </div>
        <div class="content-preview-body">
          <div class="content-preview-subject">{{ renderedTitle }}</div>
          <pre class="content-preview-pre">{{ renderedBody }}</pre>
        </div>
      </div>

      <div
        v-if="isBlank"
        class="form-error"
      >
        Title and message can't be blank -- notifications sent through this channel would go out
        empty.
      </div>

      <div
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </div>

      <div class="content-actions">
        <button
          class="btn btn-sm btn-primary"
          :disabled="saving || !dirty || isBlank"
          @click="save"
        >
          {{ saving ? 'Saving...' : 'Save content' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.content-editor {
  margin-top: var(--space-4);
}

.content-toggle {
  gap: var(--space-2);
}

.content-toggle-chevron {
  transition: transform var(--duration-base) ease;
}

.content-toggle--open .content-toggle-chevron {
  transform: rotate(90deg);
}

.content-panel {
  margin-top: var(--space-5);
  padding-top: var(--space-5);
  border-top: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.content-intro {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  max-width: 60ch;
}

.content-textarea {
  min-height: 9rem;
  resize: vertical;
  line-height: 1.6;
}

.content-var-picker {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.content-var-picker-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.content-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.content-chip {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  padding: var(--space-2) var(--space-4);
  border-radius: var(--radius-pill);
  background: var(--bg-hover);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  cursor: pointer;
}

.content-chip:hover {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-subtle);
}

.content-chip--dedup {
  border-color: var(--success);
  color: var(--success);
}

.content-chip--dedup:hover {
  background: var(--success-subtle);
}

.content-reset-link {
  align-self: flex-start;
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  background: none;
  border: none;
  padding: 0;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-muted);
  cursor: pointer;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.content-reset-link:hover {
  color: var(--accent);
}

.content-preview {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-elevated);
  overflow: hidden;
}

.content-preview-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-3) var(--space-5);
  background: var(--bg-hover);
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}

.content-preview-toolbar-left {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.content-preview-label {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.content-dedup-tag {
  padding: var(--space-1) var(--space-3);
  border-radius: var(--radius-pill);
  background: var(--success-subtle);
  color: var(--success);
  font-size: var(--fs-2xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.content-preview-select {
  width: auto;
}

.content-preview-body {
  padding: var(--space-5);
}

.content-preview-subject {
  font-weight: 650;
  font-size: var(--fs-md);
  color: var(--text-primary);
}

.content-preview-pre {
  margin: var(--space-4) 0 0;
  padding: var(--space-4) var(--space-5);
  background: var(--bg-code);
  border-radius: var(--radius-sm);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  line-height: 1.65;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}

.content-actions {
  display: flex;
  justify-content: flex-end;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { apiClient } from '../api/client'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseModal from './BaseModal.vue'

interface ArchiveEntry {
  name: string
  start: string
  hostname: string
  comment: string
}

interface Props {
  open: boolean
  repoId: number | null
  archives: ArchiveEntry[]
}

const props = defineProps<Props>()

const emit = defineEmits<{
  close: []
}>()

type RestoreMethod = 'download' | 'agent'

const step = ref(1)
const selectedArchiveName = ref<string | null>(null)
const pathsInput = ref('')
const restoreMethod = ref<RestoreMethod>('download')
const targetPath = ref('')
const hostname = ref('')
const { loading: executing, error, run } = useAsyncAction()
const success = ref(false)

const totalSteps = 4

const paths = computed<string[]>(() =>
  pathsInput.value
    .split('\n')
    .map((p) => p.trim())
    .filter((p) => p.length > 0),
)

const canProceed = computed<boolean>(() => {
  switch (step.value) {
    case 1:
      return selectedArchiveName.value !== null
    case 2:
      return paths.value.length > 0
    case 3:
      if (restoreMethod.value === 'agent') {
        return targetPath.value.trim().length > 0 && hostname.value.trim().length > 0
      }
      return true
    case 4:
      return true
    default:
      return false
  }
})

function reset(): void {
  step.value = 1
  selectedArchiveName.value = null
  pathsInput.value = ''
  restoreMethod.value = 'download'
  targetPath.value = ''
  hostname.value = ''
  executing.value = false
  error.value = null
  success.value = false
}

function close(): void {
  reset()
  emit('close')
}

function next(): void {
  if (step.value < totalSteps) {
    step.value += 1
  }
}

function back(): void {
  if (step.value > 1) {
    step.value -= 1
  }
}

async function execute(): Promise<void> {
  if (props.repoId === null || selectedArchiveName.value === null) return

  const archiveEncoded = encodeURIComponent(selectedArchiveName.value)

  await run(async () => {
    if (restoreMethod.value === 'download') {
      const response = await apiClient.post(
        `/repos/${props.repoId}/archives/${archiveEncoded}/download`,
        { paths: paths.value },
        { responseType: 'blob' },
      )
      const blob = response.data as Blob
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `restore-${selectedArchiveName.value}.tar`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
    } else {
      await apiClient.post(`/repos/${props.repoId}/archives/${archiveEncoded}/restore`, {
        paths: paths.value,
        target_path: targetPath.value.trim(),
        hostname: hostname.value.trim(),
      })
    }
    success.value = true
  })
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Restore Files"
    size="lg"
    @close="close"
  >
    <!-- Step indicators -->
    <div class="steps-indicator">
      <div
        v-for="s in totalSteps"
        :key="s"
        class="step-dot"
        :class="{ active: s === step, completed: s < step }"
      >
        {{ s }}
      </div>
    </div>

    <!-- Success state -->
    <div
      v-if="success"
      class="success-msg"
    >
      <p>Restore completed successfully.</p>
      <button
        class="btn btn-primary"
        @click="close"
      >
        Done
      </button>
    </div>

    <!-- Step 1: Select archive -->
    <div
      v-else-if="step === 1"
      class="step-content"
    >
      <label class="field-label">Select Archive</label>
      <select
        v-model="selectedArchiveName"
        class="select-input full-width"
      >
        <option
          :value="null"
          disabled
        >
          — choose archive —
        </option>
        <option
          v-for="archive in archives"
          :key="archive.name"
          :value="archive.name"
        >
          {{ archive.name }}
        </option>
      </select>
    </div>

    <!-- Step 2: Enter paths -->
    <div
      v-else-if="step === 2"
      class="step-content"
    >
      <label class="field-label">Paths to restore (one per line)</label>
      <textarea
        v-model="pathsInput"
        class="textarea-input"
        rows="6"
        placeholder="/etc/nginx/nginx.conf&#10;/home/user/documents"
      />
      <p class="field-hint">Enter full paths to files or directories you want to restore.</p>
    </div>

    <!-- Step 3: Restore method -->
    <div
      v-else-if="step === 3"
      class="step-content"
    >
      <label class="field-label">Restore Method</label>
      <div class="radio-group">
        <label class="radio-option">
          <input
            v-model="restoreMethod"
            type="radio"
            value="download"
          />
          <span>Download to browser</span>
        </label>
        <label class="radio-option">
          <input
            v-model="restoreMethod"
            type="radio"
            value="agent"
          />
          <span>Restore to agent filesystem</span>
        </label>
      </div>

      <template v-if="restoreMethod === 'agent'">
        <label class="field-label mt-1">Target hostname</label>
        <input
          v-model="hostname"
          type="text"
          class="text-input full-width"
          placeholder="backup-host-01"
        />
        <label class="field-label mt-1">Target path</label>
        <input
          v-model="targetPath"
          type="text"
          class="text-input full-width"
          placeholder="/tmp/restore"
        />
      </template>
    </div>

    <!-- Step 4: Confirm -->
    <div
      v-else-if="step === 4"
      class="step-content"
    >
      <label class="field-label">Confirm Restore</label>
      <dl class="confirm-list">
        <dt>Archive</dt>
        <dd>{{ selectedArchiveName }}</dd>
        <dt>Paths</dt>
        <dd>
          <code
            v-for="p in paths"
            :key="p"
            class="path-tag"
            >{{ p }}</code
          >
        </dd>
        <dt>Method</dt>
        <dd>
          {{
            restoreMethod === 'download'
              ? 'Download to browser'
              : `Agent restore → ${hostname}:${targetPath}`
          }}
        </dd>
      </dl>
      <div
        v-if="error"
        class="form-error"
      >
        {{ error }}
      </div>
    </div>

    <template #footer>
      <template v-if="!success">
        <button
          v-if="step > 1"
          class="btn btn-ghost"
          :disabled="executing"
          @click="back"
        >
          Back
        </button>
        <button
          v-if="step < totalSteps"
          class="btn btn-primary"
          :disabled="!canProceed"
          @click="next"
        >
          Next
        </button>
        <button
          v-if="step === totalSteps"
          class="btn btn-primary"
          :disabled="!canProceed || executing"
          @click="execute"
        >
          {{ executing ? 'Restoring...' : 'Restore' }}
        </button>
      </template>
    </template>
  </BaseModal>
</template>

<style scoped>
.steps-indicator {
  display: flex;
  gap: 0.5rem;
  justify-content: center;
  margin-bottom: 1.5rem;
}

.step-dot {
  width: 2rem;
  height: 2rem;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.75rem;
  font-weight: 600;
  background: var(--bg-card);
  border: 2px solid var(--border);
  color: var(--text-muted);
}

.step-dot.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-subtle);
}

.step-dot.completed {
  border-color: var(--success);
  background: var(--success);
  color: #fff;
}

.step-content {
  min-height: 140px;
}

.field-label {
  display: block;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-bottom: 0.5rem;
}

.field-hint {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin-top: 0.4rem;
}

.select-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.55rem 0.75rem;
  font-size: 0.875rem;
}

.full-width {
  width: 100%;
}

.textarea-input {
  width: 100%;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.55rem 0.75rem;
  font-size: 0.85rem;
  font-family: var(--mono);
  resize: vertical;
}

.textarea-input:focus,
.text-input:focus,
.select-input:focus {
  outline: none;
  border-color: var(--accent);
}

.text-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.55rem 0.75rem;
  font-size: 0.875rem;
}

.radio-group {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  margin-bottom: 1rem;
}

.radio-option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.875rem;
  color: var(--text-primary);
  cursor: pointer;
}

.mt-1 {
  margin-top: 1rem;
}

.confirm-list {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.4rem 1rem;
  font-size: 0.85rem;
}

.confirm-list dt {
  font-weight: 600;
  color: var(--text-muted);
}

.confirm-list dd {
  color: var(--text-primary);
  margin: 0;
}

.path-tag {
  display: inline-block;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.1rem 0.4rem;
  font-size: 0.78rem;
  margin-right: 0.3rem;
  margin-bottom: 0.2rem;
}

.form-error {
  color: var(--danger);
  font-size: 0.85rem;
  margin-top: 0.75rem;
}

.success-msg {
  text-align: center;
  padding: 2rem 0;
  color: var(--success);
  font-weight: 500;
}

.success-msg p {
  margin-bottom: 1rem;
}
</style>

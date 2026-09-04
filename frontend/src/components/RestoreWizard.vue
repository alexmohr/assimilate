<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import axios from 'axios'
import { downloadArchiveFiles, restoreArchiveFiles } from '../api/archives'
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
const downloadAbortController = ref<AbortController | null>(null)

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
  downloadAbortController.value?.abort()
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
  const repoId = props.repoId
  const archiveName = selectedArchiveName.value
  if (repoId === null || archiveName === null) return

  await run(async () => {
    if (restoreMethod.value === 'download') {
      const controller = new AbortController()
      downloadAbortController.value = controller
      try {
        const blob = await downloadArchiveFiles(repoId, archiveName, paths.value, controller.signal)
        const url = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = url
        a.download = `restore-${archiveName}.tar`
        document.body.appendChild(a)
        a.click()
        document.body.removeChild(a)
        URL.revokeObjectURL(url)
      } catch (e) {
        if (axios.isCancel(e)) return
        throw e
      } finally {
        downloadAbortController.value = null
      }
    } else {
      await restoreArchiveFiles(repoId, archiveName, {
        paths: paths.value,
        target_path: targetPath.value.trim(),
        hostname: hostname.value.trim(),
      })
    }
    success.value = true
  })
}

function cancelDownload(): void {
  downloadAbortController.value?.abort()
}
</script>

<template>
  <BaseModal
    :open="open"
    title="Restore files"
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
      <label class="field-label">Select archive</label>
      <select
        v-model="selectedArchiveName"
        class="input"
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
        class="input textarea-input"
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
      <label class="field-label">Restore method</label>
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
          class="input text-input full-width"
          placeholder="backup-host-01"
        />
        <label class="field-label mt-1">Target path</label>
        <input
          v-model="targetPath"
          type="text"
          class="input text-input full-width"
          placeholder="/tmp/restore"
        />
      </template>
    </div>

    <!-- Step 4: Confirm -->
    <div
      v-else-if="step === 4"
      class="step-content"
    >
      <label class="field-label">Confirm restore</label>
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
              : `Agent restore to ${hostname}:${targetPath}`
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
          v-if="step === totalSteps && !(executing && restoreMethod === 'download')"
          class="btn btn-primary"
          :disabled="!canProceed || executing"
          @click="execute"
        >
          {{ executing ? 'Restoring...' : 'Restore' }}
        </button>
        <button
          v-if="step === totalSteps && executing && restoreMethod === 'download'"
          class="btn btn-ghost"
          @click="cancelDownload"
        >
          Cancel download
        </button>
      </template>
    </template>
  </BaseModal>
</template>

<style scoped>
.field-label {
  display: block;
  margin-bottom: var(--space-4);
}

.field-hint {
  margin-top: var(--space-3);
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
  padding: var(--space-4) var(--space-5);
  font-size: var(--fs-base);
  font-family: var(--mono);
  resize: vertical;
}

.textarea-input:focus,
.text-input:focus {
  outline: none;
  border-color: var(--accent);
}

.text-input {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: var(--space-4) var(--space-5);
  font-size: var(--fs-base);
}

.radio-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.radio-option {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--fs-base);
  color: var(--text-primary);
  cursor: pointer;
}

.mt-1 {
  margin-top: var(--space-6);
}

.confirm-list {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: var(--space-3) var(--space-6);
  font-size: var(--fs-base);
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
  padding: var(--space-1) var(--space-3);
  font-size: var(--fs-xs);
  margin-right: var(--space-2);
  margin-bottom: var(--space-2);
}

.success-msg {
  text-align: center;
  padding: var(--space-9) 0;
  color: var(--success);
  font-weight: 500;
}

.success-msg p {
  margin-bottom: var(--space-6);
}
</style>

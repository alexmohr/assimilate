<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from '../components/BaseSpinner.vue'
import BorgPatternReference from '../components/BorgPatternReference.vue'

const { loading, error, run } = useAsyncAction()
const text = ref('')
const { loading: saving, error: saveError, run: runSave } = useAsyncAction()
const saveOk = ref(false)
const refOpen = ref(false)

async function loadData(): Promise<void> {
  await run(async () => {
    const res = await apiClient.get<{ raw_text: string }>('/excludes')
    text.value = res.data.raw_text
  })
}

async function save(): Promise<void> {
  saveOk.value = false
  await runSave(async () => {
    await apiClient.put('/excludes', { raw_text: text.value })
    saveOk.value = true
    setTimeout(() => {
      saveOk.value = false
    }, 2500)
  })
}

onMounted(loadData)
</script>

<template>
  <div class="excludes-view">
    <div class="page-header">
      <h1 class="page-title">Global Excludes</h1>
      <div class="header-actions">
        <button
          class="btn btn-ghost btn-sm"
          @click="refOpen = !refOpen"
        >
          {{ refOpen ? 'Close Reference' : 'Pattern Reference' }}
        </button>
      </div>
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="state-msg state-error"
    >
      {{ error }}
    </div>

    <div
      v-else
      class="layout"
      :class="{ 'layout-with-ref': refOpen }"
    >
      <div class="panels">
        <section class="panel panel--sectioned">
          <div class="panel-header">
            <span class="panel-title">Patterns</span>
            <span class="panel-hint">Applied to all schedules unless overridden</span>
          </div>
          <textarea
            v-model="text"
            class="input pattern-area"
            placeholder="One pattern per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;pp:__pycache__"
            spellcheck="false"
          />
          <div class="panel-footer">
            <span
              v-if="saveOk"
              class="save-ok"
              >Saved</span
            >
            <span
              v-if="saveError"
              class="save-err"
              >{{ saveError }}</span
            >
            <button
              class="btn btn-primary btn-sm"
              :disabled="saving"
              @click="save"
            >
              {{ saving ? 'Saving...' : 'Save' }}
            </button>
          </div>
        </section>
      </div>

      <BorgPatternReference
        v-if="refOpen"
        variant="sidebar"
      >
        <template #note>
          Schedules can override by setting "ignore global excludes" and defining their own
          patterns.
        </template>
      </BorgPatternReference>
    </div>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
}

/* Header carries a label plus inline controls, packed left rather than
   pushed to the edges. */
.panel .panel-header {
  justify-content: flex-start;
  gap: 0.75rem;
}

.excludes-view {
  max-width: 1200px;
  color: var(--text-primary);
}

.layout {
  display: flex;
  gap: 1.5rem;
  align-items: flex-start;
}

.panels {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
  min-width: 0;
}

.panel-hint {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-left: auto;
}

.pattern-area {
  background: var(--bg-input);
  border: none;
  color: var(--text-primary);
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.6;
  padding: 0.875rem 1.25rem;
  resize: vertical;
  min-height: 200px;
  width: 100%;
  box-sizing: border-box;
  outline: none;
}

.pattern-area::placeholder {
  color: var(--text-muted);
}

.panel-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 0.75rem;
  padding: 0.75rem 1.25rem;
  border-top: 1px solid var(--border-subtle);
  background: var(--bg-base);
}

.save-ok {
  font-size: var(--fs-sm);
  color: var(--success);
}

.save-err {
  font-size: var(--fs-sm);
  color: var(--danger);
  flex: 1;
}
</style>

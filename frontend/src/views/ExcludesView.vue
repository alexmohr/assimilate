<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { apiClient } from '../api/client'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from '../components/BaseSpinner.vue'

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
        <section class="panel">
          <div class="panel-header">
            <span class="panel-title">Patterns</span>
            <span class="panel-hint">Applied to all schedules unless overridden</span>
          </div>
          <textarea
            v-model="text"
            class="pattern-area"
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

      <aside
        v-if="refOpen"
        class="ref-panel"
      >
        <div class="ref-title">Borg Pattern Syntax</div>

        <div class="ref-section">
          <div class="ref-section-title">Shell Patterns (default)</div>
          <div class="ref-entry">
            <code>*.cache</code>
            <span>any file ending in .cache</span>
          </div>
          <div class="ref-entry">
            <code>home/*/Downloads</code>
            <span>Downloads in any home dir</span>
          </div>
          <div class="ref-entry">
            <code>*.{jpg,png}</code>
            <span>multiple extensions</span>
          </div>
        </div>

        <div class="ref-section">
          <div class="ref-section-title">Path Prefix <code>pp:</code></div>
          <div class="ref-entry">
            <code>pp:__pycache__</code>
            <span>any path component named __pycache__</span>
          </div>
          <div class="ref-entry">
            <code>pp:/proc</code>
            <span>exact path prefix /proc</span>
          </div>
        </div>

        <div class="ref-section">
          <div class="ref-section-title">Regex <code>re:</code></div>
          <div class="ref-entry">
            <code>re:\.git/objects/</code>
            <span>regex match anywhere in path</span>
          </div>
          <div class="ref-entry">
            <code>re:/tmp/[^/]+\.sock$</code>
            <span>socket files in /tmp</span>
          </div>
        </div>

        <div class="ref-section">
          <div class="ref-section-title">Fnmatch <code>fm:</code></div>
          <div class="ref-entry">
            <code>fm:*.log</code>
            <span>fnmatch pattern (case-sensitive)</span>
          </div>
        </div>

        <div class="ref-note">
          Schedules can override by setting "ignore global excludes" and defining their own
          patterns.
        </div>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.excludes-view {
  max-width: 1200px;
  color: var(--text-primary);
}

.state-msg {
  padding: 2rem;
  text-align: center;
  color: var(--text-muted);
}

.state-error {
  color: var(--danger);
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

.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.875rem 1.25rem;
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}

.panel-title {
  font-size: 0.8rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.panel-hint {
  font-size: 0.78rem;
  color: var(--text-muted);
  margin-left: auto;
}

.pattern-area {
  background: var(--bg-input);
  border: none;
  color: var(--text-primary);
  font-family: var(--mono);
  font-size: 0.82rem;
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
  font-size: 0.8rem;
  color: var(--success);
}

.save-err {
  font-size: 0.8rem;
  color: var(--danger);
  flex: 1;
}

.ref-panel {
  width: 280px;
  flex-shrink: 0;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
  position: sticky;
  top: 1rem;
}

.ref-title {
  font-size: 0.85rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  padding-bottom: 0.75rem;
  border-bottom: 1px solid var(--border);
}

.ref-section {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.ref-section-title {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.2rem;
}

.ref-section-title code {
  font-family: var(--mono);
  color: var(--accent);
  text-transform: none;
  letter-spacing: 0;
  background: transparent;
  padding: 0;
}

.ref-entry {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.ref-entry code {
  font-family: var(--mono);
  font-size: 0.8rem;
  color: var(--text-primary);
  background: var(--bg-base);
  padding: 0.15rem 0.4rem;
  border-radius: var(--radius-sm);
  display: inline-block;
}

.ref-entry span {
  font-size: 0.72rem;
  color: var(--text-muted);
  padding-left: 0.25rem;
}

.ref-note {
  font-size: 0.72rem;
  color: var(--text-muted);
  line-height: 1.5;
  padding-top: 0.5rem;
  border-top: 1px solid var(--border);
}
</style>

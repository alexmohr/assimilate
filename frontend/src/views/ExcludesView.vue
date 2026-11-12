<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, onMounted } from 'vue'
import { getExcludes, setExcludes } from '../api/excludes'
import { useAsyncAction } from '../composables/useAsyncAction'
import BaseSpinner from '../components/BaseSpinner.vue'
import BorgPatternReference from '../components/BorgPatternReference.vue'
import EditableSection from '../components/EditableSection.vue'
import { excludePatternWarnings } from '../utils/excludePatterns'

/**
 * The patterns every backup leaves out unless its schedule opts out of
 * them. Read as a list, like every other settings card, and edited as text
 * behind Edit, so a long list can still be pasted in one go and keep its
 * `#` comments.
 */
const { loading, error, run } = useAsyncAction()
const { loading: saving, error: saveError, run: runSave } = useAsyncAction()
/** The saved text, as the server has it. */
const text = ref('')
/** The text being edited. */
const draft = ref('')
const editing = ref(false)
const refOpen = ref(false)
const warnings = computed(() => excludePatternWarnings(draft.value))

/** One line of the saved text: a `#` comment, shown as a heading, or a pattern. */
interface PatternLine {
  kind: 'comment' | 'pattern'
  text: string
}

const lines = computed<PatternLine[]>(() =>
  text.value
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) =>
      line.startsWith('#')
        ? { kind: 'comment', text: line.replace(/^#+\s*/, '') }
        : { kind: 'pattern', text: line },
    ),
)

const patternCount = computed(() => lines.value.filter((l) => l.kind === 'pattern').length)

async function loadData(): Promise<void> {
  await run(async () => {
    const res = await getExcludes()
    text.value = res.raw_text
  })
}

function startEdit(): void {
  draft.value = text.value
  editing.value = true
}

async function save(): Promise<void> {
  await runSave(async () => {
    await setExcludes({ raw_text: draft.value })
    text.value = draft.value
    editing.value = false
  })
}

onMounted(loadData)
</script>

<template>
  <div class="excludes-view">
    <div class="page-header">
      <h1 class="page-title">Global Excludes</h1>
    </div>

    <BaseSpinner
      v-if="loading"
      size="lg"
    />
    <div
      v-else-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>

    <div
      v-else
      class="layout"
    >
      <section class="panel excludes-card">
        <EditableSection
          lede="Paths every backup leaves out, unless its schedule opts out of the global excludes."
          :editing="editing"
          can-edit
          :saving="saving"
          :error="saveError"
          @edit="startEdit"
          @cancel="editing = false"
          @save="save"
        >
          <template #view>
            <p
              v-if="patternCount === 0"
              class="muted"
            >
              No global excludes yet. Every backup reads all of its sources.
            </p>
            <ul
              v-else
              class="pattern-list"
              aria-label="Global exclude patterns"
            >
              <li
                v-for="(line, i) in lines"
                :key="i"
                :class="line.kind === 'comment' ? 'pattern-comment' : 'pattern-row mono'"
              >
                {{ line.text }}
              </li>
            </ul>
            <p class="pattern-foot">
              <span>{{ patternCount }} {{ patternCount === 1 ? 'pattern' : 'patterns' }}</span>
              <button
                type="button"
                class="pattern-ref-link"
                @click="refOpen = !refOpen"
              >
                {{ refOpen ? 'Close pattern reference' : 'Pattern reference' }}
              </button>
            </p>
          </template>
          <template #edit>
            <textarea
              v-model="draft"
              class="input pattern-area"
              aria-label="Global exclude patterns"
              placeholder="One pattern per line&#10;# Lines starting with # are comments&#10;e.g. *.cache&#10;*/__pycache__"
              spellcheck="false"
            />
            <ul
              v-if="warnings.length > 0"
              class="pattern-warnings"
              aria-label="Pattern warnings"
            >
              <li
                v-for="warning in warnings"
                :key="warning.line"
              >
                <span class="pattern-warning-line">Line {{ warning.line }}</span>
                {{ warning.message }}
              </li>
            </ul>
            <p class="pattern-foot">
              <span>One pattern per line; lines starting with # are comments.</span>
              <button
                type="button"
                class="pattern-ref-link"
                @click="refOpen = !refOpen"
              >
                {{ refOpen ? 'Close pattern reference' : 'Pattern reference' }}
              </button>
            </p>
          </template>
        </EditableSection>
      </section>

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
.excludes-view {
  max-width: 1200px;
  color: var(--text-primary);
}

.layout {
  display: flex;
  gap: var(--space-8);
  align-items: flex-start;
}

/* A settings card like the others: padded, its content inside. */
.excludes-card {
  flex: 1;
  min-width: 0;
  padding: var(--space-7) var(--space-8);
}

/* At rest the patterns read as a list split by hairlines; a # comment
   heads the patterns under it. */
.pattern-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
}

.pattern-row {
  padding: var(--space-3) 0;
  border-top: 1px solid var(--border-subtle);
  font-size: var(--fs-sm);
  overflow-wrap: anywhere;
}

.pattern-row:first-child,
.pattern-comment + .pattern-row {
  border-top: none;
}

.pattern-comment {
  padding: var(--space-6) 0 var(--space-2);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.pattern-comment:first-child {
  padding-top: 0;
}

.pattern-foot {
  display: flex;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-3) var(--space-6);
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.pattern-ref-link {
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  color: var(--accent);
  cursor: pointer;
}

.pattern-ref-link:hover {
  text-decoration: underline;
}

.pattern-area {
  font-family: var(--mono);
  font-size: var(--fs-sm);
  line-height: 1.6;
  resize: vertical;
  min-height: 240px;
}

.pattern-warnings {
  list-style: none;
  margin: 0;
  padding: var(--space-4) var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  font-size: var(--fs-sm);
  color: var(--warning);
  background: var(--warning-subtle);
  border-radius: var(--radius-sm);
}

.pattern-warning-line {
  font-weight: 600;
  margin-right: var(--space-3);
}

@media (max-width: 768px) {
  .layout {
    flex-direction: column;
  }

  .excludes-card {
    width: 100%;
    padding: var(--space-6);
  }
}
</style>

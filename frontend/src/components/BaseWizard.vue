<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts" generic="T extends string">
import { computed, ref, shallowRef, watch } from 'vue'
import { Check, ChevronLeft, ChevronRight, TriangleAlert } from '@lucide/vue'

/**
 * The shell every creation wizard uses (schedule, dependency, notification
 * channel): a rail of numbered steps beside the current step's pane, and a
 * foot that says what the step still needs, with Cancel, Back and Continue -
 * or the create action on the last step alone. On a phone the rail becomes a
 * row of numbered dots with the current step named under it.
 *
 * A step counts as answered once `blockers` returns nothing for it; Continue
 * waits for that, and the create action waits for every step. The caller owns
 * the form; this owns which step is shown.
 */
export interface WizardStep<V extends string> {
  id: V
  label: string
  /** A short line under the label in the rail. */
  sub?: string
}

const props = withDefaults(
  defineProps<{
    steps: readonly WizardStep<T>[]
    /** What a step still needs, as short phrases ("a name"); empty when it is answered. */
    blockers: (id: T) => string[]
    /** Names the rail for assistive tech, e.g. "Schedule steps". */
    label: string
    submitLabel: string
    submittingLabel?: string
    submitting?: boolean
  }>(),
  { submittingLabel: 'Creating...', submitting: false },
)

const emit = defineEmits<{ cancel: []; submit: [] }>()

defineSlots<{
  /**
   * The current step's pane; `goTo` jumps to another step, e.g. from a review
   * block, and `blocked` says whether any step still needs something (the
   * foot names what), so a review pane need not work that out again.
   */
  default: (props: { step: T; goTo: (id: T) => void; blocked: boolean }) => unknown
}>()

const stepIndex = ref(0)
const visited = shallowRef<Set<T>>(new Set())

const currentStep = computed<WizardStep<T>>(
  () => props.steps[Math.min(stepIndex.value, props.steps.length - 1)]!,
)
const isLastStep = computed(() => stepIndex.value >= props.steps.length - 1)

const currentBlockers = computed(() => props.blockers(currentStep.value.id))
const allBlockers = computed(() =>
  props.steps.flatMap((step) => props.blockers(step.id).map((text) => ({ step, text }))),
)

// The steps can change under the wizard (a schedule type that has no sources
// step); keep the index inside them.
watch(
  () => props.steps.length,
  (length) => {
    if (stepIndex.value > length - 1) stepIndex.value = length - 1
  },
)

type StepState = 'current' | 'done' | 'todo' | 'blank'

function stepState(step: WizardStep<T>, index: number): StepState {
  if (index === stepIndex.value) return 'current'
  if (props.blockers(step.id).length === 0) return 'done'
  return visited.value.has(step.id) ? 'todo' : 'blank'
}

function goToIndex(index: number): void {
  const left = currentStep.value.id
  stepIndex.value = Math.max(0, Math.min(index, props.steps.length - 1))
  // A new Set, so the rail re-renders on the visit itself, not only because
  // the step index happened to change in the same tick.
  visited.value = new Set([...visited.value, left, currentStep.value.id])
}

function goTo(id: T): void {
  const index = props.steps.findIndex((s) => s.id === id)
  if (index >= 0) goToIndex(index)
}

function submit(): void {
  if (allBlockers.value.length > 0 || props.submitting) return
  emit('submit')
}
</script>

<template>
  <div class="panel wizard">
    <nav
      class="wizard-rail"
      :aria-label="label"
    >
      <button
        v-for="(step, index) in steps"
        :key="step.id"
        type="button"
        class="wizard-step"
        :class="{ 'wizard-step--current': index === stepIndex }"
        :aria-current="index === stepIndex"
        @click="goToIndex(index)"
      >
        <span
          class="wizard-num"
          :class="`wizard-num--${stepState(step, index)}`"
        >
          <Check
            v-if="stepState(step, index) === 'done'"
            :size="12"
          />
          <template v-else>{{ index + 1 }}</template>
        </span>
        <span class="wizard-step-text">
          <span>{{ step.label }}</span>
          <span
            v-if="step.sub"
            class="wizard-step-sub"
            >{{ step.sub }}</span
          >
        </span>
      </button>
    </nav>
    <p class="wizard-caption">
      Step {{ stepIndex + 1 }} of {{ steps.length }} · {{ currentStep.label }}
    </p>

    <section class="wizard-pane">
      <slot
        :step="currentStep.id"
        :go-to="goTo"
        :blocked="allBlockers.length > 0"
      />
    </section>

    <div class="wizard-foot">
      <span
        v-if="isLastStep && allBlockers.length > 0"
        class="wizard-status wizard-status--blocked"
      >
        <TriangleAlert :size="14" />
        Still needs {{ allBlockers.map((b) => `${b.text} (${b.step.label})`).join(', ') }}.
      </span>
      <span
        v-else-if="currentBlockers.length > 0"
        class="wizard-status wizard-status--blocked"
      >
        <TriangleAlert :size="14" />
        Needs {{ currentBlockers.join(' and ') }} to continue.
      </span>
      <span
        v-else
        class="wizard-status"
      >
        Step {{ stepIndex + 1 }} of {{ steps.length }}.
      </span>

      <div class="wizard-actions">
        <button
          type="button"
          class="btn btn-ghost"
          @click="emit('cancel')"
        >
          Cancel
        </button>
        <button
          type="button"
          class="btn btn-ghost"
          :disabled="stepIndex === 0"
          @click="goToIndex(stepIndex - 1)"
        >
          <ChevronLeft :size="14" />
          Back
        </button>
        <button
          v-if="!isLastStep"
          type="button"
          class="btn btn-primary"
          :disabled="currentBlockers.length > 0"
          @click="goToIndex(stepIndex + 1)"
        >
          Continue
          <ChevronRight :size="14" />
        </button>
        <button
          v-else
          type="button"
          class="btn btn-primary"
          :disabled="allBlockers.length > 0 || submitting"
          @click="submit"
        >
          {{ submitting ? submittingLabel : submitLabel }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wizard {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  padding: 0;
  overflow: hidden;
}

.wizard-rail {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  padding: var(--space-6) var(--space-5);
  border-right: 1px solid var(--border);
}

/* Only on a phone, where the rail shows numbers alone. */
.wizard-caption {
  display: none;
}

.wizard-step {
  display: grid;
  grid-template-columns: 1.375rem minmax(0, 1fr);
  align-items: center;
  gap: var(--space-5);
  width: 100%;
  padding: var(--space-4);
  border: none;
  background: transparent;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-base);
}

.wizard-step:hover {
  background: var(--bg-hover);
}

.wizard-step--current {
  background: var(--accent-subtle);
  color: var(--accent);
  font-weight: 600;
}

.wizard-num {
  display: grid;
  place-items: center;
  width: 1.375rem;
  height: 1.375rem;
  border-radius: 50%;
  border: 1px solid var(--border);
  background: var(--bg-input);
  color: var(--text-muted);
  font-size: var(--fs-2xs);
  font-weight: 700;
}

.wizard-num--current {
  border-color: var(--accent);
  color: var(--accent);
}

.wizard-num--done {
  border-color: var(--success);
  background: var(--success);
  color: var(--text-on-accent);
}

.wizard-num--todo {
  border-color: var(--warning);
  color: var(--warning);
}

.wizard-step-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  font-size: var(--fs-sm);
}

.wizard-step-sub {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-weight: 400;
}

.wizard-pane {
  display: flex;
  flex-direction: column;
  gap: var(--space-7);
  padding: var(--space-8);
  min-width: 0;
}

.wizard-foot {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-6);
  padding: var(--space-6) var(--space-8);
  border-top: 1px solid var(--border);
}

.wizard-status {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.wizard-status--blocked {
  color: var(--warning);
}

.wizard-actions {
  display: flex;
  gap: var(--space-4);
  margin-left: auto;
}

@media (max-width: 768px) {
  .wizard {
    grid-template-columns: minmax(0, 1fr);
  }

  /* A row of numbered dots spread across the width; the current step's
     name is the caption under it. */
  .wizard-rail {
    flex-direction: row;
    justify-content: space-between;
    padding: var(--space-5);
    border-right: none;
  }

  .wizard-step {
    display: flex;
    justify-content: center;
    width: auto;
    padding: var(--space-2);
  }

  .wizard-step-text {
    display: none;
  }

  .wizard-caption {
    display: block;
    margin: 0;
    padding: 0 var(--space-6) var(--space-5);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .wizard-pane {
    padding: var(--space-6);
  }
}
</style>

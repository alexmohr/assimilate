<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { cronToHuman, CRON_ANY, CRON_TOP_OF_HOUR } from '../utils/cron'
import { getConfiguredTimezone } from '../composables/useTimezone'
import { logger } from '../utils/logger'
import { validateCron as validateCronInWasm } from '../wasm/domain'
import { nextCronRuns } from '../wasm/timezones'

type Frequency = 'hourly' | 'daily' | 'weekly' | 'monthly'

const props = defineProps<{
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const showHelper = ref(false)
const frequency = ref<Frequency>('daily')
const hourlyInterval = ref(6)
const timeHour = ref(2)
const timeMinute = ref(0)
const selectedDays = ref<number[]>([1])
const monthDay = ref(1)
const validationError = ref<string | null>(null)

const dayLabels: { value: number; short: string }[] = [
  { value: 1, short: 'Mon' },
  { value: 2, short: 'Tue' },
  { value: 3, short: 'Wed' },
  { value: 4, short: 'Thu' },
  { value: 5, short: 'Fri' },
  { value: 6, short: 'Sat' },
  { value: 0, short: 'Sun' },
]

const hourOptions = computed((): number[] => {
  const opts: number[] = []
  for (let i = 0; i < 24; i++) {
    opts.push(i)
  }
  return opts
})

const minuteOptions = computed((): number[] => {
  const opts: number[] = []
  for (let i = 0; i < 60; i += 5) {
    opts.push(i)
  }
  return opts
})

function pad(n: number): string {
  return n.toString().padStart(2, '0')
}

const helperExpression = computed((): string => {
  switch (frequency.value) {
    case 'hourly':
      return `0 */${hourlyInterval.value} * * *`
    case 'daily':
      return `${timeMinute.value} ${timeHour.value} * * *`
    case 'weekly': {
      const days = selectedDays.value.length > 0 ? selectedDays.value.join(',') : '1'
      return `${timeMinute.value} ${timeHour.value} * * ${days}`
    }
    case 'monthly':
      return `${timeMinute.value} ${timeHour.value} ${monthDay.value} * *`
    default:
      return `${timeMinute.value} ${timeHour.value} * * *`
  }
})

function parseExpressionToHelper(expr: string): boolean {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return false

  const [min, hour, dom, month, dow] = parts

  if (month !== CRON_ANY) return false

  const hourlyMatch = hour.match(/^\*\/(\d+)$/)
  if (hourlyMatch && min === CRON_TOP_OF_HOUR && dom === CRON_ANY && dow === CRON_ANY) {
    frequency.value = 'hourly'
    hourlyInterval.value = parseInt(hourlyMatch[1], 10)
    return true
  }

  const minNum = parseInt(min, 10)
  const hourNum = parseInt(hour, 10)
  if (isNaN(minNum) || isNaN(hourNum)) return false
  if (minNum < 0 || minNum > 59 || hourNum < 0 || hourNum > 23) return false

  timeMinute.value = minNum
  timeHour.value = hourNum

  if (dom === CRON_ANY && dow === CRON_ANY) {
    frequency.value = 'daily'
    return true
  }

  if (dom === CRON_ANY && dow !== CRON_ANY) {
    const dayParts = dow.split(',')
    const days = dayParts.map((d) => parseInt(d, 10))
    if (days.some((d) => isNaN(d) || d < 0 || d > 7)) return false
    frequency.value = 'weekly'
    selectedDays.value = days
    return true
  }

  if (dow === CRON_ANY && dom !== CRON_ANY) {
    const domNum = parseInt(dom, 10)
    if (isNaN(domNum) || domNum < 1 || domNum > 31) return false
    frequency.value = 'monthly'
    monthDay.value = domNum
    return true
  }

  return false
}

// The server's own validator (`domain::schedule::validate_cron`), so the form
// accepts exactly what saving the schedule accepts - including month and
// weekday names like `MON-FRI`.
function validateCron(expr: string): string | null {
  return validateCronInWasm(expr) ?? null
}

const nextRuns = ref<string[]>([])
let nextRunsRequest = 0

// Computed by the scheduler's `next_runs` in the configured timezone, so the
// preview resolves DST gaps and repeats exactly as the schedule will fire.
watch(
  () => props.modelValue,
  async (expr) => {
    nextRunsRequest += 1
    const request = nextRunsRequest
    if (validateCron(expr)) {
      nextRuns.value = []
      return
    }
    try {
      const runs = await nextCronRuns(expr, new Date(), getConfiguredTimezone() ?? 'UTC', 3)
      if (request === nextRunsRequest) nextRuns.value = runs.map(formatRunDate)
    } catch (e: unknown) {
      logger.debug('next run preview failed', e)
      if (request === nextRunsRequest) nextRuns.value = []
    }
  },
  { immediate: true },
)

function formatRunDate(date: Date): string {
  return new Intl.DateTimeFormat(undefined, {
    timeZone: getConfiguredTimezone(),
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date)
}

const humanDescription = computed((): string => cronToHuman(props.modelValue))

function toggleDay(day: number): void {
  const idx = selectedDays.value.indexOf(day)
  if (idx >= 0) {
    if (selectedDays.value.length > 1) {
      selectedDays.value.splice(idx, 1)
    }
  } else {
    selectedDays.value.push(day)
  }
}

function applyHelper(): void {
  emit('update:modelValue', helperExpression.value)
}

function onInput(event: Event): void {
  const value = (event.target as HTMLInputElement).value
  validationError.value = validateCron(value)
  emit('update:modelValue', value)
}

function toggleHelper(): void {
  if (!showHelper.value) {
    parseExpressionToHelper(props.modelValue)
  }
  showHelper.value = !showHelper.value
}

watch(
  () => props.modelValue,
  (val) => {
    validationError.value = validateCron(val)
  },
  { immediate: true },
)
</script>

<template>
  <div class="cron-builder">
    <div class="cron-input-row">
      <input
        type="text"
        class="input cron-input"
        :value="modelValue"
        placeholder="0 2 * * *"
        spellcheck="false"
        @input="onInput"
      />
      <button
        type="button"
        class="helper-toggle"
        :class="{ active: showHelper }"
        @click="toggleHelper"
      >
        {{ showHelper ? 'Hide Helper' : 'Helper' }}
      </button>
    </div>

    <span class="cron-hint">5-field cron: minute hour day-of-month month day-of-week</span>

    <span
      v-if="validationError"
      class="cron-error"
    >
      {{ validationError }}
    </span>

    <!-- Visual Helper -->
    <div
      v-if="showHelper"
      class="helper-panel"
    >
      <div class="helper-row">
        <label class="helper-label">Frequency</label>
        <select
          v-model="frequency"
          class="input helper-select"
        >
          <option value="hourly">Hourly</option>
          <option value="daily">Daily</option>
          <option value="weekly">Weekly</option>
          <option value="monthly">Monthly</option>
        </select>
      </div>

      <div
        v-if="frequency === 'hourly'"
        class="helper-row"
      >
        <label class="helper-label">Every</label>
        <input
          v-model.number="hourlyInterval"
          type="number"
          min="1"
          max="23"
          class="input helper-input helper-input-narrow"
        />
        <span class="helper-suffix">hours</span>
      </div>

      <template v-if="frequency !== 'hourly'">
        <div class="helper-row">
          <label class="helper-label">Time</label>
          <div class="time-picker">
            <select
              v-model.number="timeHour"
              class="input helper-select time-select"
            >
              <option
                v-for="h in hourOptions"
                :key="h"
                :value="h"
              >
                {{ pad(h) }}
              </option>
            </select>
            <span class="time-sep">:</span>
            <select
              v-model.number="timeMinute"
              class="input helper-select time-select"
            >
              <option
                v-for="m in minuteOptions"
                :key="m"
                :value="m"
              >
                {{ pad(m) }}
              </option>
            </select>
          </div>
        </div>
      </template>

      <div
        v-if="frequency === 'weekly'"
        class="helper-row"
      >
        <label class="helper-label">Days</label>
        <div class="day-picker">
          <button
            v-for="d in dayLabels"
            :key="d.value"
            type="button"
            class="day-btn"
            :class="{ selected: selectedDays.includes(d.value) }"
            @click="toggleDay(d.value)"
          >
            {{ d.short }}
          </button>
        </div>
      </div>

      <div
        v-if="frequency === 'monthly'"
        class="helper-row"
      >
        <label class="helper-label">Day</label>
        <input
          v-model.number="monthDay"
          type="number"
          min="1"
          max="31"
          class="input helper-input helper-input-narrow"
        />
      </div>

      <div class="helper-apply-row">
        <code class="helper-preview">{{ helperExpression }}</code>
        <button
          type="button"
          class="helper-apply-btn"
          @click="applyHelper"
        >
          Apply
        </button>
      </div>
    </div>

    <!-- Preview -->
    <div
      v-if="!validationError && modelValue.trim()"
      class="cron-preview"
    >
      <span
        v-if="humanDescription"
        class="cron-description"
      >
        {{ humanDescription }}
      </span>
      <div
        v-if="nextRuns.length > 0"
        class="next-runs"
      >
        <span class="next-runs-label">Next:</span>
        <span
          v-for="(run, i) in nextRuns"
          :key="i"
          class="next-run"
        >
          {{ run }}<template v-if="i < nextRuns.length - 1">,</template>
        </span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cron-builder {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.cron-input-row {
  display: flex;
  gap: var(--space-4);
  align-items: center;
}

.cron-input {
  flex: 1;
  font-family: var(--mono);
  font-size: var(--fs-base);
  letter-spacing: 0.03em;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: var(--space-4) var(--space-5);
  box-sizing: border-box;
  transition: border-color var(--duration-base);
}

.cron-input:focus {
  outline: none;
  border-color: var(--accent);
}

.helper-toggle {
  padding: var(--space-4) var(--space-5);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--fs-sm);
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  transition:
    background var(--duration-base),
    border-color var(--duration-base),
    color var(--duration-base);
}

.helper-toggle:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.helper-toggle.active {
  background: var(--bg-hover);
  border-color: var(--accent);
  color: var(--accent);
}

.cron-hint {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.cron-error {
  font-size: var(--fs-xs);
  color: var(--danger);
}

.helper-panel {
  margin-top: var(--space-3);
  padding: var(--space-5);
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.helper-row {
  display: flex;
  align-items: center;
  gap: var(--space-4);
}

.helper-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  min-width: 5rem;
  flex-shrink: 0;
}

.helper-select {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: var(--space-3) var(--space-4);
  font-size: var(--fs-sm);
}

.helper-select:focus {
  outline: none;
  border-color: var(--accent);
}

.helper-input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: var(--space-3) var(--space-4);
  font-size: var(--fs-sm);
}

.helper-input:focus {
  outline: none;
  border-color: var(--accent);
}

.helper-input-narrow {
  width: 4rem;
}

.helper-suffix {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.time-picker {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.time-select {
  width: auto;
  min-width: 3.5rem;
}

.time-sep {
  font-weight: 700;
  color: var(--text-muted);
}

.day-picker {
  display: flex;
  gap: var(--space-2);
  flex-wrap: wrap;
}

.day-btn {
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
  transition:
    background var(--duration-base),
    border-color var(--duration-base),
    color var(--duration-base);
}

.day-btn.selected {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}

.day-btn:not(.selected):hover {
  border-color: var(--accent);
  color: var(--accent);
}

.helper-apply-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-top: var(--space-4);
  border-top: 1px solid var(--border);
}

.helper-preview {
  font-family: var(--mono);
  font-size: var(--fs-sm);
  color: var(--accent);
  background: transparent;
}

.helper-apply-btn {
  padding: var(--space-2) var(--space-5);
  border: none;
  border-radius: var(--radius-sm);
  background: var(--accent);
  color: #fff;
  font-size: var(--fs-xs);
  font-weight: 600;
  cursor: pointer;
  transition: background var(--duration-base);
}

.helper-apply-btn:hover {
  background: var(--accent-hover);
}

.cron-preview {
  margin-top: var(--space-2);
  padding: var(--space-3) var(--space-5);
  background: var(--bg-input);
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-subtle);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.cron-description {
  font-size: var(--fs-xs);
  color: var(--text-primary);
  font-weight: 600;
}

.next-runs {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.next-runs-label {
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.next-run {
  color: var(--text-secondary);
}
</style>

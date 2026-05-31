<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'

interface CalendarEvent {
  type: string
  status: string
  repo_name: string
  time: string
  report_id?: number
  repo_id?: number
  schedule_id?: number
  archive_name?: string
  error_message?: string
}

interface CalendarDay {
  date: string
  events: CalendarEvent[]
}

interface RepoOption {
  id: number
  name: string
}

const props = defineProps<{
  repos: RepoOption[]
}>()

const selectedRepoId = ref<number | undefined>(undefined)
const currentYear = ref(new Date().getFullYear())
const currentMonth = ref(new Date().getMonth() + 1)
const calendarDays = ref<CalendarDay[]>([])
const loading = ref(true)
const selectedDay = ref<CalendarDay | null>(null)

const monthLabel = computed((): string => {
  const d = new Date(currentYear.value, currentMonth.value - 1, 1)
  return d.toLocaleString('default', { month: 'long', year: 'numeric' })
})

const monthStr = computed(
  (): string => `${currentYear.value}-${String(currentMonth.value).padStart(2, '0')}`,
)

function prevMonth(): void {
  if (currentMonth.value === 1) {
    currentMonth.value = 12
    currentYear.value -= 1
  } else {
    currentMonth.value -= 1
  }
}

function nextMonth(): void {
  if (currentMonth.value === 12) {
    currentMonth.value = 1
    currentYear.value += 1
  } else {
    currentMonth.value += 1
  }
}

async function fetchCalendar(): Promise<void> {
  loading.value = true
  try {
    const params = new URLSearchParams({ month: monthStr.value })
    if (selectedRepoId.value !== undefined) {
      params.set('repo_id', String(selectedRepoId.value))
    }
    const response = await apiClient.get<CalendarDay[]>(`/stats/calendar?${params.toString()}`)
    calendarDays.value = response.data
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchCalendar().catch(logger.error)
})

watch([currentYear, currentMonth, selectedRepoId], () => {
  selectedDay.value = null
  fetchCalendar().catch(logger.error)
})

interface GridCell {
  day: number
  date: string
  inMonth: boolean
  events: CalendarEvent[]
}

const calendarGrid = computed((): GridCell[][] => {
  const firstDay = new Date(currentYear.value, currentMonth.value - 1, 1)
  const startDow = firstDay.getDay()
  const daysInMonth = new Date(currentYear.value, currentMonth.value, 0).getDate()

  const eventMap = new Map<string, CalendarEvent[]>()
  for (const day of calendarDays.value) {
    eventMap.set(day.date, day.events)
  }

  const cells: GridCell[] = []
  for (let i = 0; i < startDow; i++) {
    cells.push({ day: 0, date: '', inMonth: false, events: [] })
  }
  for (let d = 1; d <= daysInMonth; d++) {
    const date = `${currentYear.value}-${String(currentMonth.value).padStart(2, '0')}-${String(d).padStart(2, '0')}`
    cells.push({ day: d, date, inMonth: true, events: eventMap.get(date) ?? [] })
  }
  while (cells.length % 7 !== 0) {
    cells.push({ day: 0, date: '', inMonth: false, events: [] })
  }

  const weeks: GridCell[][] = []
  for (let i = 0; i < cells.length; i += 7) {
    weeks.push(cells.slice(i, i + 7))
  }
  return weeks
})

function selectDay(cell: GridCell): void {
  if (!cell.inMonth || cell.events.length === 0) {
    selectedDay.value = null
    return
  }
  selectedDay.value = { date: cell.date, events: cell.events }
}

function eventColor(status: string): string {
  if (status === 'success') return 'var(--success)'
  if (status === 'failed') return 'var(--danger)'
  if (status === 'warning') return 'var(--warning)'
  return 'var(--info)'
}

const router = useRouter()
const errorPopup = ref<{
  repo_name: string
  repo_id?: number
  schedule_id?: number
  schedule_name?: string
  time: string
  message: string
} | null>(null)

function onEventClick(evt: CalendarEvent): void {
  if (evt.status === 'success' && evt.repo_id) {
    const query: Record<string, string> = { tab: 'archives' }
    if (evt.archive_name) {
      query.archive = evt.archive_name
    }
    router.push({ name: 'repo-detail', params: { id: String(evt.repo_id) }, query })
  } else if (evt.status === 'failed') {
    errorPopup.value = {
      repo_name: evt.repo_name,
      repo_id: evt.repo_id,
      schedule_id: evt.schedule_id,
      time: evt.time,
      message: evt.error_message ?? 'No error details available.',
    }
  } else if (evt.status === 'warning') {
    errorPopup.value = {
      repo_name: evt.repo_name,
      repo_id: evt.repo_id,
      schedule_id: evt.schedule_id,
      time: evt.time,
      message: evt.error_message ?? 'No warning details available.',
    }
  } else if (evt.status === 'scheduled' && evt.schedule_id) {
    router.push(`/schedules/${evt.schedule_id}`)
  }
}

function closeErrorPopup(): void {
  errorPopup.value = null
}
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Backup Calendar</h2>
      <select
        v-model="selectedRepoId"
        class="cal-select"
      >
        <option :value="undefined">All Repos</option>
        <option
          v-for="repo in props.repos"
          :key="repo.id"
          :value="repo.id"
        >
          {{ repo.name }}
        </option>
      </select>
    </div>
    <div class="cal-nav">
      <button
        class="cal-nav-btn"
        @click="prevMonth"
      >
        &larr;
      </button>
      <span class="cal-month-label">{{ monthLabel }}</span>
      <button
        class="cal-nav-btn"
        @click="nextMonth"
      >
        &rarr;
      </button>
    </div>
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading…
    </div>
    <template v-else>
      <div class="cal-grid">
        <div class="cal-header-cell">Sun</div>
        <div class="cal-header-cell">Mon</div>
        <div class="cal-header-cell">Tue</div>
        <div class="cal-header-cell">Wed</div>
        <div class="cal-header-cell">Thu</div>
        <div class="cal-header-cell">Fri</div>
        <div class="cal-header-cell">Sat</div>
        <template
          v-for="(week, wi) in calendarGrid"
          :key="wi"
        >
          <div
            v-for="cell in week"
            :key="cell.date || `empty-${wi}-${cell.day}`"
            class="cal-cell"
            :class="{
              'cal-cell-active': cell.inMonth,
              'cal-cell-has-events': cell.events.length > 0,
              'cal-cell-selected': selectedDay?.date === cell.date,
            }"
            @click="selectDay(cell)"
          >
            <span
              v-if="cell.inMonth"
              class="cal-day-num"
            >
              {{ cell.day }}
            </span>
            <div
              v-if="cell.events.length > 0"
              class="cal-dots"
            >
              <span
                v-for="(evt, ei) in cell.events.slice(0, 4)"
                :key="ei"
                class="cal-dot"
                :style="{ background: eventColor(evt.status) }"
              />
              <span
                v-if="cell.events.length > 4"
                class="cal-dot-more"
              >
                +{{ cell.events.length - 4 }}
              </span>
            </div>
          </div>
        </template>
      </div>
      <div
        v-if="selectedDay"
        class="cal-detail"
      >
        <h3 class="cal-detail-title">{{ selectedDay.date }}</h3>
        <div
          v-for="(evt, i) in selectedDay.events"
          :key="i"
          class="cal-event"
          :class="{
            'cal-event-clickable':
              evt.status === 'success' ||
              evt.status === 'failed' ||
              evt.status === 'warning' ||
              evt.status === 'scheduled',
          }"
          @click="onEventClick(evt)"
        >
          <span
            class="cal-event-dot"
            :style="{ background: eventColor(evt.status) }"
          />
          <span class="cal-event-time">{{ evt.time }}</span>
          <a
            v-if="evt.repo_id"
            class="cal-event-repo cal-event-repo-link"
            @click.stop="router.push(`/repos/${evt.repo_id}`)"
          >
            {{ evt.repo_name }}
          </a>
          <span
            v-else
            class="cal-event-repo"
          >
            {{ evt.repo_name }}
          </span>
          <span
            class="cal-event-badge"
            :class="`cal-badge-${evt.status}`"
          >
            {{ evt.status }}
          </span>
        </div>
      </div>
    </template>
    <div
      v-if="errorPopup"
      class="cal-error-overlay"
      @click="closeErrorPopup"
    >
      <div
        class="cal-error-popup"
        @click.stop
      >
        <div class="cal-error-header">
          <span class="cal-error-title">Backup Failed</span>
          <button
            class="cal-error-close"
            @click="closeErrorPopup"
          >
            &times;
          </button>
        </div>
        <div class="cal-error-meta">
          <a
            v-if="errorPopup.repo_id"
            class="cal-error-link"
            @click="router.push(`/repos/${errorPopup.repo_id}`); closeErrorPopup()"
          >
            {{ errorPopup.repo_name }}
          </a>
          <span v-else>{{ errorPopup.repo_name }}</span>
          at {{ errorPopup.time }}
          <template v-if="errorPopup.schedule_id">
            &middot;
            <a
              class="cal-error-link"
              @click="router.push(`/schedules/${errorPopup.schedule_id}`); closeErrorPopup()"
            >
              Schedule
            </a>
          </template>
        </div>
        <pre class="cal-error-msg">{{ errorPopup.message }}</pre>
      </div>
    </div>
  </section>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}

.panel-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin: 0;
}

.cal-select {
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
}

.cal-nav {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1rem;
  margin-bottom: 0.75rem;
}

.cal-nav-btn {
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  cursor: pointer;
  padding: 0.2rem 0.5rem;
  font-size: 0.85rem;
}

.cal-nav-btn:hover {
  background: var(--bg-hover);
}

.cal-month-label {
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--text-primary);
}

.cal-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 1px;
}

.cal-header-cell {
  font-size: 0.6rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
  text-align: center;
  padding: 0.3rem 0;
}

.cal-cell {
  aspect-ratio: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.15rem;
  border-radius: var(--radius-sm);
  padding: 0.2rem;
  min-height: 2.5rem;
}

.cal-cell-active {
  cursor: default;
}

.cal-cell-has-events {
  cursor: pointer;
}

.cal-cell-has-events:hover {
  background: var(--bg-hover);
}

.cal-cell-selected {
  background: var(--bg-hover);
  border: 1px solid var(--accent);
}

.cal-day-num {
  font-size: 0.75rem;
  color: var(--text-primary);
}

.cal-dots {
  display: flex;
  gap: 2px;
  align-items: center;
}

.cal-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
}

.cal-dot-more {
  font-size: 0.5rem;
  color: var(--text-muted);
}

.cal-detail {
  margin-top: 1rem;
  border-top: 1px solid var(--border);
  padding-top: 0.75rem;
}

.cal-detail-title {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 0.5rem;
}

.cal-event {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.3rem 0;
  font-size: 0.75rem;
}

.cal-event-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.cal-event-time {
  color: var(--text-muted);
  font-family: var(--mono);
  min-width: 3rem;
}

.cal-event-repo {
  color: var(--text-primary);
  font-weight: 500;
  flex: 1;
}

.cal-event-repo-link {
  cursor: pointer;
  text-decoration: none;
}

.cal-event-repo-link:hover {
  color: var(--accent);
  text-decoration: underline;
}

.cal-error-link {
  color: var(--accent);
  cursor: pointer;
  text-decoration: none;
}

.cal-error-link:hover {
  text-decoration: underline;
}

.cal-event-badge {
  font-size: 0.6rem;
  font-weight: 700;
  text-transform: uppercase;
  padding: 0.1rem 0.35rem;
  border-radius: 0.2rem;
}

.cal-badge-success {
  background: var(--success-subtle);
  color: var(--success);
}

.cal-badge-failed {
  background: var(--danger-subtle);
  color: var(--danger);
}

.cal-badge-scheduled {
  background: var(--info-subtle);
  color: var(--info);
}

.state-msg {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 1rem 0;
}

.cal-event-clickable {
  cursor: pointer;
  border-radius: var(--radius-sm);
  padding-left: 0.3rem;
  padding-right: 0.3rem;
}

.cal-event-clickable:hover {
  background: var(--bg-hover);
}

.cal-error-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.cal-error-popup {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  max-width: 32rem;
  width: 90%;
  max-height: 60vh;
  overflow: auto;
}

.cal-error-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.cal-error-title {
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--danger);
}

.cal-error-close {
  background: transparent;
  border: none;
  font-size: 1.25rem;
  cursor: pointer;
  color: var(--text-muted);
  line-height: 1;
}

.cal-error-meta {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-bottom: 0.75rem;
}

.cal-error-msg {
  font-family: var(--mono);
  font-size: 0.75rem;
  color: var(--text-primary);
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>

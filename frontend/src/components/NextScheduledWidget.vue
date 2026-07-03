<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { relativeTime } from '../utils/format'
import { logger } from '../utils/logger'

interface CalendarEvent {
  type: string
  status: string
  repo_name: string
  time: string
  schedule_id?: number
}

interface CalendarDay {
  date: string
  events: CalendarEvent[]
}

function isScheduledEvent(status: string): status is 'scheduled' {
  return status === 'scheduled'
}

interface ScheduledItem {
  repo_name: string
  datetime: string
  iso: string
  schedule_id?: number
}

const items = ref<ScheduledItem[]>([])
const loading = ref(true)
const router = useRouter()
const now = ref(Date.now())
let refreshTimer: ReturnType<typeof setInterval> | null = null

async function fetchScheduled(): Promise<void> {
  try {
    const currentNow = new Date()
    const month1 = `${currentNow.getFullYear()}-${String(currentNow.getMonth() + 1).padStart(2, '0')}`
    const nextMonth = new Date(currentNow.getFullYear(), currentNow.getMonth() + 1, 1)
    const month2 = `${nextMonth.getFullYear()}-${String(nextMonth.getMonth() + 1).padStart(2, '0')}`

    const [r1, r2] = await Promise.all([
      apiClient.get<CalendarDay[]>(`/stats/calendar?month=${month1}`),
      apiClient.get<CalendarDay[]>(`/stats/calendar?month=${month2}`),
    ])

    const nowTs = currentNow.getTime()
    const all: ScheduledItem[] = []
    for (const day of [...r1.data, ...r2.data]) {
      for (const evt of day.events) {
        if (isScheduledEvent(evt.status)) {
          const iso = `${day.date}T${evt.time}:00Z`
          const evtTs = new Date(iso).getTime()
          if (evtTs > nowTs) {
            all.push({
              repo_name: evt.repo_name,
              datetime: `${day.date} ${evt.time}`,
              iso,
              schedule_id: evt.schedule_id,
            })
          }
        }
      }
    }
    all.sort((a, b) => a.iso.localeCompare(b.iso))
    items.value = all.slice(0, 5)
    now.value = Date.now()
  } finally {
    loading.value = false
  }
}

function navigateToSchedule(item: ScheduledItem): void {
  if (item.schedule_id) {
    router.push(`/schedules/${item.schedule_id}`)
  } else {
    router.push({ name: 'schedules', query: { repo: item.repo_name } })
  }
}

function liveRelativeTime(iso: string): string {
  void now.value
  return relativeTime(iso)
}

onMounted(() => {
  fetchScheduled().catch(logger.error)
  refreshTimer = setInterval(() => {
    now.value = Date.now()
    fetchScheduled().catch(logger.error)
  }, 30_000)
})

onUnmounted(() => {
  if (refreshTimer) clearInterval(refreshTimer)
})
</script>

<template>
  <section class="panel">
    <h2 class="panel-title">Next Scheduled</h2>
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading…
    </div>
    <div
      v-else-if="items.length === 0"
      class="state-msg"
    >
      No upcoming backups.
    </div>
    <div
      v-else
      class="scheduled-list"
    >
      <div
        v-for="(item, i) in items"
        :key="i"
        class="scheduled-item"
        @click="navigateToSchedule(item)"
      >
        <span class="scheduled-icon">⏱</span>
        <div class="scheduled-info">
          <span class="scheduled-repo">{{ item.repo_name }}</span>
          <span class="scheduled-time">{{ item.datetime }}</span>
        </div>
        <span class="scheduled-countdown">{{ liveRelativeTime(item.iso) }}</span>
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

.panel-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin: 0 0 0.75rem;
}

.state-msg {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 0.5rem 0;
}

.scheduled-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.scheduled-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s;
}

.scheduled-item:hover {
  background: var(--bg-hover);
}

.scheduled-icon {
  font-size: 0.75rem;
  flex-shrink: 0;
}

.scheduled-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.scheduled-repo {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scheduled-time {
  font-size: 0.7rem;
  color: var(--text-muted);
  font-family: var(--mono);
}

.scheduled-countdown {
  font-size: 0.7rem;
  color: var(--text-muted);
  flex-shrink: 0;
}
</style>

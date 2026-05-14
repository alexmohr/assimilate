<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from 'vue'

const props = defineProps<{
  modelValue: string
  placeholder?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const allTimezones: string[] = Intl.supportedValuesOf('timeZone')

const searchQuery = ref('')
const isOpen = ref(false)
const highlightedIndex = ref(0)
const wrapperRef = ref<HTMLElement | null>(null)
const inputRef = ref<HTMLInputElement | null>(null)
const listRef = ref<HTMLElement | null>(null)

const filtered = computed((): string[] => {
  if (!searchQuery.value) {
    return allTimezones
  }
  const q = searchQuery.value.toLowerCase()
  return allTimezones.filter((tz) => tz.toLowerCase().includes(q))
})

function open(): void {
  isOpen.value = true
  searchQuery.value = ''
  highlightedIndex.value = 0
}

function close(): void {
  isOpen.value = false
  searchQuery.value = ''
}

function select(tz: string): void {
  emit('update:modelValue', tz)
  close()
}

function onInputFocus(): void {
  open()
}

function onInput(): void {
  isOpen.value = true
  highlightedIndex.value = 0
}

function onKeydown(e: KeyboardEvent): void {
  if (!isOpen.value) {
    if (e.key === 'ArrowDown' || e.key === 'Enter') {
      open()
      e.preventDefault()
    }
    return
  }

  if (e.key === 'ArrowDown') {
    e.preventDefault()
    highlightedIndex.value = Math.min(highlightedIndex.value + 1, filtered.value.length - 1)
    scrollToHighlighted()
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    highlightedIndex.value = Math.max(highlightedIndex.value - 1, 0)
    scrollToHighlighted()
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const item = filtered.value[highlightedIndex.value]
    if (item) {
      select(item)
    }
  } else if (e.key === 'Escape') {
    close()
    inputRef.value?.blur()
  }
}

function scrollToHighlighted(): void {
  nextTick(() => {
    const list = listRef.value
    if (!list) return
    const item = list.children[highlightedIndex.value] as HTMLElement | undefined
    if (item) {
      item.scrollIntoView({ block: 'nearest' })
    }
  })
}

function onClickOutside(e: MouseEvent): void {
  if (wrapperRef.value && !wrapperRef.value.contains(e.target as Node)) {
    close()
  }
}

watch(isOpen, (open) => {
  if (open) {
    const selectedIdx = filtered.value.indexOf(props.modelValue)
    if (selectedIdx >= 0) {
      highlightedIndex.value = selectedIdx
      nextTick(() => scrollToHighlighted())
    }
  }
})

onMounted(() => {
  document.addEventListener('mousedown', onClickOutside)
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onClickOutside)
})

const displayValue = computed((): string => {
  if (isOpen.value) return searchQuery.value
  return props.modelValue
})
</script>

<template>
  <div
    ref="wrapperRef"
    class="tz-select"
  >
    <input
      ref="inputRef"
      :value="displayValue"
      type="text"
      class="form-input"
      :placeholder="placeholder ?? 'Search timezone...'"
      autocomplete="off"
      @focus="onInputFocus"
      @input="
        (e) => {
          searchQuery = (e.target as HTMLInputElement).value
          onInput()
        }
      "
      @keydown="onKeydown"
    />
    <div
      v-if="isOpen"
      ref="listRef"
      class="tz-dropdown"
    >
      <div
        v-if="filtered.length === 0"
        class="tz-no-results"
      >
        No timezones found
      </div>
      <div
        v-for="(tz, idx) in filtered"
        :key="tz"
        class="tz-option"
        :class="{ highlighted: idx === highlightedIndex, selected: tz === modelValue }"
        @mousedown.prevent="select(tz)"
        @mouseenter="highlightedIndex = idx"
      >
        {{ tz }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.tz-select {
  position: relative;
  width: 100%;
  max-width: 300px;
}

.tz-dropdown {
  position: absolute;
  top: 100%;
  left: 0;
  right: 0;
  z-index: 50;
  max-height: 240px;
  overflow-y: auto;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  margin-top: 4px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
}

.tz-option {
  padding: 0.5rem 0.75rem;
  font-size: 0.875rem;
  color: var(--text-primary);
  cursor: pointer;
}

.tz-option.highlighted {
  background: var(--bg-hover);
}

.tz-option.selected {
  font-weight: 600;
  color: var(--primary);
}

.tz-no-results {
  padding: 0.75rem;
  font-size: 0.875rem;
  color: var(--text-muted);
  text-align: center;
}
</style>

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { X } from '@lucide/vue'

interface Props {
  open: boolean
  title?: string
  size?: 'sm' | 'md' | 'lg'
}

const props = withDefaults(defineProps<Props>(), {
  title: undefined,
  size: 'md',
})

const emit = defineEmits<{
  close: []
}>()

const dialogRef = ref<HTMLElement | null>(null)
const previousActiveElement = ref<Element | null>(null)

function trapFocus(e: KeyboardEvent): void {
  if (!dialogRef.value) return
  const focusable = dialogRef.value.querySelectorAll<HTMLElement>(
    'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
  )
  if (focusable.length === 0) return

  const first = focusable[0]
  const last = focusable[focusable.length - 1]

  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault()
    last.focus()
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault()
    first.focus()
  }
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    emit('close')
  } else if (e.key === 'Tab') {
    trapFocus(e)
  }
}

watch(
  () => props.open,
  async (isOpen) => {
    if (isOpen) {
      previousActiveElement.value = document.activeElement
      document.documentElement.style.overflow = 'hidden'
      await nextTick()
      dialogRef.value?.focus()
      document.addEventListener('keydown', onKeydown)
    } else {
      document.documentElement.style.overflow = ''
      document.removeEventListener('keydown', onKeydown)
      if (previousActiveElement.value instanceof HTMLElement) {
        previousActiveElement.value.focus()
      }
    }
  },
)

onMounted(() => {
  if (props.open) {
    document.addEventListener('keydown', onKeydown)
  }
})

onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown)
  document.documentElement.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div
        v-if="open"
        class="modal-backdrop"
        @mousedown.self="emit('close')"
      >
        <div
          ref="dialogRef"
          role="dialog"
          aria-modal="true"
          :aria-labelledby="title ? 'modal-title' : undefined"
          class="modal-dialog"
          :class="`modal-${size}`"
          tabindex="-1"
        >
          <div
            v-if="title || $slots.header"
            class="modal-header"
          >
            <slot name="header">
              <h2
                id="modal-title"
                class="modal-title"
              >
                {{ title }}
              </h2>
            </slot>
            <button
              class="modal-close"
              aria-label="Close"
              @click="emit('close')"
            >
              <X :size="18" />
            </button>
          </div>
          <div class="modal-body">
            <slot />
          </div>
          <div
            v-if="$slots.footer"
            class="modal-footer"
          >
            <slot name="footer" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-backdrop {
  position: fixed;
  inset: 0;
  background: var(--overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 300;
  backdrop-filter: blur(2px);
  padding: 1rem;
}

.modal-dialog {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  max-width: 95vw;
  max-height: 85vh;
  overflow-y: auto;
  box-shadow: var(--shadow-lg);
  outline: none;
}

.modal-sm {
  width: 380px;
}

.modal-md {
  width: 460px;
}

.modal-lg {
  width: 640px;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1.25rem 1.5rem 0;
}

.modal-title {
  font-size: 1.1rem;
  font-weight: 700;
}

.modal-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition:
    color 0.15s,
    background 0.15s;
}

.modal-close:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.modal-body {
  padding: 1.25rem 1.5rem;
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 0.75rem;
  padding: 0 1.5rem 1.25rem;
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.2s ease;
}

.modal-enter-active .modal-dialog,
.modal-leave-active .modal-dialog {
  transition: transform 0.2s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-from .modal-dialog {
  transform: scale(0.95) translateY(0.5rem);
}

.modal-leave-to .modal-dialog {
  transform: scale(0.95) translateY(0.5rem);
}
</style>

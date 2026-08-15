<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick, useId } from 'vue'
import { X } from '@lucide/vue'

interface Props {
  open: boolean
  title?: string
  size?: 'sm' | 'md' | 'lg'
  /**
   * Wrap body and footer in a `<form>` so that a submit button in the footer
   * submits the fields in the body. Emits `submit` with the native event
   * already prevented.
   */
  form?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  title: undefined,
  size: 'md',
  form: false,
})

const emit = defineEmits<{
  close: []
  submit: []
}>()

const dialogRef = ref<HTMLElement | null>(null)
const previousActiveElement = ref<Element | null>(null)
// Unique per instance: a view may declare several modals, and a duplicated
// id would point aria-labelledby at another dialog's heading.
const titleId = useId()

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

function activate(): void {
  previousActiveElement.value = document.activeElement
  document.documentElement.style.overflow = 'hidden'
  // Bind synchronously. Deferring this until after the nextTick below would let
  // a close that lands in between remove a listener that is not attached yet,
  // and the listener would then outlive the dialog.
  document.addEventListener('keydown', onKeydown)
  void nextTick().then(() => {
    if (props.open) dialogRef.value?.focus()
  })
}

function deactivate(): void {
  document.documentElement.style.overflow = ''
  document.removeEventListener('keydown', onKeydown)
  if (previousActiveElement.value instanceof HTMLElement) {
    previousActiveElement.value.focus()
  }
}

function onSubmit(e: Event): void {
  e.preventDefault()
  emit('submit')
}

watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) {
      activate()
    } else {
      deactivate()
    }
  },
)

onMounted(() => {
  if (props.open) activate()
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
          :aria-labelledby="title || $slots.header ? titleId : undefined"
          class="modal-dialog"
          :class="`modal-${size}`"
          tabindex="-1"
        >
          <div
            v-if="title || $slots.header"
            class="modal-header"
          >
            <!-- `titleId` is handed to the slot so a custom header can still
                 name the dialog for assistive tech. -->
            <slot
              name="header"
              :title-id="titleId"
            >
              <h2
                :id="titleId"
                class="modal-title"
              >
                {{ title }}
              </h2>
            </slot>
            <button
              type="button"
              class="modal-close"
              aria-label="Close"
              @click="emit('close')"
            >
              <X :size="18" />
            </button>
          </div>

          <form
            v-if="form"
            class="modal-content"
            @submit="onSubmit"
          >
            <div class="modal-body">
              <slot />
            </div>
            <div
              v-if="$slots.footer"
              class="modal-footer"
            >
              <slot name="footer" />
            </div>
          </form>
          <div
            v-else
            class="modal-content"
          >
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
  width: 100%;
  max-height: 85vh;
  display: flex;
  flex-direction: column;
  box-shadow: var(--shadow-lg);
  outline: none;
}

.modal-sm {
  max-width: 380px;
}

.modal-md {
  max-width: 460px;
}

.modal-lg {
  max-width: 640px;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 1.25rem 1.5rem 0;
  flex: none;
}

.modal-title {
  font-size: var(--fs-lg);
  font-weight: 700;
}

.modal-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  flex: none;
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition:
    color var(--duration-base),
    background var(--duration-base);
}

.modal-close:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

/* The scroll container is the body, not the dialog, so the header and the
   footer stay put while a long form scrolls between them. */
.modal-content {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.modal-body {
  padding: 1.25rem 1.5rem;
  overflow-y: auto;
  min-height: 0;
}

.modal-footer {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 0.75rem;
  padding: 0 1.5rem 1.25rem;
  flex: none;
}

/* A validation error in the footer explains the buttons next to it, so it
   takes a row of its own rather than squeezing in beside them. */
.modal-footer :deep(.form-error) {
  flex-basis: 100%;
  margin-top: 0;
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity var(--duration-slow) ease;
}

.modal-enter-active .modal-dialog,
.modal-leave-active .modal-dialog {
  transition: transform var(--duration-slow) ease;
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

<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { useToast } from '../composables/useToast'
import { CheckCircle, XCircle, AlertTriangle, Info } from '@lucide/vue'
import type { Component } from 'vue'
import type { ToastType } from '../composables/useToast'

const { toasts, remove } = useToast()

const iconMap: Record<ToastType, Component> = {
  success: CheckCircle,
  error: XCircle,
  warning: AlertTriangle,
  info: Info,
}
</script>

<template>
  <Teleport to="body">
    <div
      class="toast-container"
      aria-live="polite"
      aria-atomic="false"
    >
      <TransitionGroup name="toast">
        <div
          v-for="toast in toasts"
          :key="toast.id"
          class="toast"
          :class="`toast-${toast.type}`"
          role="alert"
        >
          <component
            :is="iconMap[toast.type]"
            class="toast-icon"
            :size="16"
            aria-hidden="true"
          />
          <span class="toast-message">{{ toast.message }}</span>
          <button
            class="toast-dismiss"
            aria-label="Dismiss"
            @click="remove(toast.id)"
          >
            &times;
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-container {
  position: fixed;
  top: 1rem;
  right: 1rem;
  z-index: 400;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  max-width: 380px;
  width: calc(100vw - 2rem);
  pointer-events: none;
}

.toast {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-lg);
  font-size: 0.85rem;
  pointer-events: auto;
}

.toast-success {
  border-left: 3px solid var(--success);
}

.toast-error {
  border-left: 3px solid var(--danger);
}

.toast-warning {
  border-left: 3px solid var(--warning);
}

.toast-info {
  border-left: 3px solid var(--info);
}

.toast-icon {
  flex-shrink: 0;
}

.toast-success .toast-icon {
  color: var(--success);
}

.toast-error .toast-icon {
  color: var(--danger);
}

.toast-warning .toast-icon {
  color: var(--warning);
}

.toast-info .toast-icon {
  color: var(--info);
}

.toast-message {
  flex: 1;
  color: var(--text-primary);
  line-height: 1.4;
}

.toast-dismiss {
  flex-shrink: 0;
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 1.2rem;
  line-height: 1;
  padding: 0.125rem 0.25rem;
  border-radius: var(--radius-sm);
  transition:
    color 0.15s,
    background 0.15s;
}

.toast-dismiss:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.toast-enter-active {
  transition:
    transform 0.3s ease,
    opacity 0.3s ease;
}

.toast-leave-active {
  transition:
    transform 0.2s ease,
    opacity 0.2s ease;
}

.toast-enter-from {
  transform: translateX(1rem);
  opacity: 0;
}

.toast-leave-to {
  transform: translateX(1rem);
  opacity: 0;
}

.toast-move {
  transition: transform 0.3s ease;
}
</style>
